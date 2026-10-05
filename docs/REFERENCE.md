# GodotVim Reference

Complete reference for settings, commands, modes, motions, operators, text objects, registers, and configuration syntax. For a quick overview, see the [README](../README.md).

---

## Table of Contents

- [Modes](#modes)
- [Motions](#motions)
- [Operators](#operators)
- [Text Objects](#text-objects)
- [Registers and Macros](#registers-and-macros)
- [Search and Replace](#search-and-replace)
- [Marks and Jumps](#marks-and-jumps)
- [Insert Mode](#insert-mode)
- [Visual Block](#visual-block)
- [Standard Ex Commands](#standard-ex-commands)
- [Undo Tree](#undo-tree)
- [Fold Commands](#fold-commands)
- [Vim Options (`:set`)](#vim-options-set)
- [Filetype Plugins](#filetype-plugins)
- [Settings](#settings)
- [Custom Commands](#custom-commands)
- [Preset Mappings](#preset-mappings)
- [.godot-vimrc Syntax](#godot-vimrc-syntax)
- [Panel Key Bindings (panelmap)](#panel-key-bindings-panelmap)
- [Security](#security)
- [Status Bar](#status-bar)
- [Line Numbers](#line-numbers)
- [Custom Cursor](#custom-cursor)

---

## Modes

| Mode | Entry | Cursor shape |
|------|-------|-------------|
| Normal | `Escape` | Block (white) |
| Insert | `i`, `a`, `o`, `O`, `c`, `s` | Beam (green) |
| Visual | `v` (char), `V` (line), `Ctrl-V` (block) | Block (orange) |
| Replace | `R` | Underline (red) |
| Command-line | `:`, `/`, `?` | Block (blinking) |
| Select | `gh` | Block (orange) |
| Operator-pending | `d`, `c`, `y`, `>`, etc. | Half block for `d`, `c`, `y` and the case operators, block otherwise; the hue rotates per operator |

---

## Motions

The full composable Vim grammar. Operators compose with motions and text objects, counts multiply, registers route output:

```
[count] [register] operator [count] motion/textobject
```

| Category | Keys |
|----------|------|
| Character | `h`, `j`, `k`, `l` |
| Word | `w`, `W`, `b`, `B`, `e`, `E`, `ge`, `gE` |
| Line | `0`, `^`, `$`, `g_`, `\|` |
| Screen line | `gj`, `gk`, `g0`, `g^`, `g$`, `gm`, `gM` (`g0`/`g^`/`g$` fall back to physical-line equivalents) |
| Document | `gg`, `G`, `{count}G`, `{n}%`, `go` (goto byte) |
| Find char | `f{c}`, `F{c}`, `t{c}`, `T{c}`, `;`, `,` |
| Search | `/`, `?`, `n`, `N`, `*`, `#`, `g*`, `g#`, `gn`, `gN` |
| Paragraph / Sentence | `{`, `}`, `(`, `)` |
| Brackets | `%`, `[(`, `])`, `[{`, `]}`, `[[`, `]]`, `[]`, `][` |
| Mark navigation | `]'`, `['` |
| Indent navigation | `[i`, `]i`, `[-`, `]-`, `[+`, `]+` |
| Changelist | `g;`, `g,` |
| Scroll | `Ctrl-D`, `Ctrl-U`, `Ctrl-F`, `Ctrl-B`, `Ctrl-E`, `Ctrl-Y` |
| Scroll position | `zz`, `zt`, `zb` |
| Screen position | `H`, `M`, `L` |

---

## Operators

| Key | Operation |
|-----|-----------|
| `d` | Delete |
| `c` | Change (delete + insert) |
| `y` | Yank (copy) |
| `>` | Indent |
| `<` | Outdent |
| `=` | Reindent |
| `gu` | Lowercase |
| `gU` | Uppercase |
| `g~` | Toggle case |
| `gq` | Format / wrap text |
| `gw` | Format / wrap text (keep cursor position) |
| `g?` | ROT13 encode |

---

## Text Objects

| Inner | Around | Object |
|-------|--------|--------|
| `iw` | `aw` | word |
| `iW` | `aW` | WORD |
| `is` | `as` | sentence |
| `ip` | `ap` | paragraph |
| `i"` | `a"` | double quotes |
| `i'` | `a'` | single quotes |
| `` i` `` | `` a` `` | backticks |
| `i(` / `i)` | `a(` / `a)` | parentheses |
| `i{` / `iB` | `a{` / `aB` | braces |
| `i[` | `a[` | brackets |
| `ie` | `ae` | entire buffer |
| `ii` | `ai` | indent level |
| `ib` | `ab` | any bracket, tightest enclosing `()`, `[]`, `{}`, `<>`; nearest pair as fallback |
| `iq` | `aq` | any quote (nearest `"`, `'`, `` ` ``) |
| `im` | `am` | symbol / identifier |

---

## Registers and Macros

**Named registers:** `"a`-`"z` (lowercase set, uppercase append).

**Numbered registers:** `"0` (last yank), `"1`-`"9` (delete history).

**Special registers:** `""` (unnamed), `"_` (black hole), `"+`/`"*` (system clipboard), `".` (last insert), `"%` (filename).

**Expression register:** `Ctrl-R =` in Insert/Command-line mode. Supports string literals, integer literals, `mode()`, and `nr2char(N)`. Complex VimL expressions are not supported.

**Macros:** `qa` to record into register `a`, `q` to stop, `@a` to replay, `@@` to repeat last.

**Dot repeat:** `.` replays the last edit (insert, operator, or command). `g.` repeats the last edit with intent (preserving cursor semantics for multi-cursor workflows).

---

## Search and Replace

- **Incremental search**: results highlight in real-time as you type `/pattern` or `?pattern`.
- **Live substitute match highlighting**: `:s/old/new/g` highlights match regions in yellow as you type the pattern, showing exactly what will be affected before Enter (`inccommand` setting). Note: highlights the match locations, not the replacement text.
- **Regex support**: Vim-compatible regex with all four magic modes.
- **Search commands**: `*`, `#`, `gn`, `gN`, `n`, `N`, `hlsearch`, `:noh`.

---

## Marks and Jumps

- **Local marks:** `'a`-`'z` (line), `` `a ``-`` `z `` (exact position).
- **Global marks:** `'A`-`'Z` (cross-buffer).
- **Special marks:** `''` (last jump), `'.` (last edit), `'^` (last insert).
- **Jump list:** `Ctrl-O` (older), `Ctrl-I` (newer).
- **Change list:** `g;` (older change), `g,` (newer change).
- **Last visual:** `gv` (reselect last visual selection).

---

## Insert Mode

All standard insert-mode keybindings:

| Key | Action |
|-----|--------|
| `Ctrl-R {reg}` | Insert contents of register (`=` register: string/int literals, `mode()`, `nr2char(N)` only) |
| `Ctrl-W` | Delete word before cursor |
| `Ctrl-U` | Delete to start of line |
| `Ctrl-O` | Execute one Normal-mode command, then return to Insert |
| `Ctrl-A` | Re-insert last inserted text |
| `Ctrl-@` | Trigger the completion popup (claimed by `editor.completion`; `panelunmap editor.completion <C-@>` restores Vim's insert-last-text-and-exit) |
| `Ctrl-T` | Increase indent of current line |
| `Ctrl-D` | Decrease indent of current line |
| `Ctrl-E` | Insert character from line below (with the completion popup visible: close the popup) |
| `Ctrl-Y` | Insert character from line above (with the completion popup visible: accept the candidate) |
| `Ctrl-V {char}` | Insert literal character / unicode codepoint |
| `Ctrl-G u` | Break undo sequence |
| `Ctrl-G U` | Don't break undo on next cursor movement |
| `Ctrl-N` | Next completion candidate, opening the popup if closed |
| `Ctrl-P` | Previous completion candidate, opening the popup if closed |
| `Ctrl-Space` | Trigger completion menu (same keystroke as `Ctrl-@`: Godot reports Ctrl+Space as Ctrl+`@`) |

While the completion popup is visible, the `editor.completion` bindings take
these keys first; each is a `panelmap` rule you can change:

| Key | While the popup is visible |
|-----|----------------------------|
| `Ctrl-Y` | Accept the candidate, chosen or not (`require_selection=0`) |
| `Ctrl-E` | Close the popup, keeping what you typed and staying in Insert |
| `Tab`, `Enter` | Accept **only a candidate you explicitly selected**; otherwise the popup closes and the key does its ordinary job (indent, newline) |
| `Ctrl-N`, `Ctrl-P` | Move the selection, wrapping |
| `Up`, `Down` | Hand the key to Godot's own popup handling |
| `Esc` | Not bound: one press closes the popup **and** leaves Insert, through the engine |

Auto-pair insertion for `()`, `[]`, `{}`, `""`, `''`, `` `` `` is GodotVim's own
reimplementation of Godot's `CodeEdit` behaviour, not a delegation to it
(`src/effects/auto_brace.rs`): `handle_unicode_input` is not callable from gdext.
The pair list comes from `get_auto_brace_completion_pairs()` on the attached
`CodeEdit` rather than from a hardcoded table, so it follows what Godot's editor
is configured with.

---

## Visual Block

Visual block mode (`Ctrl-V`) supports multi-line editing:

| Key | Action |
|-----|--------|
| `I` | Insert text at the beginning of each selected line |
| `A` | Append text at the end of each selected line |
| `c` / `s` | Change the selected block (delete + insert on each line) |
| `r{c}` | Replace every character in the block with `{c}` |
| `>` / `<` | Indent / outdent selected lines |
| `d` / `x` | Delete the block |
| `y` | Yank the block |
| `$` | Extend selection to end of each line (ragged block) |

---

## Standard Ex Commands

In addition to the [Godot-specific commands](#custom-commands), the following standard Vim ex-commands are supported:

### Substitution and Global

| Command | Description |
|---------|-------------|
| `:[range]s/pattern/replacement/[flags]` | Substitute within range |
| `:[range]g/pattern/cmd` | Execute cmd on matching lines |
| `:[range]v/pattern/cmd` | Execute cmd on non-matching lines |
| `:&`, `:&&` | Repeat last substitute (without / with flags) |

### Line Operations

| Command | Description |
|---------|-------------|
| `:[range]d [reg]` | Delete lines (optionally into register) |
| `:[range]y [reg]` | Yank lines (optionally into register) |
| `:[range]m {address}` | Move lines to address |
| `:[range]t {address}` / `:co` | Copy lines to address |
| `:[range]j` | Join lines |
| `:[range]sort [options]` | Sort lines |
| `:[range]put [reg]` | Put register contents after line |
| `:[range]retab` | Replace tabs with spaces (or vice versa) |
| `:[range]left [indent]` | Left-align lines |
| `:[range]right [width]` | Right-align lines |
| `:[range]center [width]` | Center lines |
| `:[range]norm {commands}` | Execute normal-mode commands on each line |
| `:[range]!{cmd}` | Filter range through external command |

### Information

| Command | Description |
|---------|-------------|
| `:reg [names]` | Display register contents |
| `:marks [args]` | List marks |
| `:jumps` | Show jump list |
| `:changes` | Show change list |
| `:messages` | Show message history |
| `:panelmap` | List every panel binding, and every vimrc line that was rejected ([details](#panel-key-bindings-panelmap)) |
| `:panelmap {keys}` | Explain how one key resolves against the panel that has focus right now |

### Undo

| Command | Description |
|---------|-------------|
| `:earlier {N}` / `:earlier {time}` | Travel backward in undo tree (count, time, or save-based) |
| `:later {N}` / `:later {time}` | Travel forward in undo tree |
| `:undolist` | Show undo history |

### Buffers and Tabs

| Command | Description |
|---------|-------------|
| `:w` / `:write` | Save the current file. `:w {path}` writes elsewhere, subject to File Access Scope. |
| `:q` / `:quit` | Close the current tab. Refuses on unsaved changes; `:q!` discards them. |
| `:wq` / `:x` | Save the current file, then close the tab. |
| `:e {path}` / `:edit` | Open a file. `:e!` on the file already open reloads it from disk, discarding changes. |
| `:qa` / `:qall` | Close the current tab. It runs the same handler as `:q` and does not close every tab. |
| `:bn` / `:bnext` | Next buffer |
| `:bp` / `:bprev` | Previous buffer |
| `:b {number}` | Switch to buffer by number |
| `:bf` / `:bfirst` | First buffer |
| `:bl` / `:blast` | Last buffer |
| `:ls` / `:buffers` | List open buffers |
| `:tabnew {path}` | Open file in new tab |
| `:tabn` / `:tabnext` | Next tab |
| `:tabp` / `:tabprev` | Previous tab |
| `:tabc` / `:tabclose` | Close tab |

### Configuration and Misc

| Command | Description |
|---------|-------------|
| `:set {option}[={value}]` | Set a Vim option |
| `:setlocal {option}[={value}]` | Set an option for the current script only (see [Filetype Plugins](#filetype-plugins)) |
| `:echo {expr}` | Echo expression |
| `:!{cmd}` | Execute shell command (when enabled) |
| `:actionlist [filter]` | List available Godot editor actions |
| `:mkvimrc` | Write a starter `res://.godot-vimrc`: header, `let mapleader`, `set timeoutlen`, and all 20 presets commented out. Refuses if the file exists; `:mkvimrc!` overwrites. |
| `:source` | Re-read the active `.godot-vimrc` from disk. There is no file watcher, so this is how a hand edit takes effect. |

---

## Undo Tree

Undo, redo, and time-based navigation.

| Key / Command | Action |
|---------------|--------|
| `u` | Undo |
| `Ctrl-R` | Redo |
| `g-` | Move to the previous state in the undo tree; `{count}g-` for N steps |
| `g+` | Move to the next state in the undo tree; `{count}g+` for N steps |
| `:earlier {N}` | Go back N changes |
| `:earlier {N}s` / `{N}m` / `{N}h` | Go back by time (seconds, minutes, hours) |
| `:earlier {N}f` | Go back N file saves |
| `:later {N}` | Go forward N changes |
| `:later {N}s` / `{N}m` / `{N}h` | Go forward by time |
| `:later {N}f` | Go forward N file saves |
| `:undolist` | Display undo history |
| `:undotree` | Draw the undo tree |

---

## Fold Commands

| Key | Action |
|-----|--------|
| `zo` | Open fold under cursor |
| `zc` | Close fold under cursor |
| `za` | Toggle fold under cursor |
| `zM` | Close all folds in buffer |
| `zR` | Open all folds in buffer |

---

## Vim Options (`:set`)

The following Vim options are supported via `:set`, `:setlocal`, and `.godot-vimrc`. `:set` changes the global value and the current script's own value; `:setlocal` changes only the current script's, which it keeps when you switch scripts and come back. In a `.godot-vimrc`, which no script is open for, `:setlocal` acts like `:set`.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `ignorecase` / `ic` | `bool` | `false` | Case-insensitive search |
| `smartcase` / `scs` | `bool` | `false` | Override `ignorecase` when pattern has uppercase |
| `wrapscan` / `ws` | `bool` | `true` | Searches wrap around end of file |
| `hlsearch` / `hls` | `bool` | `true` | Highlight all search matches |
| `incsearch` / `is` | `bool` | `true` | Incremental search |
| `expandtab` / `et` | `bool` | (from Godot) | Use spaces instead of tabs |
| `tabstop` / `ts` | `int` | (from Godot) | Number of spaces a tab counts for |
| `shiftwidth` / `sw` | `int` | (from Godot) | Number of spaces for indent |
| `scrolloff` / `so` | `int` | `5` | Minimum lines above/below cursor |
| `textwidth` / `tw` | `int` | `0` | Width at which typing in Insert mode breaks lines, when `formatoptions` allows it (`t` for text, `c` for comments). The break happens when you type a non-blank character past this display column (a tab counts as `tabstop` columns, a wide character as 2), at the last blank before the cursor. `0` never breaks while typing; `gq` then formats at 79. Seeded from the **Textwidth** setting; changing that setting later wins over an earlier `:set tw` or `:setlocal tw` in every script, and unrelated Editor Settings changes leave it alone. A script's filetype plugin decides what may wrap: in GDScript only comments do, see [Filetype Plugins](#filetype-plugins). `wrapmargin` is not supported. |
| `formatoptions` / `fo` | `string` | `tcqj` | Vim's flags for automatic formatting. `t` wraps text and `c` wraps comments (inserting the comment leader) at `textwidth`, `q` lets `gq` format comments, `l` leaves lines that were already long when the insert started. Also acted on: `w`, `2`, `v`, `b`, `1`, `p`; `M` and `B` by `gq` only. Accepted but without effect yet: `r`, `o`, `/`, `a`, `n`, `m`, `]`, `j`. An unknown letter is refused with `E539`. Takes `+=`, `-=` and `^=`, one flag at a time: `:set fo-=t`. Set per script by the filetype plugins. |
| `comments` / `com` | `string` | `s1:/*,mb:*,ex:*/,://,b:#,:%,:XCOMM,n:>,fb:-` | Vim's comment leaders, used by `c` and `q` in `formatoptions` and by `gq`. Same syntax as Vim (`b:#`, `s1:/*,mb:*,ex:*/`). Takes `+=`, `-=` and `^=` on whole items. Set per script by the filetype plugins. |
| `commentstring` / `cms` | `string` | `// %s` | Comment template. Set per script from Godot's comment delimiters (`# %s` in GDScript, `// %s` in shaders and C#) and by the filetype plugins, so a vimrc `set commentstring` reaches only scripts with neither; `:setlocal commentstring` overrides it in one script. |
| `timeoutlen` / `tm` | `int` | `1000` | Mapping timeout in milliseconds |
| `number` / `nu` | `bool` | `false` | Show line numbers. Engine-only: nothing in GodotVim reads it, and the gutter is driven by the **Line Numbers** setting. |
| `relativenumber` / `rnu` | `bool` | `false` | Show relative line numbers. Engine-only: nothing in GodotVim reads it, and the gutter is driven by the **Line Numbers** setting. |
| `inccommand` / `icm` | `string` | `nosplit` | Highlight substitute match regions as you type |
| `clipboard` | `string` | `""` | Clipboard integration |
| `iskeyword` / `isk` | `string` | (default) | Characters considered part of a word |
| `whichwrap` / `ww` | `string` | `b,s` | Keys that wrap across lines |
| `virtualedit` / `ve` | `string` | `""` | Allow cursor beyond end of line |
| `selection` / `sel` | `string` | `inclusive` | Visual selection behavior |

---

## Filetype Plugins

Vim gives each language its own formatting through filetype plugins: when a
buffer gets its filetype, a plugin runs a few `:setlocal` lines. GodotVim
does the same each time a script editor tab first gets the Vim cursor.

### Detection

The filetype comes from what Godot says about the editor, in this order:

1. The script's class, for script tabs: `GDScript` (files and built-in
   scripts in a scene) is `gdscript`, `CSharpScript` is `cs`.
2. The syntax highlighter Godot picked for the file, which you can change
   from the script editor's **Syntax Highlighter** menu: GDScript,
   GDShader (`gdshader`), JSON (`json`), Markdown (`markdown`) and
   ConfigFile (`cfg`). Godot gives **Plain Text** to every text file it has
   no highlighter for, so for Plain Text the file's extension decides, read
   from the script list: `.txt` is `text`, `.yml` and `.yaml` are `yaml`,
   `.xml` is `xml`, `.toml` is `toml` (and `.json`, `.md` and `.cfg` keep
   their own filetype if you switched them to Plain Text). Any other
   extension, such as `.log`, has no filetype, as in Vim. So does a file
   whose entry the script list does not show: one hidden by the script
   list's filter when its tab first gets the cursor, or a new file not yet
   saved.
3. The comment delimiters: `#` means `gdscript`. Anything else gives no
   filetype, and no plugin runs.

The detected name is also what filetype-specific mappings match.

### What each plugin sets

Every line is transcribed from Vim 9.1's runtime ftplugin of the same name,
limited to the options GodotVim formats with. No plugin turns on `r` or `o`.
Indent is not touched: it always follows the script editor.

| Filetype | Lines | Source |
|----------|-------|--------|
| `gdscript` | `setlocal commentstring=#\ %s comments=b:##,b:# formatoptions-=t formatoptions+=cql` | `gdscript.vim` for `commentstring`; `comments` and `formatoptions` deviate, see below |
| `gdshader` | `setlocal comments=sO:*\ -,mO:*\ \ ,exO:*/,s1:/*,mb:*,ex:*/,:// commentstring=//\ %s formatoptions-=t formatoptions+=cql` | `gdshader.vim`; `formatoptions` as in `c.vim` without `r` and `o` |
| `json` | `setlocal formatoptions-=t comments= commentstring=` | `json.vim` |
| `markdown` | `setlocal comments=fb:*,fb:-,fb:+,n:> commentstring=<!--\ %s\ --> formatoptions+=tcqln formatoptions-=r formatoptions-=o` | `markdown.vim` (`n` has no effect yet) |
| `text` | `setlocal comments=fb:-,fb:*,n:> commentstring=` | `text.vim` |
| `cs` | `setlocal formatoptions-=t formatoptions+=cql comments=sO:*\ -,mO:*\ \ ,exO:*/,s1:/*,mb:*,ex:*/,:///,:// commentstring=//\ %s` | `cs.vim` without `r` and `o` |
| `cfg` | `setlocal commentstring=#\ %s formatoptions-=t formatoptions+=cql` | `cfg.vim` without `r` and `o` |
| `yaml` | `setlocal comments=:# commentstring=#\ %s formatoptions-=t formatoptions+=cql` | `yaml.vim` without `r`, `o` and `expandtab` |
| `xml` | `setlocal commentstring=<!--\ %s\ --> comments=s:<!--,e:--> formatoptions-=t formatoptions+=cql` | `xml.vim` without `r` and `o` |
| `toml` | `setlocal commentstring=#\ %s comments=:#` | `toml.vim` |
| anything else | nothing | |

With the default `textwidth=0` none of this wraps anything. Set a width (the
**Textwidth** setting, or `:set tw=100`) and:

- **GDScript, shader and C# code never wraps; comments do.** A `#`, `##`,
  `//` or `///` comment line breaks at the last blank before the width,
  and the new line starts with the same indent and leader, so the script
  still parses. A trailing comment after code (`x = 1  # note`) counts as
  code.
- **JSON, `.cfg`, YAML and XML values never wrap**; `#` comments in
  `.cfg` and YAML files do. A break inside a quoted `.cfg` value would
  make it a multi-line string, and one inside a YAML value breaks the file.
- **Markdown and `.txt` files wrap as prose**, as in Vim. So do TOML
  files, whose `toml.vim` keeps `t`: `:setlocal fo-=t` stops it in one
  file. A file with no filetype (`.log`, or anything detection cannot
  place) keeps the global `formatoptions`, which has `t`, as in Vim.

**GDScript deviates from Vim on purpose.** Vim's `gdscript.vim`, like
`python.vim`, leaves `formatoptions` alone, so `t` wraps code as you type.
In GDScript a newline ends the statement, so a break outside brackets is a
parse error or silently changes what the line means. GodotVim drops `t` and
adds `c`, `q` and `l`, as `c.vim` does for C. `comments` is `b:##,b:#`
rather than Python's `b:#,fb:-`: `##` doc comments continue with `##`, and
without `fb:-` a code line that starts with `- ` (a continued expression)
is not mistaken for a list item and wrapped. A `# ` line inside a
multi-line string still counts as a comment.

### Changing it

- `:setlocal fo+=t` lets code wrap in the current script, the Vim way. It
  can break GDScript.
- `:set fo-=t` or `:set comments^=b:##` work as in Vim. At startup a
  `.godot-vimrc` sets the global value, which the plugins then adjust per
  script, as `setlocal` in an ftplugin overrides your vimrc in Vim.
  Sourcing the vimrc again (`:source`, or saving key mappings) also writes
  its options into the current script, as Vim's `:source` does, and the
  plugin does not run again for it: a vimrc `set fo+=t` sourced then lets
  code wrap in that script until `:setlocal fo-=t`.
- A plugin runs once per script. A `:setlocal` you make afterwards stays
  when you switch scripts and come back. If the script's filetype changes
  (you pick another syntax highlighter), the old plugin is undone first,
  each option it set going back to the global value, like Vim's
  `setlocal fo<`.
- **Turning plugins off:** the **Filetype Plugin** setting, or
  `filetype plugin off` in a `.godot-vimrc` (`filetype plugin on` turns
  them back on; `filetype off` stops detection altogether). Whichever you
  changed last wins; turning the setting on also turns detection back on,
  as `filetype plugin on` does. The current script follows at once and
  other scripts the next time you switch to them, their plugin undone. The
  `commentstring` from Godot's delimiters is set either way.

---

## Settings

All settings are in **Editor > Editor Settings > Plugins > GodotVim**.

> **Note:** Indent settings (tab size, spaces vs tabs) are read from Godot's CodeEdit, not GodotVim. Configure them in **Editor > Editor Settings > Text Editor > Behavior > Indent**.

### General

| Setting | Key | Type | Default | Description |
|---------|-----|------|---------|-------------|
| Enabled | `plugins/GodotVim/enabled` | `bool` | `true` | **Master switch**: global, per-user editor setting (stored in `editor_settings-*.tres`, not `project.godot`). When `false` the plugin is inert: no keybindings, overlays, input handling, input signal handlers, filesystem-prompt interception, or `.godot-vimrc` sourcing occur. What remains connected while inert: the settings listener (so re-enable is observed), two idle one-shot timers (the mapping timer and the panel timer) whose connections persist but never fire while input is off, filesystem Callables (plain data), the process-global panic hook, and the always-loaded native extension. To disable only one project, turn off the plugin in that project's **Project Settings → Plugins** (writes `project.godot`). |
| Log Level | `plugins/GodotVim/log_level` | `enum` | `Off` | `Off`, `Error`, `Warn`, `Info`, `Debug`, `Trace`. |

### Editor Behavior

| Setting | Key | Type | Default | Description |
|---------|-----|------|---------|-------------|
| Scroll Off | `plugins/GodotVim/editor/scrolloff` | `int` | `5` | Minimum lines above/below cursor (0-20). |
| Textwidth | `plugins/GodotVim/editor/textwidth` | `int` | `0` | Vim `textwidth`, 0 to 200: the width at which typing breaks lines where `formatoptions` allows it. In GDScript and shaders only comments wrap, JSON never does (see [Filetype Plugins](#filetype-plugins)). `0` never breaks; `gq` formats at 79 when it is `0`. Shown as **Textwidth** under **Godot Vim > Editor**; filter for `textwidth` to find it. |
| Filetype Plugin | `plugins/GodotVim/editor/filetype_plugin` | `bool` | `true` | Vim's `filetype plugin on`: run the [filetype plugins](#filetype-plugins), and turn detection back on after a vimrc `filetype off`. Off is `filetype plugin off`. A change here wins over the vimrc line until the vimrc is sourced again. |
| Clipboard | `plugins/GodotVim/editor/clipboard_enabled` | `bool` | `false` | Sync Vim registers with system clipboard. |
| Ignore Case | `plugins/GodotVim/editor/ignorecase` | `bool` | `false` | Case-insensitive search. |
| Smart Case | `plugins/GodotVim/editor/smartcase` | `bool` | `false` | Uppercase in pattern overrides Ignore Case. |
| Line Numbers | `plugins/GodotVim/editor/line_number_mode` | `enum` | `Hybrid` | `None`, `Absolute`, `Relative`, `Hybrid`. |
| Inccommand | `plugins/GodotVim/editor/inccommand` | `enum` | `nosplit` | Live `:s` preview. `nosplit` = enabled, `off` = disabled. |
| Highlight Yank | `plugins/GodotVim/editor/highlight_yank_duration` | `int` | `150` | Yank highlight duration in ms, 0 to 5000 (0 = disabled). |

### Cursor

| Setting | Key | Type | Default | Description |
|---------|-----|------|---------|-------------|
| Enabled | `plugins/GodotVim/cursor/enabled` | `bool` | `true` | Custom cursor overlay (disable for native caret). **NOT the master toggle, see General → Enabled.** |
| Lerp Speed | `plugins/GodotVim/cursor/lerp_speed` | `float` | `25.0` | Cursor glide speed, on a log-scaled slider from 1 to 500. Higher is snappier; 500 settles within one 60 Hz frame. Values above 500 can be typed in but are not perceptibly faster. |
| Underline Height | `plugins/GodotVim/cursor/underline_height` | `float` | `4.0` | Replace-mode underline height in pixels. |
| Normal Color | `plugins/GodotVim/cursor/normal_mode_color` | `Color` | `#FFFFFF` | Cursor color in Normal mode. |
| Insert Color | `plugins/GodotVim/cursor/insert_mode_color` | `Color` | `#55FF7F` | Cursor color in Insert mode. |
| Visual Color | `plugins/GodotVim/cursor/visual_mode_color` | `Color` | `#FFB855` | Cursor color in Visual mode. |
| Replace Color | `plugins/GodotVim/cursor/replace_mode_color` | `Color` | `#FF333399` | Cursor color in Replace mode. |
| Operator Mode Color | `plugins/GodotVim/cursor/operator_mode_color` | `Color` | `#FFB855` | Cursor color in Operator-pending mode. |
| Command Mode Color | `plugins/GodotVim/cursor/command_mode_color` | `Color` | `#FFFFFF` | Cursor color in Command-line mode. |

Glide time to settle, in ms (`ln(2 * distance) / speed`):

| speed | one line (20 px) | half screen (500 px) | full screen (2000 px) |
|-------|-----|-----|-----|
| 1 | 3689 | 6908 | 8294 |
| 25 (default) | 148 | 276 | 332 |
| 100 | 37 | 69 | 83 |
| 500 | 7 | 14 | 17 |

The animation is exponential decay, so it is frame-rate independent in wall
clock: 500 takes the same 17 ms at 60, 144 and 240 Hz. Only the cursor's own
motion is animated. Scrolling, folding and resizing translate it in the same
frame, so it never trails the text it sits on.

> **Note:** Line highlighting, cursor blink, and beam width are controlled by Godot's native settings under `text_editor/appearance/caret/`.

### Key Mapping

| Setting | Key | Type | Default | Description |
|---------|-----|------|---------|-------------|
| Timeout Length | `plugins/GodotVim/mapping/timeoutlen` | `int` | `1000` | Timeout for ambiguous mappings in ms. |
| Config File Path | `plugins/GodotVim/mapping/config_file_path` | `string` | `""` | Path to `.godot-vimrc`. Empty = auto-resolve. |

### Input

| Setting | Key | Type | Default | Description |
|---------|-----|------|---------|-------------|
| Passthrough Keys | `plugins/GodotVim/input/passthrough_keys` | `string` | `""` | Comma-separated keys bypassing Vim (e.g. `<C-v>,<C-a>`). |

Godot builds the settings tree from the key path, so this one renders under
**Input**, not under **Key Mapping** where its two former neighbours sit.

### Security Settings

| Setting | Key | Type | Default | Description |
|---------|-----|------|---------|-------------|
| Shell Execution | `plugins/GodotVim/security/shell_execution` | `enum` | `Disabled` | Allow `:!` commands. |
| File Access Scope | `plugins/GodotVim/security/file_access_scope` | `enum` | `Project Only` | Restrict file ops to `res://` and `user://`. |
| Project Vimrc | `plugins/GodotVim/security/project_vimrc` | `enum` | `Sandbox` | `Disabled`, `Sandbox`, `Trusted`. |

### Status Bar Colors

All configurable in **Editor > Editor Settings > Plugins > GodotVim > Status Bar**:

| Setting | Key | Type | Default | Description |
|---------|-----|------|---------|-------------|
| Normal BG | `plugins/GodotVim/status_bar/normal_bg` | `Color` | `rgb(0.5, 0.6, 0.8)` | Background in Normal mode. |
| Insert BG | `plugins/GodotVim/status_bar/insert_bg` | `Color` | `rgb(0.6, 0.8, 0.5)` | Background in Insert mode. |
| Visual BG | `plugins/GodotVim/status_bar/visual_bg` | `Color` | `rgb(0.8, 0.5, 0.5)` | Background in Visual mode. |
| Replace BG | `plugins/GodotVim/status_bar/replace_bg` | `Color` | `rgb(0.9, 0.6, 0.3)` | Background in Replace mode. |
| Command BG | `plugins/GodotVim/status_bar/command_bg` | `Color` | `rgb(0.157, 0.173, 0.204)` | Background in Command-line mode. |
| Recording BG | `plugins/GodotVim/status_bar/recording_bg` | `Color` | `rgb(0.9, 0.2, 0.2)` | Background while recording a macro. |
| Text FG | `plugins/GodotVim/status_bar/text_fg` | `Color` | `#FFFFFF` | Foreground text color. |
| Error FG | `plugins/GodotVim/status_bar/error_fg` | `Color` | `rgb(1.0, 0.3, 0.3)` | Foreground color for error messages. |

---

## Custom Commands

Godot-specific ex-commands, in addition to standard Vim commands.

| Command | Alias | Description |
|---------|-------|-------------|
| `:run` | `:play` | Run main scene (F5) |
| `:runcurrent` | `:playcurrent` | Run current scene (F6) |
| `:stop` | | Stop running scene |
| `:zen` | | Distraction-free mode |
| `:unzen` | | Exit distraction-free mode |
| `:GodotBreakpoint` | | Toggle breakpoint |
| `:GodotContinue` | `:cont` | Debugger continue |
| `:GodotNext` | `:next` | Step over |
| `:GodotStepIn` | `:stepin` | Step into |
| `:GodotStepOut` | `:stepout` | Godot's debugger exposes no step-out, so this continues instead and says so in the status bar |
| `:GodotPause` | `:pause` | Pause execution |
| `:FileSystem` | | Focus FileSystem dock |
| `:Inspector` | | Focus Inspector dock |
| `:Scene` | | Focus Scene tree dock |
| `:Script` | | Switch to Script editor |
| `:Output` | | Focus Output panel |
| `:save` | | Save current file |
| `:saveall` | | Save all scenes |
| `:savescene` | | Save current scene |
| `:mappings` | | Open key mapping dialog |
| `:panelmap` | | List all panel bindings and rejected config lines |
| `:panelmap {keys}` | | Explain how `{keys}` resolves where focus is now |
| `:perf` | | Show keystroke performance stats |
| `:vimdebug` | | Toggle debug annotations |
| `gd` | | Go to the definition of the symbol under the cursor |
| `K` | | Show Godot's documentation tooltip for the symbol under the cursor |

`gd` and `K` are Normal-mode keys rather than ex-commands. They are listed here
because they are Godot integrations rather than Vim engine features.

---

## Preset Mappings

20 built-in presets, togglable via `:mappings` dialog or `.godot-vimrc` preset markers.

| Keys | Action | Default |
|------|--------|---------|
| `jj` | Exit insert mode | off |
| `jk` | Exit insert mode | off |
| `<Space>w` | Save file | off |
| `<Space>W` | Save all | off |
| `<Space>n` | Next buffer | off |
| `<Space>p` | Previous buffer | off |
| `<Space>db` | Toggle breakpoint | off |
| `<Space>dc` | Continue | off |
| `<Space>dn` | Step over | off |
| `<Space>di` | Step in | off |
| `<Space>do` | Continue (Godot exposes no step-out) | off |
| `<Space>r` | Run main scene | off |
| `<Space>R` | Run current scene | off |
| `<Space>S` | Stop scene | off |
| `<Space>e` | FileSystem dock | off |
| `<Space>i` | Inspector dock | off |
| `<Space>s` | Script editor | off |
| `<Space>z` | Zen mode | off |
| `<Space>Z` | Exit zen mode | off |
| `<Esc>` | Clear search highlights | off |

---

## .godot-vimrc Syntax

Place a `.godot-vimrc` file at your project root (`res://.godot-vimrc`) or user directory (`user://.godot-vimrc`). Auto-detected on startup, hot-reloadable via `:source`.

### Supported Commands

| Syntax | Description |
|--------|-------------|
| `let mapleader = "x"` | Set leader key (must come before `<Leader>` mappings) |
| `set timeoutlen=N` | Mapping timeout in milliseconds |
| `set {option}` / `setlocal {option}` | Any option from [Vim Options](#vim-options-set); `setlocal` acts like `set` here |
| `filetype plugin on` / `filetype plugin off` | Turn the [filetype plugins](#filetype-plugins) on or off; `filetype on` / `filetype off` turn detection on or off. A leading `:`, a trailing `" comment` and `\| next-command` are accepted; the command after `\|` is not run. |
| `nmap` / `nnoremap` | Normal mode mapping |
| `imap` / `inoremap` | Insert mode mapping |
| `vmap` / `vnoremap` | Visual mode mapping |
| `omap` / `onoremap` | Operator-pending mode mapping |
| `cmap` / `cnoremap` | Command-line mode mapping |
| `map` / `noremap` | Normal + Visual + Operator-pending |
| `panelmap` | Bind a key outside the script editor: docks, panels, FileSystem, debugger, completion popup ([details](#panel-key-bindings-panelmap)) |
| `panelunmap` | Remove a panel binding |

The recursive forms (`map`, `nmap`, `vmap`, `imap`, `omap`, `cmap`) are stripped
from a committed `res://.godot-vimrc` under the default `Sandbox` policy whatever
they map to. Use the `noremap` forms there, or move the file to
`user://.godot-vimrc`. See [Security](#security).

### Key Notation

| Notation | Key |
|----------|-----|
| `<Esc>` | Escape |
| `<CR>` | Enter |
| `<Space>` | Space |
| `<Leader>` | Leader key value |
| `<C-x>` | Ctrl + x |
| `<S-x>` | Shift + x |
| `<A-x>` / `<M-x>` | Alt / Meta + x |
| `<Action>(shortcut path)` | Execute a Godot editor action by editor shortcut path |

### Godot Actions

Invoke any Godot editor shortcut by name using `<Action>` in mappings or `:action` from the command line:

```vim
" In .godot-vimrc: map Leader+s to save
nnoremap <Leader>s <Action>(editor/save_scene)

" From command line
:action editor/save_scene
:actionlist script_text_editor    " list actions matching a filter
```

Use `:actionlist` to browse all available shortcut paths, or see [Godot's default key mapping](https://docs.godotengine.org/en/stable/tutorials/editor/default_key_mapping.html) for the full reference with descriptions.

**Limitation:** Actions in `scene_tree/`, `spatial_editor/`, `canvas_item_editor/`, and `filesystem_dock/` categories will not fire while the text editor is focused. Godot's own code blocks these shortcuts when a text field has focus. Actions in `script_text_editor/`, `script_editor/`, and `editor/` categories work reliably from within the code editor.

### Preset Markers

Control built-in preset state in your config file:

```vim
" preset:enabled        <- active preset
nnoremap <Space>r :run<CR>

" preset:disabled       <- inactive preset
" inoremap jj <Esc>
```

---

## Panel Key Bindings (panelmap)

Everything GodotVim does **outside** the script editor is a table of bindings you can read, change and remove: moving focus between panels (`Ctrl-h/j/k/l`), navigating docks with `h/j/k/l`, the FileSystem file operations (`a`/`d`/`r`/`y`/`R`), the debugger keys (`J`/`K`/`G`/`y`), and the autocomplete popup (`Ctrl-N`/`Ctrl-P`/`Ctrl-Y`/`Ctrl-E`/`Tab`/`Enter`; `Esc` is deliberately not bound there).

Those 31 bindings are not hardcoded. They are `panelmap` lines the plugin writes for itself and hands to the same parser that reads your `.godot-vimrc`. Anything the defaults can express, you can express.

```
panelmap   [<flag> ...] <surface> <lhs> <target> [key=value ...]
panelunmap <surface> <lhs>
```

Zero config is still the default. If you never write a `panelmap` line, the shipped keyset is exactly what you get.

### Where the lines go

`panelmap` and `panelunmap` are ordinary `.godot-vimrc` lines. They live in the same file as your `nnoremap` mappings, in any order.

Exactly **one** config file is read, and the first hit wins:

1. **Editor Settings → Plugins → GodotVim → Key Mapping → Config File Path**, if you set it
2. `res://.godot-vimrc`, your project root, committed with the project
3. `user://.godot-vimrc`, per-user, shared by every project you open

There is no layering between files: if `res://.godot-vimrc` exists, `user://.godot-vimrc` is never read. After editing the file by hand, run `:source`, the binding table is rebuilt from scratch and swapped in atomically, so a broken line can never leave a half-built keyset live.

### Seeing what you have

| Command | What it prints |
|---------|----------------|
| `:panelmap` | Every live binding, grouped by surface, in the exact syntax you would paste back into a vimrc, and every line of your config that was **rejected** and the reason |
| `:panelmap {keys}` | One key, resolved against whatever has focus right now: the focus chain that was sampled, the surface stack, which rule won and on which surface, which gate stopped it if none did, and whether the key is consumed |

Both print to the **Output** panel, not the status bar, since a resolution trace is a dozen lines.

Two things to know about `:panelmap {keys}`. It samples **the focus you have while typing the command**, which is the command line, so run it with the panel you care about in mind and read the focus chain it prints back at you, which is the chain it actually used; because the completion popup can never be open at that moment, the report also prints a clearly labelled **counterfactual** section showing how the key would resolve if the script editor were in Insert mode with the popup open. And it answers for the key **as written**: a real keystroke on a non-QWERTY layout also carries a US-QWERTY position that a written left-hand side cannot reconstruct, so a `<physical>` rule may be reachable in practice without appearing in the trace.

A malformed line costs you that line and nothing else. The rest of your config still loads. `:panelmap` is the primary channel for those rejections and needs no setup at all.

To watch them scroll past as the file loads instead, set **Editor Settings → Plugins → GodotVim → Log Level** to `Debug`. **The default is `Off`**, which is exactly why a rejected binding can otherwise look like a key that simply stopped working: the diagnostic exists, it is just not being printed. `Debug` also reports how many bindings were rebuilt, and, for a project-level config under the `Sandbox` policy, which lines the sandbox stripped.

### Rebinding is `panelunmap` + `panelmap`

There is exactly one binding per (surface, key), and a second `panelmap` on the same surface and key replaces the first. But **adding a key never removes the old one**: there is no implicit "move this verb". To relocate a binding, unmap the old key and map the new one:

```vim
" Dock navigation on Colemak's home row instead of j/k.
panelunmap dock j
panelunmap dock k
panelmap <physical> dock n godotvim.item.next
panelmap <physical> dock e godotvim.item.prev
```

Without the two `panelunmap` lines, `j` and `k` would keep working alongside `n` and `e`.

Two more things you can do with `panelunmap`:

```vim
" Turn a key off entirely, d in the FileSystem dock no longer deletes.
panelunmap dock.filesystem d

" Hand a key back to Godot at one surface instead of turning it off.
" `native` stops the lookup here; the key is not passed further up the forest.
panelmap dock.filesystem <C-h> native
```

`panelunmap` and `native` are not the same thing. `panelunmap` removes the rule and lets the search continue to the parent surface; `native` is a rule that terminates the search and gives the keystroke to Godot.

### Surfaces, and why deeper wins

A **surface** is a named place in the editor UI. Surfaces form a tree, and a keystroke is resolved by walking from the surface that has focus **up to the root**, taking the first rule that matches. That is the whole specificity mechanism, there are no priorities to assign.

| Surface | Parent | What has focus there |
|---------|--------|----------------------|
| `panel` | (root) | The root of everything below. Reached by the upward walk and never claimed directly, this is where cross-panel keys belong. |
| `dock` | `panel` | Any focusable `Tree`, `ItemList` or `RichTextLabel`: Scene tree, Inspector, Output log, built-in docs. |
| `dock.filesystem` | `dock` | The FileSystem dock's tree or file list. |
| `dock.debugger` | `dock` | The Debugger panel's Stack Frames / Breakpoints trees. |
| `searchbox` | `panel` | A dock's filter `LineEdit` (the box `/` jumps to). |
| `prompt` | `panel` | GodotVim's own FileSystem create/rename prompt. |
| `editor.nav` | `panel` | The attached script editor in Normal, Visual or Operator-pending mode. |
| `editor.insert` |, (root) | The attached script editor in any *other* mode, Insert, Replace, Select. **Takes no bindings.** |
| `editor.completion` |, (root) | An **overlay**: the script editor in an insert-like mode, joined per keystroke rather than by focus. Grants the `POPUP` capability while the completion popup is visible. |
| `foreign` |, (root) | Somebody else's text input, a Project Settings field, an addon's editor. **Takes no bindings.** |
| `unknown` | `panel` | A focused control none of the above claimed, or no focus owner at all. |

Because `dock.filesystem` is deeper than `dock`, `r` renames a file in the FileSystem dock and moves to the previous item everywhere else, same key, two rules, no conflict:

```vim
panelmap dock r godotvim.item.prev
panelmap <physical> dock.filesystem r godotvim.fs.rename
```

Four surfaces stop the walk, in two different ways:

- `editor.insert` and `foreign` are **barriers**. Nothing is looked up there and no ancestor is consulted, which is why `Ctrl-H` still backspaces in a Project Settings field and in Insert mode. Any `panelmap` on them is rejected.
- `searchbox` and `prompt` are **sealed**: bare keys stop there and reach the control's own input handling, so you can type in a filter box, while keys carrying Ctrl, Alt or Meta keep walking up to `panel`, which is how `Ctrl-h/j/k/l` still escapes them.

Only the key **as typed** takes part in that walk. The US-QWERTY positional fallback (see `<physical>` below) is offered to every surface only after the typed key has been offered to all of them, so a positional guess on a deep surface can never beat what you actually pressed on a shallow one.

### Targets

| Target | Meaning |
|--------|---------|
| `godotvim.*` | A registered action id, see the table below. A typo is rejected at load, never a silently dead key. |
| `native` | Give this keystroke back to Godot at this surface and stop walking. |

The target vocabulary is deliberately closed. There is no shell form, no `:` command form, and no way for a binding to expand into another mapping, which is what makes it safe to honour `panelmap` lines from a committed project config (see [Security](#security)).

### Flags

Flags come first, in any order, each at most once.

| Flag | Effect | On `editor.completion` |
|------|--------|------------------------|
| `<physical>` | Also match this rule against the key's **US-QWERTY physical position**, when that differs from what was typed. Opt-in per rule. | Inert: the surface refuses the positional probe (see below) |
| `<void>` | Consume the keystroke whether or not the action succeeded, and stop the walk. Without it, a declining action lets the key fall through. | Live |
| `<norepeat>` | Ignore auto-repeat while the key is held. The rule fires once per press. | Live |
| `<shift>` | Also match this key with Shift held. Only meaningful for named keys (`<CR>`, `<Esc>`, `<Up>`…), `R` is already the shifted spelling of `r`. | Live |
| `<nowait>` | Fire immediately even if this key is also the first key of a longer sequence, instead of waiting `timeoutlen` for the rest. | Inert: multi-key sequences are refused there, so no wait can exist |

**Why the shipped defaults carry the flags they do**: these are not decoration, and dropping one has a specific consequence:

- The four `panel` chords are `<physical> <void> <norepeat>`.
  - **Drop `<physical>` and `Ctrl-h/j/k/l` silently stop working on non-QWERTY layouts.** On Dvorak, Colemak or AZERTY, the key in the QWERTY `j` position does not report `j`; the positional probe is the only thing that finds it, and it is offered only to rules that asked for it.
  - **Drop `<void>` and the chords leak to Godot at the edges of your layout.** `godotvim.focus.left` declines when there is no panel to the left; an elastic rule then hands `Ctrl-h` to Godot, which will do something else with it. `<void>` is what makes "no panel that way" a no-op instead of a surprise.
  - **Drop `<norepeat>` and holding `Ctrl-j` queues a focus-grab storm**: roughly twenty deferred focus changes a second for as long as the key is down.
- `dock` and `dock.filesystem` navigation and file-operation keys are `<physical>` for the same layout reason. `<CR>` and `<Esc>` on `dock` are not: named keys have no layout ambiguity.
- The two `searchbox` rules are `<shift>`, so Shift+Enter and Shift+Esc leave the filter box exactly like the unshifted keys, matching what the filter box did before it was a binding table.

If you rebind one of these, carry the flags across. The `:panelmap` listing prints every rule with its flags, so you can copy the line you are replacing.

Two flags are inert on `editor.completion`, for **two different reasons**, and `:panelmap {keys}` prints both: `<physical>` because the surface declares `refuses_positional: true` (inside the attached editor a positional guess would turn a Dvorak `Ctrl-p` into a completion key), and `<nowait>` because the surface is editor-reachable and therefore refuses multi-key left-hand sides at load, so there is never a wait to skip.

### Parameters

Trailing `key=value` pairs, at most **4** per rule. Values are **decimal integers only**: there is no string or enum form.

Parameters are honoured on every surface, `editor.completion` included.

| Parameter | Range | Meaning |
|-----------|-------|---------|
| `count` | `1`–`100` | Repeat the action this many times per keystroke. |
| `require_selection` | `0` or `1` (default `1`) | On `godotvim.completion.confirm`: with `1`, accept only a candidate you explicitly selected, cancelling the popup otherwise; with `0`, accept whatever is highlighted. |

```vim
" Half-page-ish movement in any dock.
panelmap dock <C-d> godotvim.item.next count=10
panelmap dock <C-u> godotvim.item.prev count=10
```

An out-of-range `count` is rejected at load rather than quietly clamped, because an unbounded repeat is a frozen editor rather than a slow keystroke.

### Actions

29 verbs.

**Needs** is what the focused control must be able to *do*, not what class it is. "A vertical cursor" is held by `Tree`, `ItemList` and `RichTextLabel`; "a hierarchy" only by `Tree`; "the FileSystem dock" is granted by the surface rather than the widget, which is what stops `panelmap dock a godotvim.fs.create` from creating files from a focused Scene tree. If the control cannot meet an action's needs, that rule is skipped and the keystroke keeps walking up, it does not die there.

The **`:action`** column marks the verbs that also work by name, from `:action {id}` or `<Action>({id})` in a mapping. The rest need a real keystroke on a real surface, because they act on whatever the keystroke was aimed at.

| Action id | Default binding | Does | Needs | `:action` |
|-----------|-----------------|------|-------|-----------|
| `godotvim.item.next` | `j` on `dock` | Move to the next item | a vertical cursor | |
| `godotvim.item.prev` | `k` on `dock` | Move to the previous item | a vertical cursor | |
| `godotvim.item.collapse` | `h` on `dock` | Collapse the current item | a hierarchy (`Tree`) | |
| `godotvim.item.expand` | `l` on `dock` | Expand the current item | a hierarchy (`Tree`) | |
| `godotvim.item.activate` | `<CR>` on `dock` | Open or activate the current item | activation (`Tree`, `ItemList`) | |
| `godotvim.dock.search` | `/` on `dock` | Focus the dock's filter box |, | |
| `godotvim.focus.editor` | `<Esc>` on `dock` | Return focus to the script editor |, | yes |
| `godotvim.focus.left` | `<C-h>` on `panel` | Move focus to the panel on the left |, | yes |
| `godotvim.focus.right` | `<C-l>` on `panel` | Move focus to the panel on the right |, | yes |
| `godotvim.focus.up` | `<C-k>` on `panel` | Move focus to the panel above |, | yes |
| `godotvim.focus.down` | `<C-j>` on `panel` | Move focus to the panel below |, | yes |
| `godotvim.focus.cycle_next` | *(unbound, see below)* | Cycle focus to the next panel |, | yes |
| `godotvim.focus.cycle_prev` | *(unbound, see below)* | Cycle focus to the previous panel |, | yes |
| `godotvim.search.accept` | `<CR>`, `<Esc>` on `searchbox` | Leave the filter box, keeping the filter | a text field | |
| `godotvim.fs.create` | `a` on `dock.filesystem` | Create a file or folder | the FileSystem dock | yes |
| `godotvim.fs.delete` | `d` on `dock.filesystem` | Delete the selected path | the FileSystem dock | yes |
| `godotvim.fs.rename` | `r` on `dock.filesystem` | Rename the selected path | the FileSystem dock | yes |
| `godotvim.fs.yank_path` | `y` on `dock.filesystem` | Copy the selected path to the clipboard | the FileSystem dock | yes |
| `godotvim.fs.refresh` | `R` on `dock.filesystem` | Rescan the filesystem | the FileSystem dock | yes |
| `godotvim.debugger.frame_next` | `J` on `dock.debugger` | Select the next stack frame or breakpoint | a vertical cursor | |
| `godotvim.debugger.frame_prev` | `K` on `dock.debugger` | Select the previous stack frame or breakpoint | a vertical cursor | |
| `godotvim.debugger.frame_last` | `G` on `dock.debugger` | Select the deepest stack frame | a vertical cursor | |
| `godotvim.debugger.yank_frame` | `y` on `dock.debugger` | Copy the selected row to the clipboard | a vertical cursor | |
| `godotvim.completion.trigger` | `<C-@>` on `editor.completion` | Open the completion popup |, | |
| `godotvim.completion.next` | `<C-n>` on `editor.completion` | Next candidate, opening the popup if closed |, | |
| `godotvim.completion.prev` | `<C-p>` on `editor.completion` | Previous candidate, opening the popup if closed |, | |
| `godotvim.completion.confirm` | `<Tab>`, `<CR>`, `<C-y> require_selection=0` on `editor.completion` | Accept the selected candidate; by default only one you explicitly chose (`require_selection`) | a visible popup | |
| `godotvim.completion.dismiss` | `<C-e>` on `editor.completion` | Close the popup, keeping insert mode | a visible popup | |
| `godotvim.completion.navigate` | `<Up>`, `<Down>` on `editor.completion` | Hand this key to CodeEdit's own popup handling | a visible popup | |

> `<C-@>` is not a typo. Godot reports Ctrl+Space as Ctrl+`@` (the terminal NUL convention), so `<C-@>` is the spelling that actually fires. `<C-Space>` parses to a different key and would never match.

**Cycle focus ships unbound.** `godotvim.focus.cycle_next` / `cycle_prev` have no default key: inside the script editor `Ctrl-W w` and `Ctrl-W W` already cycle panels through the Vim engine, and `Ctrl-W` cannot be taken as a panel binding without destroying the `Ctrl-W` sequences in the editor. To reach cycling from a dock, bind it yourself:

```vim
" Cycle panel focus with Alt+] and Alt+[ from anywhere.
panelmap <physical> <void> <norepeat> panel <M-]> godotvim.focus.cycle_next
panelmap <physical> <void> <norepeat> panel <M-[> godotvim.focus.cycle_prev
```

### Multi-key sequences

A left-hand side may be up to 8 keys long, but **only on surfaces the script editor cannot reach**:

```vim
" Legal: dock.filesystem is never live while you are editing a script.
panelunmap dock.filesystem d
panelmap <physical> dock.filesystem dd godotvim.fs.delete
```

`panel` and every `editor.*` surface reject a multi-key binding, and so does any surface that is an ancestor of an `editor.*` surface, `panel` is `editor.nav`'s parent, so `panelmap panel gd …` is live while you are typing in a script and is refused. Reserving a bare key there would break the Vim sequence that starts with it.

Binding a sequence implicitly reserves its first key on that surface: after the example above, a bare `d` in the FileSystem dock waits `timeoutlen` for the second key. `:panelmap` prints every reservation under the surface that owns it, so this is never invisible. Add `<nowait>` to a shorter rule to opt it out of the wait.

### What gets rejected

Every rejection is reported by `:panelmap` with the offending token named. The common ones:

| Line | Rejected because |
|------|------------------|
| `panelmap sidebar j godotvim.item.next` | no surface named `sidebar` is declared |
| `panelmap Dock j godotvim.item.next` | surface ids are lowercase, `^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$` |
| `panelmap dock j godotvim.item.nxt` | no action named `godotvim.item.nxt` is registered |
| `panelmap dock j filesystem_dock/rename` | not an action id and not `native` |
| `panelmap editor.insert <C-h> godotvim.focus.left` | `editor.insert` is a barrier and takes no bindings |
| `panelmap panel gd godotvim.focus.left` | `panel` is reachable from the script editor, so its bindings must be a single key |
| `panelmap panel g godotvim.focus.cycle_next` | `g` begins a Vim command sequence; binding it here would destroy the key that follows |
| `panelmap dock <S-1> godotvim.item.next` | `<S-1>` is `!` on US and `+` on German, write the literal character instead |
| `panelmap dock j godotvim.item.next count=0` | `count` is outside `1..=100` |
| `panelmap <physicl> dock j godotvim.item.next` | unknown flag, expected one of `<nowait> <physical> <void> <norepeat> <shift>` |
| `panelunmap dock j godotvim.item.next` | `panelunmap` takes exactly two operands |

A typo in the **verb** is the one failure `:panelmap` cannot report: `panelmp dock j …` is not a panel line at all, so it is never claimed, never rejected, and never listed. If a rule is missing from `:panelmap` and no rejection mentions it, check the spelling of `panelmap` / `panelunmap` first.

---

## Security

GodotVim defaults to a locked-down security posture:

- **Shell execution disabled**: `:!` commands are blocked by default. Enable in EditorSettings under `security/shell_execution`.
- **File access scoped to project**: `:w`, `:r`, `:e` restricted to `res://` and `user://` paths by default.
- **Sandboxed project vimrc:** Under the default `Sandbox` policy a project-level `.godot-vimrc` is filtered by **whitelist**, not by pattern-stripping: only known-safe constructs survive. Comments and blanks, safe `set` and `setlocal` options, `filetype` lines, `let mapleader`, the non-recursive `noremap` forms with a clean right-hand side, `panelunmap`, and `panelmap` lines targeting a registered `godotvim.*` action all pass. Everything else is stripped, including **every recursive `map` form regardless of its right-hand side** (`map`, `nmap`, `vmap`, `imap`, `omap`, `cmap`), because a recursive expansion can compose safe fragments into a dangerous one at runtime. Stripped means commented out with a `" [sandbox] stripped` comment naming the reason, never deleted, so nothing changes silently. Three policies: Disabled, Sandbox (default), Trusted.
- **Panel bindings are honoured from a project vimrc**, but only because their right-hand side is a closed vocabulary. Under the default `Sandbox` policy a committed `res://.godot-vimrc` may use `panelunmap` (it can only *remove* a binding), `native` (it can only *reduce* what the plugin consumes), and any registered `godotvim.*` action id with integer-only parameters. None of those can expand into `:!`, `:source`, or another mapping, and an unregistered action id is refused at load. A `panelmap` line that fails to parse is stripped rather than trusted, "unparseable" and "harmless" are different claims. Stripping means commenting the line out with a reason, never deleting it, so nothing changes silently. `Trusted` honours the file verbatim; `Disabled` skips it entirely. Lines in `user://.godot-vimrc`, or in a file you named yourself under Config File Path, are trusted at every tier. See [Panel Key Bindings](#panel-key-bindings-panelmap).

---

## Status Bar

A floating overlay anchored to the bottom-right of the editor:

- Mode indicator with per-mode background colors
- Command-line prompt with cursor
- Error and info messages
- Pending command display (showcmd: `d2` while waiting for motion)
- Recording indicator with pulse animation
- Pending mapping key display

Those colors are all settings; the table lives with the rest of them under
[Status Bar Colors](#status-bar-colors).

---

## Line Numbers

Four gutter modes selectable via EditorSettings:

- **Hybrid** (default), Current line shows absolute number, others show relative distance
- **Relative**: All lines show distance from cursor
- **Absolute**: Standard line numbers
- **None**: No line numbers (fold icons still shown)

---

## Custom Cursor

The cursor overlay renders above Godot's native caret using a GLSL difference-blend shader:

- **Block** cursor in Normal, Visual, Select and Command-line mode
- **Beam** cursor in Insert mode (configurable width)
- **Underline** cursor in Replace mode (configurable height)
- **Half block** while `d`, `c`, `y` or a case operator is pending; block for
  every other pending operator, with the hue rotated per operator
- Smooth exponential-decay lerp animation between positions
- Square-wave blink when stationary
- Per-mode colors configurable in EditorSettings
