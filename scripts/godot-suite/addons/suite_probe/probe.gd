@tool
extends EditorPlugin

# Runs one case of scripts/godot-suite/cases.jsonl in this editor, then
# quits. Inert unless scripts/godot-suite.sh sets SUITE_MODE. Every line it
# prints starts with SUITE, so none can match the log gate.

const C := preload("compare.gd")
const MAIN := "res://main.gd"
const SHADER := "res://t.gdshader"
const WAIT_MS := 30000
const WATCHDOG_MS := 90000
const SET_FRAMES := 90

var _mode := ""
var _variant := ""
var _label := ""
var _case: Dictionary = {}
var _id := ""
var _keys := {}
var _toks := PackedStringArray()
var ce: CodeEdit
var _vp: Viewport
var _shader: Shader
var _snaps := {}
var _started := 0
var _done := false
var _realtime := false


func _enter_tree() -> void:
	_mode = OS.get_environment("SUITE_MODE")
	if _mode.is_empty():
		return
	# 4.8-dev starts headless editors at 0x0; 4.5 to 4.7 start at this size.
	get_tree().root.size = Vector2i(C.ROOT[0], C.ROOT[1])
	_started = Time.get_ticks_msec()
	_id = OS.get_environment("SUITE_CASE")
	_variant = OS.get_environment("SUITE_VARIANT")
	_label = OS.get_environment("SUITE_GODOT")
	_realtime = OS.get_environment("SUITE_REALTIME") == "1"
	set_process(true)
	_main.call_deferred()


func _exit_tree() -> void:
	if not _mode.is_empty() and not _done and _mode != "warmup":
		_done = true
		print("SUITE: FAIL %s %s %s" % [_id, _hash8("quit"), JSON.stringify(["the editor quit mid-case"])])


func _process(_delta: float) -> void:
	if _mode.is_empty() or _done:
		return
	if Time.get_ticks_msec() - _started > WATCHDOG_MS:
		_fatal("watchdog: no result after %d s" % (WATCHDOG_MS / 1000))


func _main() -> void:
	var err := _guard()
	if err.is_empty():
		err = _check_version()
	if not err.is_empty():
		_fatal(err)
		return
	if _mode == "warmup":
		await _warmup()
		return
	err = _load_case()
	if not err.is_empty():
		_fatal(err)
		return
	print("SUITE-HEADER ", JSON.stringify(C.header(_label, OS.get_environment("SUITE_TEMPLATE_HASH"))))
	var min_godot: String = _case.get("min_godot", "")
	if not min_godot.is_empty() and not C.version_at_least(Engine.get_version_info(), min_godot):
		_finish("SUITE: SKIP %s needs Godot %s" % [_id, min_godot])
		return
	err = await _setup()
	if not err.is_empty():
		_fatal("setup: " + err)
		return
	_snaps["start"] = _snap()
	err = await _run_keys()
	if not err.is_empty():
		_fatal(err)
		return
	await _wait_ms(C.FINAL_SETTLE_MS)
	_snaps["end"] = _snap()
	_verdict()


# HOME and the XDG dirs are checked by the runner too; this catches an editor
# that found its paths some other way.
func _guard() -> String:
	var root := OS.get_environment("SUITE_ROOT")
	if root.is_empty():
		return "SUITE_ROOT is not set"
	var ep := EditorInterface.get_editor_paths()
	for p in [OS.get_user_data_dir(), ep.get_config_dir(), ep.get_data_dir(), ep.get_cache_dir()]:
		if not str(p).begins_with(root + "/"):
			return "editor path outside the run dir: %s" % p
	return ""


func _check_version() -> String:
	var v: String = Engine.get_version_info()["string"]
	var running := v.get_slice(" (", 0)
	if running != _label:
		return "the runner says Godot %s, the editor is %s" % [_label, running]
	return ""


