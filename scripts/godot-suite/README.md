# Behaviour suite

Real Godot editors, headless, typing real key events into the script and
shader editors, with godot-vim loaded. Each case states what the buffer, the
caret, the status bar and the completion popup must look like afterwards, or
says that they must match what vanilla Godot did with the same keys. CI runs
it on every Godot version in its matrix; failures that are known are pinned
per version in `known-failures.txt`, and the job fails when that set changes
in either direction.

Nothing here ships: `scripts/assemble-addon.sh` copies named files only.

## Running it

```sh
cargo build --release --locked
./scripts/godot-suite.sh --godot ~/.cache/godot-bins/Godot_v4.7.2-stable_linux.x86_64
```

| Flag | What |
|---|---|
| `--godot BIN` | the Godot editor binary (or `GODOT=BIN`) |
| `--lib FILE` | the library to test (default `target/release/` or `$CARGO_TARGET_DIR/release/`) |
| `--addon DIR` | a ready `addons/godot_vim` folder instead, such as an unpacked release zip |
| `--only a,b` | these cases only; the pin check covers only them |
| `--cases FILE` | an ad hoc case file, for exploring; same format |
| `--jobs N` | editors at once (default 4, at most `nproc`); changes the time only |
| `--keep DIR` | keep projects, logs and snapshots in DIR (absent or empty); otherwise a temp dir is deleted on exit |
| `--record [--out DIR]` | run the `oracle: "vanilla"` cases in vanilla Godot and write `vanilla/<godot>.jsonl` (or into DIR) |
| `--bless` | rewrite this Godot's lines in `known-failures.txt` from the run |
| `--print ID` | run one case and print its snapshots as a ready `expect` block |
| `--notes` | the pins and their reasons per version, as Markdown for release notes |
| `--realtime` | drop `--fixed-fps 60`; a diagnostic, never for recording or CI |

Every local binary in turn:
`for g in ~/.cache/godot-bins/Godot_v*; do ./scripts/godot-suite.sh --godot "$g"; done`.
Scratch space is one project copy per running case plus the logs; set
`TMPDIR` or pass `--keep` to keep it off a small `/tmp`.

Exit codes: 0 when the failing set is as pinned, 1 when it changed (or a
recording failed), 2 when the run could not happen (import, guard,
self-test, warm-up, a malformed case file).

## What a run does

1. Assembles the addon with `assemble-addon.sh`, builds a project from this
   directory (`project.godot`, `main.gd`, `main.tscn`, `t.gdshader`, the
   probe) and imports it once.
2. Runs `addons/suite_probe/self_test.gd`, which checks `compare.gd` on fixed
   inputs.
3. Starts one warm-up editor, which builds the documentation cache, writes
   the editor settings file every case starts from, and lists every editor
   setting's name.
4. Per case: a fresh copy of the imported project, fresh HOME and XDG dirs
   (checked to lie in the run dir, by the runner and again by the probe),
   the case's fixture, then `godot --headless --editor --fixed-fps 60` with
   its own LSP and DAP ports. The probe opens the case, types the keys,
   takes snapshots, compares, prints `SUITE: PASS|FAIL|SKIP <id>` and
   `SUITE-OBS <json>`, puts every tab back to its loaded text so nothing is
   saved, and quits. A watchdog ends a case after 90 s.
5. The log gate: every `ERROR:`, `WARNING:`, `SCRIPT ERROR:`, `SHADER ERROR:`,
   `UNKNOWN ERROR:` or `panicked at` line fails the case's `:log` channel,
   unless `allow-log.txt` allows it or vanilla printed the same line for the
   same case.
6. The pin check against `known-failures.txt` for the running version.

## Cases

`cases.jsonl` holds one JSON object per line, and every line starts with
`{"id": "`. Write `": "` and `", "` with one space, as the file does; the
runner reads `id`, `oracle` and `fixture` with grep.

