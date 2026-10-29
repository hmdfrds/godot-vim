extends SceneTree

# Runs compare.gd on fixed inputs before any case:
#   godot --headless --path <project> --script res://addons/suite_probe/self_test.gd
# Prints SELFTEST: PASS, or one SELFTEST: FAIL line per broken check.

const C := preload("compare.gd")
const NAMES := {"<CR>": 1, "<Esc>": 1, "<Tab>": 1, "<lt>": 1, "<Space>": 1}

var _fails: PackedStringArray = []


func _init() -> void:
	_round_trip()
	_tokens()
	_modes()
	_stated()
	_vanilla()
	_digests()
	_staleness()
	_vacuity()
	_defaults()
	if _fails.is_empty():
		print("SELFTEST: PASS")
		quit(0)
	else:
		for f in _fails:
			print("SELFTEST: FAIL ", f)
		quit(1)


func _check(ok: bool, what: String) -> void:
	if not ok:
		_fails.append(what)


func _round_trip() -> void:
	var observed := [0, 6]
	var stated = JSON.parse_string("[0, 6]")
	_check(C.same(observed, stated), "an int caret equals the same caret read from JSON")
	_check(not C.same([0, 6], [0, 7]), "different carets differ")
	_check(C.same(null, null) and not C.same(false, null), "false is not null")
	_check(C.same({"b": 1, "a": [true]}, {"a": [true], "b": 1.0}), "key order and number type do not matter")


func _tokens() -> void:
	_check(C.tokens("x<Esc>u", NAMES) == PackedStringArray(["x", "<Esc>", "u"]), "x<Esc>u is three keys")
	_check(C.tokens(["pri", "<wait>", "<CR>"], NAMES) == PackedStringArray(["p", "r", "i", "<wait>", "<CR>"]), "array form")
	_check(C.tokens("<C-v>jj<S-Tab>", NAMES) == PackedStringArray(["<C-v>", "j", "j", "<S-Tab>"]), "modifier chords")
	_check(C.tokens("a<b>c", NAMES) == PackedStringArray(["a", "<", "b", ">", "c"]), "<b> is typed literally")
	_check(C.tokens("<confirm><now>x<snap:typed><wait:600><set:off><reopen>", NAMES) == PackedStringArray(["<confirm>", "<now>", "x", "<snap:typed>", "<wait:600>", "<set:off>", "<reopen>"]), "harness steps")
	_check(C.tokens("<E-Down>", {"<Down>": 1}) == PackedStringArray(["<E-Down>"]), "key repeat")
	_check(C.types_plain_key(C.tokens("<Esc>u", NAMES)), "u types a plain key")
	_check(not C.types_plain_key(C.tokens("<Esc><wait>", NAMES)), "no plain key")
	_check(C.frames(120) == 8 and C.frames(600) == 36 and C.frames(400) == 24 and C.frames(100) == 6, "ms to frames rounds up")


func _modes() -> void:
	var ok := {
		"NORMAL": "NORMAL", "INSERT": "INSERT", "V-BLOCK": "V-BLOCK", "OP-PENDING": "OP-PENDING",
		"recording @q INSERT": "INSERT", "NORMAL (3 cursors)": "NORMAL", "NORMAL  d2": "NORMAL",
		"recording @a VISUAL (2 cursors)  y": "VISUAL",
	}
	for bar in ok:
		_check(C.mode_of(bar) == ok[bar], "mode of %s" % bar)
	for bar in ["3 lines yanked", "E492: Not an editor command: x", ":set tw?", "-- INSERT --", "NORMAL extra", ""]:
		_check(C.mode_of(bar) == null, "no mode in %s" % JSON.stringify(bar))
	_check(C.mode_of(null) == null, "no bar, no mode")


func _stated() -> void:
	var c := C.with_defaults({"id": "t", "keys": "x"})
	var obs := {"end": {"text": ["\tx"], "caret": [0, 1], "mode": null, "bar": "1 line yanked"}}
	var r := C.compare(c, obs, {"end": {"text": ["\tx"], "caret": [0, 1]}}, null)
	_check(r["reasons"].is_empty(), "stated text and caret pass")
	r = C.compare(c, obs, {"end": {"caret": [0, 2]}}, null)
	_check(r["reasons"].size() == 1 and str(r["reasons"][0]).begins_with("end caret: want"), "a wrong caret fails")
	r = C.compare(c, obs, {"end": {"mode": "NORMAL"}}, null)
	_check(r["reasons"].size() == 1 and str(r["reasons"][0]).contains("no mode is shown"), "a message never passes for a mode")
	r = C.compare(c, obs, {"undo": {"text": []}}, null)
	_check(r["reasons"].size() == 1 and str(r["reasons"][0]) == "undo: no such snapshot", "a missing snapshot fails")
	r = C.compare(c, {"end": {"focus": ""}}, {"end": {"closed": false}}, null)
	_check(r["reasons"].is_empty(), "closed defaults to false")
	var by := C.expected_for({"expect": {"end": {"text": ["a"], "caret": [0, 0]}}, "by_godot": {"4.5-stable": {"end": {"text": ["b"]}}}}, "4.5-stable")
	_check(C.same(by, {"end": {"text": ["b"], "caret": [0, 0]}}), "by_godot overrides one field")


