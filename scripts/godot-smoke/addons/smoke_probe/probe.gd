@tool
extends EditorPlugin

# One step per frame from _process, not a coroutine: the editor reloads every
# script after the first scan loads an extension, and that cancels coroutines.
# Settles before quitting: quitting soon after that scan crashes Godot
# (godotengine/godot#111048).

enum Phase { SCAN, ATTACH, INSERT, NORMAL, SETTLE, DONE }

const TIMEOUT_MS := 60000
# Also lets deferred plugin errors reach the log.
const SETTLE_MS := 3000

var _phase := Phase.SCAN
var _release: InputEventKey
var _settle_until := 0


func _process(_delta: float) -> void:
	if _phase == Phase.DONE:
		return
	if _release:
		_viewport().push_input(_release)
		_release = null
		return
	if Time.get_ticks_msec() > TIMEOUT_MS:
		_finish("timed out in phase %s" % Phase.keys()[_phase])
		return
	var label := _mode_label()
	match _phase:
		Phase.SCAN:
			if EditorInterface.get_resource_filesystem().is_scanning():
				return
			if not ClassDB.class_exists("GodotVimCore"):
				_finish("GodotVimCore is not registered, so the extension did not load")
				return
			EditorInterface.set_main_screen_editor("Script")
			EditorInterface.edit_script(load("res://main.gd"))
			_phase = Phase.ATTACH
		Phase.ATTACH:
			if label == null:
				return
			print("SMOKE: attached on Godot ", Engine.get_version_info()["string"])
			_code_edit().grab_focus()
			_press(KEY_I)
			_phase = Phase.INSERT
		Phase.INSERT:
			if label.text == "INSERT":
				_press(KEY_ESCAPE)
				_phase = Phase.NORMAL
		Phase.NORMAL:
			if label.text == "NORMAL":
				_settle_until = Time.get_ticks_msec() + SETTLE_MS
				_phase = Phase.SETTLE
		Phase.SETTLE:
			if Time.get_ticks_msec() >= _settle_until:
				_finish("")


func _finish(err: String) -> void:
	_phase = Phase.DONE
	if err.is_empty():
		print("SMOKE: PASS")
		get_tree().quit(0)
	else:
		print("SMOKE: FAIL: ", err)
		get_tree().quit(1)


func _code_edit() -> CodeEdit:
	var se := EditorInterface.get_script_editor().get_current_editor()
	return se.get_base_editor() as CodeEdit if se else null


func _viewport() -> Viewport:
	return EditorInterface.get_base_control().get_viewport()


func _mode_label() -> Label:
	var ce := _code_edit()
	if ce == null:
		return null
	for bar in ce.find_children("*", "VimStatusBar", true, false):
		for label in bar.find_children("*", "Label", true, false):
			return label
	return null


# The release goes out on the next frame, as from a keyboard.
func _press(keycode: Key) -> void:
	var ev := InputEventKey.new()
	ev.keycode = keycode
	if keycode == KEY_I:
		ev.unicode = "i".unicode_at(0)
	ev.pressed = true
	_viewport().push_input(ev)
	_release = ev.duplicate() as InputEventKey
	_release.pressed = false