| Field | Default | Meaning |
|---|---|---|
| `id` | required | kebab-case, stable; never renamed or reused. History cases are `issue<N>-...`, fast variants end in `-fast` |
| `desc` | required | one line |
| `issue` | | the GitHub issue the case guards |
| `file` | `"gd"` | `"gd"` edits `main.gd` in the script editor, `"shader"` edits `t.gdshader` |
| `line`, `row`, `col` | `"\t"`, 0, end of row | line form: `line` (may hold `\n`) goes inside `func _ready() -> void:` at line 4, followed by `\tpass`; snapshots count rows from line 4 and leave out the `\tpass` |
| `text`, `caret` | | text form: the whole buffer and an absolute `[row, col]` |
| `start` | `"normal"` | `"insert"`: godot-vim presses `a` (at the end of the line) or `i`, so typing starts in Insert at the same caret vanilla has |
| `keys` | required | Vim notation, as a string or a list of strings |
| `vkeys` | `keys` | the keys for vanilla, when they must differ; the `end` snapshot is then not compared with vanilla |
| `gap_ms`, `settle_ms` | 120, 600 | wait after each key; length of `<wait>` |
| `settings_before` | | Godot editor settings applied before the file opens; godot-vim keys are refused |
| `settings` | | Godot editor settings applied after setup, before the keys |
| `shortcuts` | | Shortcuts overrides, `{"action": ["S-Tab"]}` |
| `sets` | | named groups of settings for `<set:NAME>`; may hold godot-vim keys |
| `carets` | | extra carets `[[row, col]]`, as Alt+click adds them |
| `fixture` | | a directory under `fixtures/` installed before the editor starts |
| `delivery` | `"push"` | `"input"` sends keys through `Input.parse_input_event`, which also sets key state; the key takes effect a frame later |
| `modifier_events` | false | Shift, Ctrl, Alt and Meta go out as keys of their own around the chord, as X11 sends them |
| `chord_unicode` | false | Ctrl and Alt chords carry their character, as X11 sends them without an input method |
| `unchanged` | false | the keys are meant to change nothing (see the vacuity guard) |
| `disk` | false | snapshots also hold the case file's text on disk |
| `min_godot` | | below this version the case prints `SKIP` |
| `oracle` | | `"vanilla"`: compare with vanilla Godot's recording for this exact Godot |
| `compare` | `["text", "caret", "popup", "focus"]` | fields compared with vanilla; `carets` goes with `caret` |
| `expect` | | `{"snapshot": {"field": value}}`, stated values |
| `by_godot` | | `{"4.5-stable": {"snapshot": {"field": value}}}`, merged over `expect` on that version |
| `note` | | why the expectation is what it is; required for a stated difference from vanilla |

### Keys

A plain character types itself (US layout). `<CR>`, `<Esc>`, `<Tab>`,
`<BS>`, `<Del>`, `<Insert>`, `<Up>`, `<Down>`, `<Left>`, `<Right>`, `<Home>`,
`<End>`, `<PageUp>`, `<PageDown>`, `<Space>`, `<kEnter>`, `<lt>` and `<F1>`
to `<F12>` are one key each, with `C-`, `S-`, `A-` and `M-` prefixes:
`<C-v>`, `<S-Tab>`, `<C-Space>`. `keys.tsv` defines every key, and a cargo
test checks that each row decodes to what its notation names. Anything that
is not a key name is typed literally, as in Vim.

Harness steps, none of them a Vim key name:

| Step | Effect |
|---|---|
| `<wait>`, `<wait:N>` | `settle_ms`, or N ms, as `ceil(N * 60 / 1000)` frames |
| `<snap:NAME>` | take a snapshot |
| `<confirm>` | `CodeEdit.confirm_code_completion()`, as a click on a popup row |
| `<now>` | the next key goes out in the same frame as the step before it |
| `<set:NAME>` | apply `sets[NAME]`, wait 90 frames, then notify every node of the change, as the Settings dialog does; `plugins/GodotVim/enabled` off and on runs godot-vim's disable and enable |
| `<reopen>` | open `main.gd` again, as a double click in the FileSystem dock does, and wait for godot-vim to attach; later steps and snapshots use that tab (`gd` cases only) |
| `<E-key>` | the key as a key-repeat event |