func _warmup() -> void:
	var fs := EditorInterface.get_resource_filesystem()
	var deadline := Time.get_ticks_msec() + WAIT_MS
	while fs.is_scanning():
		if Time.get_ticks_msec() > deadline:
			_fatal("warm-up: filesystem scan did not finish")
			return
		await _frames(1)
	var cache := EditorInterface.get_editor_paths().get_cache_dir()
	deadline = Time.get_ticks_msec() + 60000
	while _doc_cache(cache).is_empty():
		if Time.get_ticks_msec() > deadline:
			_fatal("warm-up: no documentation cache in " + cache)
			return
		await _frames(1)
	# The cache file is written in one go, but give it a moment to close.
	await _wait_ms(500)
	var names := PackedStringArray()
	for p in EditorInterface.get_editor_settings().get_property_list():
		names.append(p["name"])
	var f := FileAccess.open(OS.get_environment("SUITE_OUT"), FileAccess.WRITE)
	if f == null:
		_fatal("warm-up: cannot write " + OS.get_environment("SUITE_OUT"))
		return
	f.store_string("\n".join(names) + "\n")
	f.close()
	_finish("SUITE-WARM %s" % _doc_cache(cache))


func _doc_cache(dir: String) -> String:
	for f in DirAccess.get_files_at(dir):
		if f.begins_with("editor_doc_cache") and f.ends_with(".res"):
			return f
	return ""


func _load_case() -> String:
	var f := FileAccess.open(OS.get_environment("SUITE_CASES"), FileAccess.READ)
	if f == null:
		return "cannot read " + OS.get_environment("SUITE_CASES")
	while not f.eof_reached():
		var line := f.get_line()
		if not line.begins_with("{\"id\": \"%s\"" % _id):
			continue
		var c = JSON.parse_string(line)
		if not c is Dictionary or c.get("id") != _id:
			continue
		var err := C.check_case(c)
		if not err.is_empty():
			return "case %s: %s" % [_id, err]
		_case = C.with_defaults(c)
		break
	if _case.is_empty():
		return "no case %s in %s" % [_id, OS.get_environment("SUITE_CASES")]
	var e := _load_keys()
	if not e.is_empty():
		return e
	var keys: Variant = _case["keys"]
	if _variant == "vanilla" and _case.has("vkeys"):
		keys = _case["vkeys"]
	_toks = C.tokens(keys, _keys)
	for t in _toks:
		if C.is_harness(t):
			if t.begins_with("<set:") and not _case.get("sets", {}).has(t.substr(5, t.length() - 6)):
				return "%s names no entry of sets" % t
			continue
		if _key_row(t).is_empty():
			return "no row in keys.tsv for %s" % t
	return ""


func _load_keys() -> String:
	var f := FileAccess.open(OS.get_environment("SUITE_KEYS"), FileAccess.READ)
	if f == null:
		return "cannot read keys.tsv"
	while not f.eof_reached():
		var line := f.get_line()
		if line.is_empty() or line.begins_with("#"):
			continue
		var c := line.split("\t")
		if c.size() != 5:
			return "keys.tsv: not five columns: " + line
		_keys[c[0]] = [int(c[1]), int(c[2]), int(c[3]), c[4]]
	return ""


const ALIASES := {" ": "<Space>", "\t": "<Tab>", "<": "<lt>"}


func _key_row(tok: String) -> Array:
	if tok.length() == 1:
		return _keys.get(ALIASES.get(tok, tok), [])
	var body := tok.substr(1, tok.length() - 2)
	while body.length() > 2 and body[1] == "-" and "CSAME".contains(body[0]):
		body = body.substr(2)
	if body.length() == 1:
		return _keys.get(body.to_lower() if body >= "A" and body <= "Z" else body, [])
	return _keys.get("<" + body + ">", [])


func _setting_names() -> Dictionary:
	var names := {}
	for n in FileAccess.get_file_as_string(OS.get_environment("SUITE_SETTING_NAMES")).split("\n"):
		if not n.is_empty():
			names[n] = true
	return names


func _check_settings(d: Dictionary, godot_only: bool) -> String:
	var names := _setting_names()
	for k in d:
		if godot_only and str(k).begins_with("plugins/GodotVim/"):
			return "%s is a godot-vim setting: set it at startup through fixtures/<name>/config/settings.txt, or at run time through sets and <set:NAME>" % k
		if not names.has(k):
			return "no editor setting %s" % k
	return ""


