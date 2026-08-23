# Debugging GodotVim

How to read a GodotVim log. Aimed at anyone diagnosing a problem or filing a
bug report. You do not need to know the codebase.

If you are *writing* a log call rather than reading one, you want
[LOGGING.md](https://github.com/hmdfrds/godot-vim/blob/main/LOGGING.md) instead.

## Quick Reference

```
error  =  invariant violation, should-never-happen bug
warn   =  recoverable degradation, fallback used
info   =  lifecycle milestone (once per session or user action)
debug  =  per-operation (one story per discrete user action)
trace  =  per-keystroke, per-frame, per-effect
```

---

## Reading the Logs

### How to enable

In Godot: **Editor → Editor Settings → GodotVim → Log Level**. Set to `Debug`
for bug reports, `Trace` for engine development. Logs appear in Godot's
**Output** panel (bottom dock).

All log levels are available in both debug and release builds.

### The per-keystroke summary line

At `Debug` level, every keystroke produces exactly **one line** that tells
the complete story:

```
[DBG][key] k  Normal  cmd=Down  cursor=10:0→9:0  effects=2  259µs
[DBG][key] c  Normal  cmd=Pending  effects=0  117µs
[DBG][key] i  Normal  cmd=Pending  effects=0  104µs
[DBG][key] (  Normal  cmd=Change(inner-Paren)  cursor=9:0→9:14  text_mutated  mode→Insert  effects=13  581µs
[DBG][key] <Esc>  Insert  cmd=InsertExit  cursor=9:14→9:13  mode→Normal  effects=9  279µs
```

Reading it: `key  mode  cmd=command  cursor=before→after  [flags]  effects=N  latency`.

- **key**: vim notation (`k`, `<C-w>`, `<Esc>`, `ci(`)
- **mode**: mode at time of keypress (`Normal`, `Insert`, `Visual`, `V-Line`)
- **cmd**: what the engine interpreted (`Down`, `Change(inner-Paren)`, `Pending`, `InsertExit`)
- **cursor**: only shown when cursor moved, as `line:col→line:col`
- **text_mutated**: shown when text was changed
- **mode→X**: shown when mode changed
- **effects**: number of effects dispatched
- **latency**: processing time in microseconds

The `[key]` log target lets you filter keystroke summaries specifically:
`grep '\[key\]' output.log`

### Common grep patterns for debugging

```bash
# All errors and warnings (first thing to check in any bug report)
grep -E '\[ERR\]|\[WRN\]' output.log

# Keystroke-by-keystroke narrative
grep '\[key\]' output.log

# Mode transitions only
grep 'mode→' output.log

# Text mutations only
grep 'text_mutated' output.log

# A specific key sequence (e.g., what happened when user pressed ci()
grep '\[key\]' output.log | grep -E '^\[DBG\]\[key\] [ci(]'

# Editor lifecycle (attach/detach)
grep -E 'Attached|Detached' output.log

# Host requests (file I/O, shell commands)
grep 'host_request\|file::' output.log
```

### Trace level: pipeline internals

At `Trace`, you see the per-keystroke pipeline between summary lines:

```
[TRC][bridge::input] parse_godot_key: k
[TRC][controller::process] process_single_key: key=k operations_this_cycle=1
[TRC][bridge::context] build_context: cursor=10:0 (offset=259)  viewport=[lines 0..17, width=131]
[TRC][effects::dispatch] dispatch: 2 effects
[TRC][effects::cursor] set_cursor: offset=217 -> line=9 col=0
[TRC][effects::dispatch] [internal] Event(CursorMoved)
[DBG][key] k  Normal  cmd=Down  cursor=10:0→9:0  effects=2  259µs
```

Trace is for engine developers narrowing down a specific keystroke.

### Limitations

- **Logs are ephemeral.** Godot's Output panel has no persistence or rotation.
  Copy the output before closing the editor.
- **`cargo test` does not capture logs.** The `logging.rs` guard
  (`!godot::sys::is_initialized()`) silently discards logs outside Godot.
  Log-dependent behavior cannot be tested in unit tests.

---

## Filing a bug report

Set **Log Level** to `Debug`, reproduce the problem, and paste the Output
panel into the issue. If the log is long, the `grep` patterns above will
usually narrow it faster than trimming by hand: the errors-and-warnings
pattern is the right first thing to run.

Include the Godot version, your OS, and your keyboard layout. Layout matters
more than it sounds: several past defects reproduced only on Colemak, Dvorak
or AZERTY, because a key's *position* and the character it produces differ
there. If a keybinding misbehaves, `:panelmap {key}` prints exactly how that
key resolved and is worth pasting too; for completion keys it prints a
counterfactual section resolving the key as if the popup were open.
