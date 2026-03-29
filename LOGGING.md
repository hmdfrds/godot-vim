# GodotVim Logging Guidelines

Every log statement must pass this test: **"If a developer reads this in a bug
report, can they understand what happened without asking follow-up questions?"**

---

## Quick Reference

```
error  =  invariant violation, should-never-happen bug
warn   =  recoverable degradation, fallback used
info   =  lifecycle milestone (once per session or user action)
debug  =  per-operation (one story per discrete user action)
trace  =  per-keystroke, per-frame, per-effect
```

**Formatting rules:**
- Use `{}` (Display) for Key, KeyEvent, Mode — never `{:?}` on types with Display impls.
- Use `key=value` pairs for structured data, past tense for completed actions.
- Every `error!`/`warn!` must answer: what happened, what input, what the code did about it.
- Never log file contents, clipboard contents, or user text above trace.
- Never log at debug or higher in per-frame callbacks.

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

- **key** — vim notation (`k`, `<C-w>`, `<Esc>`, `ci(`)
- **mode** — mode at time of keypress (`Normal`, `Insert`, `Visual`, `V-Line`)
- **cmd** — what the engine interpreted (`Down`, `Change(inner-Paren)`, `Pending`, `InsertExit`)
- **cursor** — only shown when cursor moved, as `line:col→line:col`
- **text_mutated** — shown when text was changed
- **mode→X** — shown when mode changed
- **effects** — number of effects dispatched
- **latency** — processing time in microseconds

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

## Log Levels

The `log` crate level maps to Godot output via `logging.rs`:

| Level   | Godot output         | Compile-time gate                      |
|---------|----------------------|----------------------------------------|
| `error` | `godot_error!`       | Always available                       |
| `warn`  | `godot_warn!`        | Always available                       |
| `info`  | `godot_print!`       | Always available                       |
| `debug` | `godot_print! [DBG]` | Always available                       |
| `trace` | `godot_print! [TRC]` | Always available                       |

A "session" means one Godot editor process lifetime.

### error — Invariant violations and bugs

The code reached a state that should be impossible. The reader should treat
every `error!` as a potential bug report.

Qualifies: routing violations (effect in wrong dispatch pass), resource
corruption (undo depth overflow, caret creation failure), pipeline overflow
(drain loop, host request recursion), init failures, file I/O data loss.

Does NOT qualify: expected user errors (wrong command, file not found —
those surface via the status bar), recoverable fallbacks (use `warn`),
missing features (use `info` once per session).

### warn — Recoverable issues and degraded operation

The code recovered gracefully but something is wrong. Warnings should be
infrequent during normal operation.

Qualifies: setting type mismatch with fallback, orphaned undo groups,
security sandbox actions, missing infrastructure, unhandled engine variants
(version mismatch), non-zero shell exit codes, frame budget exceeded, UI
degradation (font/color lookup failures).

Does NOT qualify: per-frame/per-keystroke fallbacks (use `trace`),
successful operations (use `info` or `debug`).

### info — Lifecycle events and session-level milestones

At most once per session or once per user action. A user with `Info` should
see a clean chronological narrative without repetition.

Qualifies: plugin init/shutdown, config sourced, file saved/reloaded,
security policy applied, feature not supported (once per session).

Does NOT qualify: per-keystroke events (use `debug`/`trace`), internal
detail (use `debug`).

### debug — Per-operation events

One message per discrete user action. A developer troubleshooting "why
didn't `:w` work?" finds the answer at `debug` without drowning in noise.

Qualifies: the per-keystroke summary line (target `"key"`), editor
attach/detach, host request dispatch, indent/commentstring sync, undo/redo,
mode transitions, window navigation, completion interception, passthrough
keys, floating window detection, custom ex-commands.

Does NOT qualify: per-character insert/delete (use `trace`), cursor position
updates (use `trace`), effect-by-effect dispatch (use `trace`).

### trace — Per-keystroke, per-frame, and hot-path detail

High-frequency events useful only when narrowing down a specific keystroke
or frame.

Qualifies: every keystroke entering the pipeline, every effect dispatched,
cursor/scroll updates, input parsing, auto-brace decisions, search highlight
updates, clipboard lengths, context build details.

Does NOT qualify: loop body iterations (too granular even for trace — log
the summary instead).

---

## Message Format

### Display over Debug

