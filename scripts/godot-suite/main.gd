extends Node

# Longer than the CodeEdit's gutter count: Godot checks remove_gutter's
# index against the line count and logs an error on detach otherwise (#21).

@export var speed := 4.0

var _elapsed := 0.0


func _ready() -> void:
	set_process(true)


func _process(delta: float) -> void:
	_elapsed += delta * speed