func _setup() -> String:
	var fs := EditorInterface.get_resource_filesystem()
	var deadline := Time.get_ticks_msec() + WAIT_MS
	await _frames(2)
	while fs.is_scanning():
		if Time.get_ticks_msec() > deadline:
			return "filesystem scan did not finish"
		await _frames(1)
	var before: Dictionary = _case.get("settings_before", {})
	var e := _check_settings(before, true)
	if e.is_empty():
		e = _check_settings(_case.get("settings", {}), true)
	if e.is_empty():
		for n in _case.get("sets", {}):
			e = _check_settings(_case["sets"][n], false)
			if not e.is_empty():
				break
	if not e.is_empty():
		return e
	_apply_shortcuts(_case.get("shortcuts", {}))
	await _apply_settings(before)
	EditorInterface.open_scene_from_path("res://main.tscn")
	await _frames(10)
	# The case text goes into the resource before its tab exists, so Godot's
	# own load fills the tab (set_text, clear_undo_history, tag_saved_version)
	# before godot-vim can attach, as when a user opens a file.
	var start := C.start_text(_case)
	var script := load(MAIN) as Script
	if _case["file"] == "shader":
		EditorInterface.set_main_screen_editor("Script")
		EditorInterface.edit_script(script)
		await _frames(10)
		_shader = load(SHADER) as Shader
		_shader.code = start
		EditorInterface.edit_resource(_shader)
		await _frames(10)
		ce = _find_ce(EditorInterface.get_base_control(), start)
	else:
		script.source_code = start
		EditorInterface.set_main_screen_editor("Script")
		EditorInterface.edit_script(script)
		await _frames(10)
		var se := EditorInterface.get_script_editor().get_current_editor()
		ce = se.get_base_editor() as CodeEdit if se else null
	if ce == null:
		return "no CodeEdit holds the case text"
	if ce.text != start:
		return "the tab did not load the case text"
	if ce.has_undo():
		return "the loaded text can be undone"
	_vp = ce.get_viewport()
	ce.grab_focus()
	await _frames(5)
	if not ce.has_focus():
		return "the CodeEdit did not take focus"
	if _variant == "plugin":
		deadline = Time.get_ticks_msec() + WAIT_MS
		while C.mode_of(_bar()) == null:
			if Time.get_ticks_msec() > deadline:
				return "godot-vim did not attach (bar: %s)" % JSON.stringify(_bar())
			await _frames(1)
	await _apply_settings(_case.get("settings", {}))
	await _place_carets()
	ce.grab_focus()
	await _frames(3)
	return ""


func _place_carets() -> void:
	var row: int
	var col: int
	var line_text: String
	if _case.has("text"):
		row = int(_case["caret"][0])
		col = int(_case["caret"][1])
	else:
		row = C.EDIT_LINE + int(_case["row"])
		col = int(_case["col"])
	line_text = ce.get_line(row)
	ce.set_caret_line(row)
	if _variant == "plugin" and _case["start"] == "insert":
		# Normal mode cannot rest past the last character, so append there.
		var at_end := col >= line_text.length() and col > 0
		ce.set_caret_column(col - 1 if at_end else col)
		await _frames(3)
		await _press("a" if at_end else "i", true)
	else:
		ce.set_caret_column(col)
		await _frames(3)
	var base := 0 if _case.has("text") else C.EDIT_LINE
	for p in _case.get("carets", []):
		ce.add_caret(base + int(p[0]), int(p[1]))
	await _frames(2)


func _run_keys() -> String:
	var gap := C.frames(float(_case["gap_ms"]))
	var i := 0
	while i < _toks.size():
		var t := _toks[i]
		var now_next := i + 1 < _toks.size() and _toks[i + 1] == "<now>"
		if t == "<now>":
			pass
		elif t.begins_with("<snap:"):
			_snaps[t.substr(6, t.length() - 7)] = _snap()
		elif t == "<wait>":
			await _wait_ms(float(_case["settle_ms"]))
		elif t.begins_with("<wait:"):
			await _wait_ms(float(t.substr(6, t.length() - 7)))
		elif t == "<confirm>":
			if not is_instance_valid(ce):
				return "<confirm> after the CodeEdit was freed"
			# As a click on a popup row does.
			ce.confirm_code_completion()
			if not now_next:
				await _frames(2)
				await _wait_frames(gap)
		elif t.begins_with("<set:"):
			await _apply_set(t.substr(5, t.length() - 6))
		elif t == "<reopen>":
			var e := await _reopen()
			if not e.is_empty():
				return e
		else:
			await _press(t, not now_next)
		i += 1
	return ""