func _vanilla() -> void:
	var rec := {"start": {"text": ["\t"], "caret": [0, 1], "carets": 1, "popup": false, "focus": ""},
		"end": {"text": ["\tprint()"], "caret": [0, 7], "carets": 1, "popup": false, "focus": ""}}
	var c := C.with_defaults({"id": "t", "keys": "pri", "oracle": "vanilla"})
	var obs: Dictionary = rec.duplicate(true)
	obs["end"]["mode"] = "INSERT"
	_check(C.compare(c, obs, {}, rec)["reasons"].is_empty(), "plugin equal to vanilla passes; mode is not compared")
	obs["end"]["popup"] = "print"
	var r := C.compare(c, obs, {}, rec)
	_check(r["reasons"].size() == 1 and str(r["reasons"][0]).begins_with("end popup: vanilla"), "a popup vanilla lacks fails")
	var cv := C.with_defaults({"id": "t", "keys": "pri<Esc>", "vkeys": "pri", "oracle": "vanilla"})
	_check(C.compare(cv, obs, {}, rec)["reasons"].is_empty(), "with vkeys the end snapshot is not compared")
	var cn := C.with_defaults({"id": "t", "keys": "pri", "oracle": "vanilla", "compare": ["text"]})
	_check(C.compare(cn, obs, {}, rec)["reasons"].is_empty(), "compare limits the fields")
	_check(C.compare(c, obs, {"end": {"popup": "print"}}, rec)["reasons"].is_empty(), "a stated field wins over vanilla")


func _digests() -> void:
	var a := {"end": {"text": ["\tx"], "caret": [0, 1]}}
	var b := {"end": {"caret": [0.0, 1.0], "text": ["\tx"]}}
	_check(C.digest(a) == C.digest(b), "digest ignores key order and number type")
	_check(C.digest(a) != C.digest({"end": {"text": ["\ty"], "caret": [0, 1]}}), "digest follows the values")
	_check(C.digest(a).length() == 8, "digest is 8 hex digits")


func _staleness() -> void:
	var c := {"id": "t", "desc": "one", "keys": "pri", "oracle": "vanilla"}
	var h := C.case_hash(c)
	var d := c.duplicate()
	d["desc"] = "two"
	d["expect"] = {"end": {"mode": "INSERT"}}
	_check(C.case_hash(d) == h, "desc and expect do not make a recording stale")
	d = c.duplicate()
	d["keys"] = "prin"
	_check(C.case_hash(d) != h, "keys make it stale")
	d = c.duplicate()
	d["gap_ms"] = 120
	_check(C.case_hash(d) == h, "a default written out is the same case")
	d["gap_ms"] = 20
	_check(C.case_hash(d) != h, "a changed default makes it stale")
	d = c.duplicate()
	d["vkeys"] = "pr"
	_check(C.case_hash(d) != h, "vkeys replace keys")
	_check(not C.same(C.header("4.5-stable", "aa"), C.header("4.5-stable", "ab")), "the template hash is in the header")


func _vacuity() -> void:
	var toks := C.tokens("ix", NAMES)
	var same := {"start": {"text": ["\t"]}, "end": {"text": ["\t"]}}
	_check(C.vacuous({}, toks, same), "typing that changes nothing is vacuous")
	_check(not C.vacuous({"unchanged": true}, toks, same), "unchanged cases are not")
	_check(not C.vacuous({}, C.tokens("<Esc>", NAMES), same), "no plain key, not vacuous")
	_check(not C.vacuous({}, toks, {"start": {"text": ["\t"]}, "end": {"text": ["\tx"]}}), "a change is not vacuous")


func _defaults() -> void:
	var d := C.with_defaults({"id": "t", "keys": "x", "line": "\tab\n\tcd", "row": 1})
	_check(int(d["col"]) == 3 and d["file"] == "gd" and int(d["gap_ms"]) == 120, "line form defaults")
	_check(C.start_text(d) == C.HEAD + "\tab\n\tcd" + C.TAIL, "line form start text")
	_check(C.check_case({"id": "t", "desc": "d", "keys": "x"}) == "", "a minimal case is accepted")
	_check(C.check_case({"id": "t", "keys": "x"}) != "", "a case without desc is refused")
	_check(C.check_case({"id": "t", "desc": "d", "keys": "x", "colour": 1}) != "", "an unknown field is refused")
	_check(C.check_case({"id": "t", "keys": "x", "expect": {"end": {"txt": []}}}) != "", "an unknown snapshot field is refused")
	_check(C.check_case({"id": "t", "desc": "d", "keys": "x", "expect": {"end": {"top": 3}}}) == "", "top is a snapshot field")
	_check(C.check_case({"id": "t", "keys": "x", "text": "a"}) != "", "text without caret is refused")
	_check(C.version_at_least({"major": 4, "minor": 6, "patch": 1}, "4.6"), "4.6.1 is at least 4.6")
	_check(not C.version_at_least({"major": 4, "minor": 5, "patch": 2}, "4.6"), "4.5.2 is below 4.6")
