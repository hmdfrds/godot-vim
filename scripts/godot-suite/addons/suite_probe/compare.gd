@tool
extends RefCounted

# Case defaults, key notation, snapshot comparison, digests and recording
# headers. Pure functions, shared by probe.gd and self_test.gd.

const FORMAT := 1
const FPS := 60
const ROOT := [1024, 600]
const FINAL_SETTLE_MS := 400
const EDIT_LINE := 4
const HEAD := "extends Node\n\n\nfunc _ready() -> void:\n"
const TAIL := "\n\tpass"
const TAIL_LINES := 1

const DEFAULTS := {
	"file": "gd", "start": "normal", "gap_ms": 120, "settle_ms": 600,
	"delivery": "push", "modifier_events": false, "chord_unicode": false,
	"unchanged": false, "disk": false,
	"compare": ["text", "caret", "popup", "focus"],
}

const CASE_FIELDS := [
	"id", "desc", "issue", "note", "file", "line", "row", "col", "text", "caret",
	"start", "keys", "vkeys", "gap_ms", "settle_ms", "settings_before", "settings",
	"shortcuts", "sets", "carets", "fixture", "delivery", "modifier_events",
	"chord_unicode", "unchanged", "disk", "min_godot", "oracle", "compare",
	"expect", "by_godot",
]

const SNAP_FIELDS := ["text", "caret", "carets", "popup", "focus", "mode", "bar", "closed", "disk"]

# Fields that change what vanilla Godot does; a recording is stale when any of
# them changes.
const INPUT_FIELDS := [
	"file", "line", "row", "col", "text", "caret", "keys", "gap_ms", "settle_ms",
	"settings_before", "settings", "shortcuts", "sets", "carets", "fixture",
	"delivery", "modifier_events", "chord_unicode", "disk",
]

# The status bar's mode forms (src/ui/status_bar.rs). Anything else, such as
# a message, reads as no mode.
const MODE_RE := "^(?:recording @. )?(NORMAL|INSERT|VISUAL|V-LINE|V-BLOCK|SELECT|S-LINE|S-BLOCK|REPLACE|V-REPLACE|COMMAND|OP-PENDING)(?: \\(\\d+ cursors\\))?(?:  .+)?$"

const HARNESS_RE := "^(?:now|confirm|reopen|wait(?::\\d+)?|(?:snap|set):[a-z0-9_-]+)$"


# JSON numbers parse as floats and Array == is type-strict, so every value is
# compared in its JSON form.
static func rt(x: Variant) -> Variant:
	return JSON.parse_string(JSON.stringify(x))


static func same(a: Variant, b: Variant) -> bool:
	return JSON.stringify(rt(a)) == JSON.stringify(rt(b))


static func frames(ms: float) -> int:
	return ceili(ms * FPS / 1000.0)


static func check_case(c: Dictionary) -> String:
	for k in c:
		if not k in CASE_FIELDS:
			return "unknown field '%s'" % k
	if not c.has("id") or not c.has("desc") or not c.has("keys"):
		return "a case needs id, desc and keys"
	for k in ["expect"]:
		for snap in c.get(k, {}):
			for f in c[k][snap]:
				if not f in SNAP_FIELDS:
					return "unknown snapshot field '%s' in %s.%s" % [f, k, snap]
	for label in c.get("by_godot", {}):
		for snap in c["by_godot"][label]:
			for f in c["by_godot"][label][snap]:
				if not f in SNAP_FIELDS:
					return "unknown snapshot field '%s' in by_godot" % f
	for f in c.get("compare", []):
		if not f in SNAP_FIELDS:
			return "unknown compare field '%s'" % f
	if c.has("oracle") and c["oracle"] != "vanilla":
		return "oracle must be \"vanilla\""
	if c.has("text") != c.has("caret"):
		return "text and caret go together"
	if c.has("text") and (c.has("line") or c.has("row") or c.has("col")):
		return "text form and line form do not mix"
	return ""


static func with_defaults(c: Dictionary) -> Dictionary:
	var d := c.duplicate(true)
	for k in DEFAULTS:
		if not d.has(k):
			d[k] = rt(DEFAULTS[k])
	if not d.has("text"):
		if not d.has("line"):
			d["line"] = "\t"
		if not d.has("row"):
			d["row"] = 0
		if not d.has("col"):
			var lines: PackedStringArray = str(d["line"]).split("\n")
			d["col"] = lines[clampi(int(d["row"]), 0, lines.size() - 1)].length()
	return rt(d)


static func case_hash(c: Dictionary) -> String:
	var d := with_defaults(c)
	var keep := {}
	for k in INPUT_FIELDS:
		if d.has(k):
			keep[k] = d[k]
	if d.has("vkeys"):
		keep["keys"] = d["vkeys"]
	return JSON.stringify(keep).sha256_text()


static func header(label: String, template_hash: String) -> Dictionary:
	return {"format": FORMAT, "fps": FPS, "godot": label, "root": ROOT,
		"settle": FINAL_SETTLE_MS, "template": template_hash}


static func start_text(d: Dictionary) -> String:
	if d.has("text"):
		return d["text"]
	return HEAD + str(d["line"]) + TAIL


