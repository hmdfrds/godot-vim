extends SceneTree
## Runs steps.txt on a CodeEdit and writes one reading per step, or times
## bursts of undo(). See scripts/godot-undo-trace.sh.

var ce: CodeEdit
var base_version := 0


func _initialize() -> void:
	var args := OS.get_cmdline_user_args()
	ce = CodeEdit.new()
	ce.size = Vector2(800, 600)
	ce.line_folding = true
	root.add_child(ce)
	var code := 2
	if args.size() == 4 and args[0] == "--steps" and args[2] == "--out":
		code = run_steps(args[1], args[3])
	elif args.size() == 2 and args[0] == "--burst":
		code = burst(int(args[1]))
	else:
		printerr("usage: --steps FILE --out FILE | --burst N")
	quit(code)


func unescape(s: String) -> String:
	return s.replace("\\\\", "\u0001").replace("\\n", "\n").replace("\\t", "\t").replace("\u0001", "\\")


func escape(s: String) -> String:
	return s.replace("\\", "\\\\").replace("\n", "\\n").replace("\t", "\\t")


## Carets read "*" after an edit: see steps.txt.
func reading(op: String) -> String:
	var carets := PackedStringArray()
	if op == "insert" or op == "remove":
		carets.append("*")
	else:
		for i in ce.get_caret_count():
			var c := "%d:%d" % [ce.get_caret_line(i), ce.get_caret_column(i)]
			if ce.has_selection(i):
				c += "/%d:%d" % [ce.get_selection_origin_line(i), ce.get_selection_origin_column(i)]
			carets.append(c)
	var folded := PackedStringArray()
	for l in ce.get_folded_lines():
		folded.append(str(l))
	return "v=%d undo=%d redo=%d carets=%s folded=%s text=%s" % [
		ce.get_version() - base_version,
		int(ce.has_undo()),
		int(ce.has_redo()),
		",".join(carets),
		"-" if folded.is_empty() else ",".join(folded),
		escape(ce.text),
	]


func run_steps(steps_path: String, out_path: String) -> int:
	var steps := FileAccess.open(steps_path, FileAccess.READ)
	var out := FileAccess.open(out_path, FileAccess.WRITE)
	if steps == null or out == null:
		printerr("cannot open %s or %s" % [steps_path, out_path])
		return 2
	while not steps.eof_reached():
		var line := steps.get_line()
		if line.is_empty() or line.begins_with("#"):
			continue
		var op := line.get_slice(" ", 0)
		var rest := line.substr(op.length() + 1)
		var n := rest.split(" ")
		match op:
			"text":
				ce.text = unescape(rest)
				ce.clear_undo_history()
				base_version = ce.get_version()
			"fold_region":
				continue
			"insert":
				ce.insert_text(unescape(rest.substr(n[0].length() + n[1].length() + 2)), int(n[0]), int(n[1]))
			"remove":
				ce.remove_text(int(n[0]), int(n[1]), int(n[2]), int(n[3]))
			"begin":
				ce.begin_complex_operation()
			"end":
				ce.end_complex_operation()
			"typing":
				ce.start_action(TextEdit.ACTION_TYPING)
			"close":
				ce.start_action(TextEdit.ACTION_NONE)
			"undo":
				ce.undo()
			"redo":
				ce.redo()
			"caret":
				ce.deselect()
				ce.remove_secondary_carets()
				ce.set_caret_line(int(n[0]))
				ce.set_caret_column(int(n[1]))
			"select":
				ce.select(int(n[0]), int(n[1]), int(n[2]), int(n[3]), int(n[4]))
			"add_caret":
				ce.add_caret(int(n[0]), int(n[1]))
			"fold":
				ce.fold_line(int(n[0]))
			_:
				printerr("unknown step: " + line)
				return 2
		out.store_line("%s | %s" % [line, reading(op)])
	return 0


## n edits on a 2,000-line script, each one op in its own complex operation
## as godot-vim commits `x`, then n undo() calls, timed.
func burst(n: int) -> int:
	var lines := PackedStringArray()
	for i in 2000:
		lines.append("func f%d():" % i if i % 20 == 0 else "\tvar v%d = %d" % [i, i])
	ce.text = "\n".join(lines)
	ce.clear_undo_history()
	var original := ce.text
	var t0 := Time.get_ticks_usec()
	for i in n:
		var l := (i * 7) % 2000
		ce.begin_complex_operation()
		ce.remove_text(l, 1, l, 2)
		ce.end_complex_operation()
	var t1 := Time.get_ticks_usec()
	var slowest := 0
	for i in n:
		var a := Time.get_ticks_usec()
		ce.undo()
		slowest = maxi(slowest, Time.get_ticks_usec() - a)
	var t2 := Time.get_ticks_usec()
	var restored := ce.text == original and not ce.has_undo()
	print("BURST n=%d edits_us=%d undo_us=%d undo_mean_us=%.1f undo_max_us=%d restored=%s" % [
		n, t1 - t0, t2 - t1, float(t2 - t1) / maxi(n, 1), slowest, restored,
	])
	return 0 if restored else 1
