@tool
extends EditorPlugin

var _core: GodotVimCore

func _enter_tree():
	_core = GodotVimCore.new()
	add_child(_core)

func _exit_tree():
	_core = null