Use `{}` (Display) for types that have Display impls: `KeyEvent`, `Key`,
`Mode`, `Modifiers`. Use `{}` for strings (not `{:?}` which adds quotes).
Use `.to_i64()` for `InstanceId` to show plain numbers.

```rust
// Good
log::debug!("operation: key={} mode={} editor=#{}", key, mode, id.to_i64());

// Bad — raw Debug dumps
log::debug!("operation: key={:?} mode={:?} editor={:?}", key, mode, id);
```

### Structured key=value pairs

Use `key=value` pairs for machine-parseable context. Separate logical groups
with `|` when there are many fields.

```rust
// Good
log::debug!("operation: path={} force={}", path, force);
log::warn!("Budget exceeded: {} > {} | ctx={} eng={} fx={}", total, budget, ctx, eng, fx);

// Bad — prose style
log::debug!("Editing file at path {} with force mode on", path);
```

### Tense and brevity

- **Past tense** for completed actions: "Attached to editor", "Config sourced".
- **Present participle** only for in-progress with a later confirmation log.
- Keep messages under 120 characters (excluding format arguments).
- Do NOT log both entry and exit for simple operations — one log at
  completion is sufficient.

### Prefixes

Use `function_name:` as a prefix only when the module target alone is
ambiguous. Do NOT repeat the module path — `shorten_target` in `logging.rs`
already includes the last two `::` segments.

---

## Prohibited Logging

### NEVER log at debug or higher in per-frame code paths

`_process()`, `_physics_process()`, `on_editor_draw`, scrollbar handlers,
cursor position recalculation, search highlight recomputation — all must use
`trace` or not log at all.

### NEVER log sensitive data

- **File contents**: log length, never content.
- **Clipboard contents**: log length only. Never preview.
- **User text**: never at `info` or higher. `trace` may include single
  characters for auto-brace debugging.
- **Passwords, tokens, environment variables**: never, at any level.

### NEVER log redundant information

- Do not repeat what the log target already provides (module path).
- Do not repeat what the caller already logged.
- Do not log "entering function" if the function also logs its result.
- Do not log what didn't change (e.g., `search=<none>` on every keystroke).

---

## Anti-Patterns

### Printf debugging left in production

```rust
// Bad
log::debug!("here");
log::debug!("value = {:?}", x);

// Good — use trace with context
log::trace!("operation: key={} result={}", key, result);
```

Temporary debug prints must be removed before merge.

### Missing context

```rust
// Bad — what failed? why? what now?
log::error!("failed");

// Good — answers what, input, consequence
log::error!(
    "Operation exceeded limit ({}): input={} — falling back to default",
    MAX_LIMIT, input
);
```

### Logging in a loop body at debug or higher

```rust
// Bad — fires once per iteration
for item in items.iter() {
    log::debug!("processing: {}", item);
}

// Good — summary after the loop
log::trace!("processed {} items, {} matches found", items.len(), matches);
```

### Logging success of routine operations

```rust
// Bad — noise on every successful read
log::info!("Successfully read setting '{}'", key);

// Good — the happy path is silent; only log failures
log::warn!("Setting '{}' has unexpected type, using default", key);
```

### Logging without discriminating context

```rust
// Bad — which editor? which buffer?
log::debug!("Attached to editor");

// Good — includes the ID so the log is actionable
log::debug!("Attached to editor #{}", id.to_i64());
```

---

## Level Decision Flowchart

```
Is it a bug / invariant violation?
  YES → error!

Is it recoverable but wrong (fallback used, something missing)?
  YES → warn!

Is it a lifecycle milestone (once per session or user action)?
  YES → info!

Is it tied to a discrete user action (command, attach, host request)?
  YES → debug!

Is it per-keystroke, per-frame, or per-effect?
  YES → trace!

None of the above?
  → Probably don't log it.
```

---

## Checklist for New Log Statements

Before adding a `log::*!` call, verify:

1. **Level** is correct per the flowchart above.
2. **Context** is included: what happened, to what, with what input.
3. **key=value pairs** are used for structured data (not prose).
4. **No sensitive data** (file contents, clipboard, user text).
5. **Not redundant** with the caller or the log target.
6. **Not in a per-frame path** at debug or higher.
7. **Past tense** for completed actions.
8. **Under 120 characters** (excluding format arguments).
9. **Consistent** with surrounding code in the same module.
10. **Uses `{}` (Display)** for Key, Mode, strings — not `{:?}`.