A key costs two frames (press, release) plus the gap. Snapshots are `start`
(after setup), every `<snap:NAME>`, and `end` (400 ms after the last step).

Mouse steps are specified here for the work that needs them and are not
built yet: `<press:R,C>`, `<move:R,C>`, `<release:R,C>` and `<click:R,C>`,
with `S-` and `C-` prefixes, at the position `TextEdit.get_rect_at_line_column`
gives for row R, column C; `<now>` puts two of them in one frame.

### Snapshot fields

| Field | Content | Compared |
|---|---|---|
| `text` | the lines (line form: from line 4, without `\tpass`) | stated, and with vanilla by default |
| `caret`, `carets` | main caret `[row, col]`, caret count | stated, and with vanilla by default |
| `top` | the first visible line, counted as `caret` counts rows | stated only |
| `popup` | `false`, or the highlighted row's text | stated, and with vanilla by default |
| `focus` | `""` while the case's CodeEdit has focus, else the focus owner's class, plus `[dialog 'Title']` per visible dialog | stated, and with vanilla by default |
| `mode` | the mode word when the whole status bar is one of its mode forms, else `null` | stated only |
| `bar` | the status bar text, verbatim | stated only |
| `closed` | true once the case's CodeEdit is freed (`:q`, a tab close), until a `<reopen>` | stated only |
| `disk` | the case file on disk, when the case sets `disk` | stated only |

A stated `mode` never passes on a message: when the bar shows "3 lines
yanked" the reading is `null` and the case says so. State `bar` where a
message is the point. Option counts, node names, undo state and geometry
are never recorded.

## Expectations

- **Stated** (`expect`, `by_godot`): Vim behaviour across Godot's edits,
  history issues, godot-vim's own features. Written by hand and reviewed;
  nobody runs Vim to produce them.
- **Vanilla** (`oracle: "vanilla"`): where Godot decides, such as popup keys,
  pairs, Enter and indent. `--record` writes `vanilla/<godot>.jsonl`: a
  header (format, fps, root size, final settle, a hash of the template files
  and `keys.tsv`) and one line per case with a hash of the case's inputs.
  A changed case, template or probe format fails the case with "no
  recording, or a stale one". `--record --only a,b` replaces those lines
  only. CI never records into the tree; its weekly and manual runs record
  into an artifact.
- **Vacuity guard.** `--record` refuses a case whose `end` equals its `start`
  although the keys type text, unless the case says `unchanged`.
- **Intentional differences** from vanilla are stated with a `note`, so
  `known-failures.txt` lists defects only.

## Pins

```
4.7.2-stable::issue34-race 9ef5d99c   # completion accepted in the same frame as the next key: ...
4.5-stable::doc-k:log   # CanvasItem RID and ObjectDB instances leak at exit after K on 4.5; ...
```

One line per Godot version, case and channel. The behaviour channel carries
an 8-hex digest of every compared value the case observed, so a pinned case
that fails differently shows up. The `:log` channel has no digest.

| Outcome | Meaning |
|---|---|
| NEW FAILURE | a case fails that is not pinned: fix it, or pin it with its issue |
| NOW PASSING | a pinned case passes: confirm the fix and delete the line |
| CHANGED FAILURE | a pinned case fails with a different digest: read `SUITE-OBS`, then re-pin |
| PINNED BUT SKIPPED | a pinned case is skipped on this version: delete the line |

Every outcome fails the run. Every pin needs a reason after `#`; `--bless`
keeps the reasons it has and writes `TODO` on new lines, which the check
rejects. A failing job prints the exact lines to add, so a version without a
local binary can be updated by hand. If a canary (`canary-*`,
`setup-undo-floor`) fails, the run says `HARNESS BROKEN` first.