# Opens main.gd again, as a double click in the FileSystem dock does; the
# snapshots read that tab from then on.
func _reopen() -> String:
	if _case["file"] != "gd":
		return "<reopen> works in gd cases only"
	EditorInterface.edit_script(load(MAIN))
	await _frames(10)
	var se := EditorInterface.get_script_editor().get_current_editor()
	ce = se.get_base_editor() as CodeEdit if se else null
	if ce == null:
		return "<reopen>: no CodeEdit holds main.gd"
	ce.grab_focus()
	await _frames(5)
	if _variant == "plugin":
		var deadline := Time.get_ticks_msec() + WAIT_MS
		while _bar() == null:
			if Time.get_ticks_msec() > deadline:
				return "<reopen>: godot-vim did not attach"
			await _frames(1)
	await _frames(3)
	return ""


func _verdict() -> void:
	var obs := JSON.stringify({"id": _id, "snaps": _snaps})
	if _mode == "record":
		if C.vacuous(_case, _toks, _snaps):
			print("SUITE-OBS ", obs)
			_finish("SUITE: FAIL %s %s %s" % [_id, _hash8("vacuous"), JSON.stringify(["vacuous recording: end equals start although the keys type text; mark the case unchanged if that is the point"])])
			return
		print("SUITE-REC ", JSON.stringify({"id": _id, "input": C.case_hash(_case), "snaps": _snaps}))
		_finish("SUITE: REC %s" % _id)
		return
	var expected := C.expected_for(_case, _label)
	var vanilla: Variant = null
	var stale := ""
	if _case.get("oracle", "") == "vanilla":
		var got := _recording()
		stale = got[0]
		vanilla = got[1]
	var res := C.compare(_case, _snaps, expected, vanilla)
	var reasons: Array = res["reasons"]
	if not stale.is_empty():
		reasons.push_front(stale)
	print("SUITE-OBS ", obs)
	if expected.is_empty() and vanilla == null and stale.is_empty():
		reasons.append("nothing to compare: state an expect or use oracle vanilla")
	if reasons.is_empty():
		_finish("SUITE: PASS %s" % _id)
	else:
		var compared: Dictionary = res["compared"]
		if not stale.is_empty():
			compared["stale"] = true
		_finish("SUITE: FAIL %s %s %s" % [_id, C.digest(compared), JSON.stringify(reasons)])


# [stale reason or "", snapshots or null]
func _recording() -> Array:
	var path := OS.get_environment("SUITE_VANILLA")
	var none := "no recording, or a stale one: run --record (%s)"
	if path.is_empty() or not FileAccess.file_exists(path):
		return [none % "no file for %s" % _label, null]
	var f := FileAccess.open(path, FileAccess.READ)
	var head = JSON.parse_string(f.get_line())
	var mine := C.header(_label, OS.get_environment("SUITE_TEMPLATE_HASH"))
	if not head is Dictionary or not C.same(head, mine):
		return [none % "the header differs from this probe's", null]
	while not f.eof_reached():
		var line := f.get_line()
		if not line.begins_with("{\"id\":\"%s\"" % _id):
			continue
		var r = JSON.parse_string(line)
		if not r is Dictionary or r.get("id") != _id:
			continue
		if r.get("input") != C.case_hash(_case):
			return [none % "the case changed since it was recorded", null]
		return ["", r["snaps"]]
	return [none % "no line for this case", null]