# Vim notation, as history_tests.rs writes it: "x<Esc>u" is three keys.
# <mods-name> is one key when name is a key name, or one character after a
# modifier; anything else is typed literally. Harness steps are <now>,
# <confirm>, <reopen>, <wait>, <wait:N>, <snap:NAME> and <set:NAME>.
static func tokens(keys: Variant, names: Dictionary) -> PackedStringArray:
	var parts: Array = keys if keys is Array else [keys]
	var harness := RegEx.create_from_string(HARNESS_RE)
	var out := PackedStringArray()
	for p in parts:
		var s := str(p)
		var i := 0
		while i < s.length():
			if s[i] == "<":
				var j := s.find(">", i + 1)
				if j > i + 1:
					var body := s.substr(i + 1, j - i - 1)
					if harness.search(body) or is_key_body(body, names):
						out.append("<" + body + ">")
						i = j + 1
						continue
			out.append(s[i])
			i += 1
	return out


static func is_key_body(body: String, names: Dictionary) -> bool:
	var mods := false
	while body.length() > 2 and body[1] == "-" and "CSAME".contains(body[0]):
		body = body.substr(2)
		mods = true
	if names.has("<" + body + ">"):
		return true
	return mods and body.length() == 1


static func is_harness(tok: String) -> bool:
	return tok.length() > 2 and RegEx.create_from_string(HARNESS_RE).search(tok.substr(1, tok.length() - 2)) != null


# True when the keys type at least one printable character.
static func types_plain_key(toks: PackedStringArray) -> bool:
	for t in toks:
		if t.length() == 1:
			return true
	return false


static func mode_of(bar: Variant) -> Variant:
	if typeof(bar) != TYPE_STRING:
		return null
	var m := RegEx.create_from_string(MODE_RE).search(bar)
	return m.get_string(1) if m else null


static func expected_for(c: Dictionary, label: String) -> Dictionary:
	var e: Dictionary = rt(c.get("expect", {}))
	var over: Dictionary = rt(c.get("by_godot", {})).get(label, {})
	for snap in over:
		if not e.has(snap):
			e[snap] = {}
		for f in over[snap]:
			e[snap][f] = over[snap][f]
	return e


static func field(snap: Dictionary, f: String) -> Variant:
	if f == "closed":
		return snap.get("closed", false)
	return snap.get(f, null)


static func sorted_keys(d: Dictionary) -> Array:
	var k := d.keys()
	k.sort()
	return k


# Compares the observed snapshots with the stated values and, for a vanilla
# case, with the recording. Returns the failure reasons and the observed
# value of every compared field, which the digest is made of.
static func compare(c: Dictionary, observed: Dictionary, expected: Dictionary, vanilla: Variant) -> Dictionary:
	var obs: Dictionary = rt(observed)
	var reasons: Array = []
	var compared := {}
	for snap in sorted_keys(expected):
		if not obs.has(snap):
			reasons.append("%s: no such snapshot" % snap)
			_put(compared, snap, "missing", true)
			continue
		for f in sorted_keys(expected[snap]):
			var want: Variant = expected[snap][f]
			var got: Variant = field(obs[snap], f)
			_put(compared, snap, f, got)
			if not same(want, got):
				if f == "mode" and got == null:
					reasons.append("%s mode: want %s, but no mode is shown: bar is %s" % [snap, JSON.stringify(want), JSON.stringify(field(obs[snap], "bar"))])
				else:
					reasons.append("%s %s: want %s, got %s" % [snap, f, JSON.stringify(want), JSON.stringify(got)])
	if vanilla is Dictionary:
		var fields: Array = c.get("compare", DEFAULTS["compare"]).duplicate()
		if "caret" in fields and not "carets" in fields:
			fields.append("carets")
		for snap in sorted_keys(vanilla):
			# With vkeys the plugin types more than vanilla after the last named snapshot.
			if snap == "end" and c.has("vkeys"):
				continue
			if not obs.has(snap):
				reasons.append("%s: no such snapshot" % snap)
				_put(compared, snap, "missing", true)
				continue
			for f in fields:
				if expected.has(snap) and expected[snap].has(f):
					continue
				var want: Variant = field(vanilla[snap], f)
				var got: Variant = field(obs[snap], f)
				_put(compared, snap, f, got)
				if not same(want, got):
					reasons.append("%s %s: vanilla %s, got %s" % [snap, f, JSON.stringify(want), JSON.stringify(got)])
	return {"reasons": reasons, "compared": compared}


static func _put(d: Dictionary, snap: String, f: String, v: Variant) -> void:
	if not d.has(snap):
		d[snap] = {}
	d[snap][f] = v


static func digest(compared: Dictionary) -> String:
	return JSON.stringify(rt(compared)).sha256_text().substr(0, 8)


# A recording whose end equals its start, for keys that type something, is
# most likely a harness that never reached the editor.
static func vacuous(c: Dictionary, toks: PackedStringArray, snaps: Dictionary) -> bool:
	if c.get("unchanged", false) or not types_plain_key(toks):
		return false
	return snaps.has("start") and snaps.has("end") and same(snaps["start"], snaps["end"])


static func version_at_least(info: Dictionary, wanted: String) -> bool:
	var w := wanted.split(".")
	var have := [int(info.get("major", 0)), int(info.get("minor", 0)), int(info.get("patch", 0))]
	for i in 3:
		var n := int(w[i]) if i < w.size() else 0
		if have[i] != n:
			return have[i] > n
	return true