## Logs

`allow-log.txt` lines are `<godot-prefix> <case or *> <regex>`; the prefix
matches the start of the version, so `4.5` covers 4.5-stable and any 4.5
patch. `--record` writes the problem lines vanilla printed to
`vanilla/<godot>.log`, and a plugin run allows exactly those lines for that
case.

## Fixtures

`fixtures/<name>/` may hold:

| Path | Installed as |
|---|---|
| `res/` | files in the project root, such as `res://.godot-vimrc` (loaded under the Sandbox) |
| `user/` | files in `user://`, such as the trusted `user://.godot-vimrc` |
| `home/` | files in HOME |
| `project.godot` | appended to the project's `project.godot`, such as an `[editor_overrides]` section |
| `config/settings.txt` | `key = value` lines in Godot's text format, merged into the settings file; every key must be a registered setting. The way to set godot-vim's settings at startup |
| `config/editor_settings.tres` | a whole settings file, only for `upgrade-*` cases; made once by an old release (`upgrade-textwidth`: godot-vim 1.8.0 on Godot 4.5-stable) |

## Rules

- A snapshot taken while a timer is pending stays at least 6 frames (100 ms)
  from the timer's end: Godot's popup opens 17 frames after the last key,
  `timeoutlen` counts frames too. Results near a boundary move between
  `--fixed-fps` and `--realtime`.
- Waits are frames, not conditions, so a popup that opens too early fails.
- Fresh editor per case; files are restored by copying the project, never by
  undoing. Never keep a project copy across cases.
- A case that saves types valid code: Godot reloads saved scripts, and an
  invalid one logs a `SCRIPT ERROR`.
- Headless editors have no clipboard, IME, floating windows or real mouse;
  those stay out of the suite.
- A fast variant is its own case with `gap_ms: 20, settle_ms: 0`.
- Texts in cases: no em dashes, no paths into other docs.

## Versions

CI runs the suite and the smoke test on 4.5-stable (the floor), the latest
patch of each later minor (4.6.3-stable, 4.7.2-stable) and, as advisory, the
newest 4.8 snapshot (4.8-dev7) until 4.8-stable. Each binary is pinned by
SHA-512 in `.github/workflows/ci.yml`. Recordings and pins are keyed by the
version the running editor reports.

Moving a version: a blocking row moves in place to its minor's new stable
patch, at most monthly, in one change (record, bless, review the recording
diff); the dev row moves at each beta or on demand; 4.5-stable stays while
the plugin's minimum is 4.5. An older patch keeps a row only while it alone
exercises a workaround. Delete the old version's recording file and pin
lines in the same change.

The same change records Godot's undo history on the new version with
`scripts/godot-undo-trace.sh --godot BIN --record`, which writes
`scripts/godot-undo-trace/recorded/<godot>.txt`. A cargo test runs the same
steps on `MockTextEdit` and fails where the mock reads differently from any
recording; delete the old version's file with the rest.

## Earlier ids

Cases ported from the earlier live harnesses, with the ids they had there:
`P-` and `V-` from the Insert-mode suite, the others from the completion
set. Fast variants come from the old fast profile. A case named twice merges
rows that were the same.