func _snap() -> Dictionary:
	var s := {}
	if _case.get("disk", false):
		var path := SHADER if _case["file"] == "shader" else MAIN
		s["disk"] = Array(FileAccess.get_file_as_string(path).split("\n"))
	if not is_instance_valid(ce):
		s["closed"] = true
		s["focus"] = _focus()
		return s
	var lines: Array = []
	var row_base := 0
	var last := ce.get_line_count() - 1
	if not _case.has("text"):
		row_base = C.EDIT_LINE
		last -= C.TAIL_LINES
	for l in range(row_base, last + 1):
		lines.append(ce.get_line(l))
	s["text"] = lines
	s["caret"] = [ce.get_caret_line() - row_base, ce.get_caret_column()]
	s["top"] = ce.get_first_visible_line() - row_base
	s["carets"] = ce.get_caret_count()
	var sel := ce.get_code_completion_selected_index()
	s["popup"] = false
	if sel >= 0 and sel < ce.get_code_completion_options().size():
		s["popup"] = ce.get_code_completion_option(sel).get("display_text", "")
	s["focus"] = _focus()
	var bar: Variant = _bar()
	s["bar"] = bar
	s["mode"] = C.mode_of(bar)
	return s


# "" while the case's CodeEdit has focus, else the focus owner's class, plus
# each visible dialog's title. Node names are generated and move between
# versions, so they are left out.
func _focus() -> String:
	var fo := _vp.gui_get_focus_owner() if _vp else null
	var s := ""
	if not (is_instance_valid(ce) and fo == ce):
		s = fo.get_class() if fo else "none"
	for w in get_tree().root.find_children("*", "AcceptDialog", true, false):
		if (w as Window).visible:
			s += " [dialog '%s']" % (w as Window).title
	return s.strip_edges()


func _bar() -> Variant:
	for bar in ce.find_children("*", "VimStatusBar", true, false):
		for label in bar.find_children("*", "Label", true, false):
			return (label as Label).text
	return null


func _find_ce(n: Node, text: String) -> CodeEdit:
	for c in n.get_children(true):
		if c is CodeEdit and (c as CodeEdit).text == text:
			return c
		var r := _find_ce(c, text)
		if r:
			return r
	return null


func _apply_shortcuts(d: Dictionary) -> void:
	# What Editor Settings > Shortcuts does.
	for action in d:
		var evs: Array[InputEvent] = []
		for spec in d[action]:
			evs.append(_shortcut_event(spec))
		EditorInterface.get_editor_settings().set_builtin_action_override(action, evs)


func _shortcut_event(spec: String) -> InputEventKey:
	var ev := _event("<" + spec + ">" if spec.length() > 1 else spec)
	ev.unicode = 0
	ev.physical_keycode = KEY_NONE
	ev.key_label = KEY_NONE
	return ev


func _apply_settings(d: Dictionary) -> void:
	if d.is_empty():
		return
	var es := EditorInterface.get_editor_settings()
	for k in d:
		es.set_setting(k, d[k])
	_notify_settings()
	await _frames(10)


# The Settings dialog saves 1.5 s after the last change and then notifies
# every node (editor_settings_dialog.cpp, notify_changes).
func _apply_set(name: String) -> void:
	var es := EditorInterface.get_editor_settings()
	var d: Dictionary = _case["sets"][name]
	for k in d:
		es.set_setting(k, d[k])
	await _frames(SET_FRAMES)
	_notify_settings()
	await _frames(2)


func _notify_settings() -> void:
	get_tree().root.get_child(0).propagate_notification(EditorSettings.NOTIFICATION_EDITOR_SETTINGS_CHANGED)


func _event(tok: String) -> InputEventKey:
	var row := _key_row(tok)
	var mods := ""
	if tok.length() > 1:
		var body := tok.substr(1, tok.length() - 2)
		while body.length() > 2 and body[1] == "-" and "CSAME".contains(body[0]):
			mods += body[0]
			body = body.substr(2)
	var ev := InputEventKey.new()
	ev.keycode = row[0]
	ev.physical_keycode = row[1]
	ev.key_label = row[0]
	ev.unicode = row[2]
	ev.shift_pressed = str(row[3]).contains("S") or mods.contains("S")
	ev.ctrl_pressed = mods.contains("C")
	ev.alt_pressed = mods.contains("A")
	ev.meta_pressed = mods.contains("M")
	ev.echo = mods.contains("E")
	# With an input method active, X11 sends Ctrl and Alt chords without the
	# character; without one it sends the character.
	if (ev.ctrl_pressed or ev.alt_pressed) and not _case.get("chord_unicode", false):
		ev.unicode = 0
	return ev


func _modifier_events(ev: InputEventKey) -> Array:
	var out := []
	for m in [[ev.ctrl_pressed, KEY_CTRL], [ev.alt_pressed, KEY_ALT], [ev.meta_pressed, KEY_META], [ev.shift_pressed, KEY_SHIFT]]:
		if m[0]:
			var e := InputEventKey.new()
			e.keycode = m[1]
			e.physical_keycode = m[1]
			e.key_label = m[1]
			e.ctrl_pressed = ev.ctrl_pressed
			e.alt_pressed = ev.alt_pressed
			e.meta_pressed = ev.meta_pressed
			e.shift_pressed = ev.shift_pressed
			out.append(e)
	return out


func _send(ev: InputEvent) -> void:
	if _case.get("delivery", "push") == "input":
		Input.parse_input_event(ev)
	else:
		_vp.push_input(ev)


# Press, one frame, release, one frame and the gap, as from a keyboard.
# Without the trailing wait the next step happens in the release's frame.
func _press(tok: String, trailing: bool) -> void:
	var ev := _event(tok)
	var mods: Array = _modifier_events(ev) if _case.get("modifier_events", false) else []
	for m in mods:
		var down := (m as InputEventKey).duplicate() as InputEventKey
		down.pressed = true
		_send(down)
		await _frames(1)
	ev.pressed = true
	_send(ev)
	await _frames(1)
	var up := ev.duplicate() as InputEventKey
	up.pressed = false
	up.echo = false
	_send(up)
	if not mods.is_empty():
		mods.reverse()
		for m in mods:
			await _frames(1)
			var rel := (m as InputEventKey).duplicate() as InputEventKey
			rel.pressed = false
			_send(rel)
	if trailing:
		await _frames(1)
		await _wait_frames(C.frames(float(_case["gap_ms"])))


func _teardown() -> void:
	# Nothing may be saved at quit: the case tab gets the file's own text back
	# and every other edited tab is undone to its loaded text, each tagged
	# saved, so Godot finds no unsaved script to write.
	var main_text := FileAccess.get_file_as_string(MAIN)
	var shader_text := FileAccess.get_file_as_string(SHADER)
	if is_instance_valid(ce):
		ce.text = shader_text if _case.get("file", "gd") == "shader" else main_text
		ce.clear_undo_history()
		ce.tag_saved_version()
	var script := load(MAIN) as Script
	if script:
		script.source_code = main_text
	if _shader:
		_shader.code = shader_text
	for se in EditorInterface.get_script_editor().get_open_script_editors():
		var te := se.get_base_editor() as TextEdit
		if te == null or te == ce or te.get_version() == te.get_saved_version():
			continue
		while te.has_undo():
			te.undo()
		te.tag_saved_version()


func _fatal(msg: String) -> void:
	if _done:
		return
	if _mode == "warmup":
		_finish("SUITE: FAIL warm-up %s %s" % [_hash8(msg), JSON.stringify([msg])])
	else:
		if not _snaps.is_empty():
			print("SUITE-OBS ", JSON.stringify({"id": _id, "snaps": _snaps}))
		_finish("SUITE: FAIL %s %s %s" % [_id, _hash8(msg), JSON.stringify([msg])])


func _finish(line: String) -> void:
	if _done:
		return
	_done = true
	print(line)
	if _mode != "warmup" and not _case.is_empty():
		_teardown()
	get_tree().quit(0)


func _hash8(s: String) -> String:
	return s.sha256_text().substr(0, 8)


func _frames(n: int) -> void:
	for i in n:
		await get_tree().process_frame


func _wait_frames(n: int) -> void:
	if _realtime:
		await _wait_ms(n * 1000.0 / C.FPS)
	else:
		await _frames(n)


# Under --fixed-fps every wait is a whole number of frames; --realtime waits
# on a timer instead, as a diagnostic.
func _wait_ms(ms: float) -> void:
	if ms <= 0.0:
		await _frames(1)
	elif _realtime:
		await get_tree().create_timer(ms / 1000.0).timeout
	else:
		await _frames(C.frames(ms))