| Case | Earlier ids |
|---|---|
| `type-word` | `P-type` |
| `type-word-fast` | `P-type`, fast |
| `enter-after-func` | `P-enter-func` |
| `enter-after-func-fast` | `P-enter-func`, fast |
| `enter-in-parens` | `P-enter-paren` |
| `enter-in-braces` | `P-enter-brace` |
| `enter-in-brackets` | `P-enter-bracket` |
| `issue20-gd-typed` | `P-enter-typed-brace` |
| `enter-after-todo` | `P-enter-todo` |
| `enter-after-comment` | `P-enter-comment` |
| `enter-mid-line` | `P-enter-mid` |
| `enter-no-autoindent` | `P-enter-noai` |
| `enter-no-autoindent-runtime` | `P-enter-noai-rt` |
| `pair-paren` | `P-pair-paren` |
| `pair-paren-fast` | `P-pair-paren`, fast |
| `pair-bracket` | `P-pair-bracket` |
| `pair-brace` | `P-pair-brace` |
| `pair-dquote` | `P-pair-dquote` |
| `pair-squote` | `P-pair-squote` |
| `pair-overtype` | `P-pair-overtype` |
| `pair-overtype-fast` | `P-pair-overtype`, fast |
| `pair-overtype-x` | `P-pair-overtype-x` |
| `pair-bs` | `P-pair-bs` |
| `pair-before-dot` | `P-pair-before-dot` |
| `pair-in-comment` | `P-pair-comment` |
| `pair-docstring` | `P-pair-docstring` |
| `pair-off` | `P-pair-off` |
| `pair-off-runtime` | `P-pair-off-rt` |
| `tab-indent` | `P-tab` |
| `stab-indent` | `P-stab` |
| `bs-indent` | `P-bs-tabs` |
| `tab-indent-spaces` | `P-tab-spaces` |
| `stab-indent-spaces` | `P-stab-spaces` |
| `bs-indent-spaces` | `P-bs-spaces` |
| `home-first-column` | `P-home` |
| `home-twice` | `P-home2` |
| `shift-left` | `P-sleft` |
| `shift-right` | `P-sright` |
| `ctrl-bs` | `P-cbs` |
| `ctrlz-typing` | `P-undo-run` |
| `ctrlz-typing-fast` | `P-undo-run`, fast |
| `palette` | `P-palette` |
| `comp-delay` | `P-comp-delay` |
| `comp-delay-fast` | `P-comp-delay`, fast |
| `comp-enter-at-once` | `P-comp-fast-enter`, completion `d3` |
| `comp-enter-at-once-fast` | `P-comp-fast-enter`, fast |
| `comp-enter` | `P-comp-enter`, completion `1` |
| `comp-enter-fast` | `P-comp-enter`, fast |
| `comp-down-enter` | `P-comp-down-enter`, completion `2` |
| `comp-down-enter-fast` | `P-comp-down-enter`, fast |
| `comp-tab` | `P-comp-tab`, completion `5` |
| `comp-tab-fast` | `P-comp-tab`, fast |
| `comp-replace` | `P-comp-replace`, completion `9` |
| `comp-replace-fast` | `P-comp-replace`, fast |
| `comp-replace-down` | `P-comp-replace-down`, completion `9b` |
| `comp-enter-tail` | `P-comp-enter-tail`, completion `9c` |
| `comp-pgdn` | `P-comp-pgdn`, completion `x7` |
| `comp-pgdn-fast` | `P-comp-pgdn`, fast |
| `comp-query` | `P-comp-query`, completion `x6` |
| `comp-nodepath` | `P-comp-nodepath`, completion `10` |
| `comp-accept-no-enter` | `P-comp-noenter` |
| `comp-accept-no-enter-tab` | `P-comp-noenter-tab` |
| `dot-insert` | `V-dot-insert` |
| `count-insert` | `V-count-insert` |
| `dot-pair` | `V-dot-pair` |
| `dot-o` | `V-dot-o` |
| `o-esc-indent` | `V-ai-strip` |
| `dot-cw` | `V-dot-cw` |
| `macro-insert` | `V-macro` |
| `macro-accept-ctrl-y` | `V-macro-accept` |
| `macro-accept-enter` | `V-macro-accept-cr` |
| `jk-escape` | `V-jk` |
| `jk-timeout` | `V-j-timeout` |
| `abbrev-space` | `V-abbrev` |
| `tw-comment` | `V-tw-comment` |
| `tw-code` | `V-tw-code` |
| `creg` | `V-creg` |
| `ctrl-o-dd` | `V-co-dd` |
| `cw` | `V-cw` |
| `ctrl-u` | `V-cu` |
| `ctrl-u-twice` | `V-cu2` |
| `dot-replace` | `V-replace` |
| `block-insert` | `V-block-I` |
| `multicursor-gb` | `V-mc-gb` |
| `multicursor-alt-click` | `V-mc-alt` |
| `undo-insert-session` | `V-undo-session` |
| `ctrlz-insert-session` | `V-ctrlz-session` |
| `ctrl-n-popup` | `V-cn` |
| `comp-down-early` | completion `2r` |
| `comp-down-down-up` | completion `3` |
| `comp-ctrl-n` | completion `4` |
| `comp-down-tab` | completion `6` |
| `comp-ctrl-y` | completion `7` |
| `comp-exact-enter` | completion `8` |
| `comp-sticky-row` | completion `11` |
| `comp-down-kenter` | completion `x1` |
| `comp-down-repeat` | completion `x2` |
| `comp-ctrl-p` | completion `x3` |
| `comp-filter-typed` | completion `x4` |
| `comp-down-stab` | completion `x5` |
| `comp-pgdn-pgup` | completion `x8` |
| `comp-accept-tab-only-enter` | completion `t1` |
| `comp-accept-tab-only-enter-tab` | completion `t2` |
| `comp-accept-tab-only-tab` | completion `t3` |
| `comp-accept-tab-only-down-enter` | completion `t4` |
| `comp-accept-tab-only-kenter` | completion `t5` |
| `comp-replace-no-stab` | completion `t6` |
| `comp-accept-space` | completion `t7` |
| `comp-query-alt-space` | completion `t8` |
| `comp-accept-empty` | completion `t9` |
| `comp-old-defaults-enter` | completion `r1` |
| `comp-old-defaults-enter-tail` | completion `r2` |
| `comp-old-defaults-senter-tail` | completion `r3` |
| `comp-old-defaults-down-tab` | completion `r4` |
| `comp-enter-before-delay` | completion `d1` |
| `comp-esc-in-delay` | completion `d2` |
| `comp-bs-closes` | completion `d4` |
| `comp-query-at-once-enter` | completion `d5` |
| `comp-bs-after-space` | completion `d6` |
| `comp-esc-a-in-delay` | completion `d7` |
| `comp-delay-short` | completion `d8` |
| `comp-disabled-enter` | completion `d9` |
| `comp-disabled-query` | completion `d10` |
| `comp-ctrl-e-in-delay` | completion `d11` |
| `comp-delay-long` | completion `d12` |
| `comp-enter-dot` | completion `dot1` |
| `comp-replace-dot` | completion `dot2` |
| `comp-enter-macro` | completion `mac1` |
| `comp-replace-macro` | completion `mac2` |
| `comp-two-carets` | completion `m1` |
| `comp-esc-open-next-frame` | completion `e15a` |
| `comp-esc-in-delay-next-frame` | completion `e15b` |
| `comp-esc-a-in-delay-next-frame` | completion `e15c` |
| `comp-semicolon` | completion `e19` |
| `comp-esc-open-shader` | completion `s15` |
| `comp-semicolon-shader` | completion `s19` |
| `comp-enter-shader` | completion `s1` |
| `comp-query-at-once` | completion `q0` |
| `jk-comp-enter` | completion `jk1` |
| `jk-comp-ob-enter` | completion `jk2` |
| `jk-comp-escape` | completion `jk3` |
| `jk-comp-down-enter` | completion `jk4` |
| `jk-comp-continue` | completion `jk5` |
| `jk-comp-ob-tab` | completion `jk6` |
| `comp-query-replace` | completion `q1` |
| `comp-query-vreplace` | completion `q2` |
| `comp-query-after-word` | completion `q3` |
| `comp-replace-mode-enter` | completion `q4` |
| `ctrl-at-inserts-last` | completion `q5` |
