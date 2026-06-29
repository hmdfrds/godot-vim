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
- [Settings](#settings)
- [Custom Commands](#custom-commands)
- [Preset Mappings](#preset-mappings)
- [.godot-vimrc Syntax](#godot-vimrc-syntax)
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
| Command-line | `:`, `/`, `?` | — |
| Select | `gh` | Block (orange) |
| Operator-pending | `d`, `c`, `y`, `>`, etc. | Block (orange) |

---

## Motions

The full composable Vim grammar — operators compose with motions and text objects, counts multiply, registers route output:

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
| `ib` | `ab` | any bracket (nearest `()`, `[]`, `{}`) |
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

- **Incremental search** — results highlight in real-time as you type `/pattern` or `?pattern`.
- **Live substitute match highlighting** — `:s/old/new/g` highlights match regions in yellow as you type the pattern, showing exactly what will be affected before Enter (`inccommand` setting). Note: highlights the match locations, not the replacement text.
- **Regex support** — Vim-compatible regex with all four magic modes.
- **Search commands** — `*`, `#`, `gn`, `gN`, `n`, `N`, `hlsearch`, `:noh`.

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
| `Ctrl-@` | Insert last inserted text and exit Insert mode |
| `Ctrl-T` | Increase indent of current line |
| `Ctrl-D` | Decrease indent of current line |
| `Ctrl-E` | Insert character from line below |
| `Ctrl-Y` | Insert character from line above |
| `Ctrl-V {char}` | Insert literal character / unicode codepoint |
| `Ctrl-G u` | Break undo sequence |
| `Ctrl-G U` | Don't break undo on next cursor movement |
| `Ctrl-N` | Next completion item |
| `Ctrl-P` | Previous completion item |
| `Ctrl-Space` | Trigger completion menu |

Auto-pair insertion for `()`, `[]`, `{}`, `""`, `''`, `` `` `` is handled by Godot's CodeEdit; GodotVim preserves this behavior in Insert mode.

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

### Undo

| Command | Description |
|---------|-------------|
| `:earlier {N}` / `:earlier {time}` | Travel backward in undo tree (count, time, or save-based) |
| `:later {N}` / `:later {time}` | Travel forward in undo tree |
| `:undolist` | Show undo history |

### Buffers and Tabs

| Command | Description |
|---------|-------------|
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
| `:setlocal {option}[={value}]` | Set option locally |
| `:echo {expr}` | Echo expression |
| `:!{cmd}` | Execute shell command (when enabled) |
| `:actionlist [filter]` | List available Godot editor actions |

---

## Undo Tree

Undo, redo, and time-based navigation.

| Key / Command | Action |
|---------------|--------|
| `u` | Undo |
| `Ctrl-R` | Redo |
| `:earlier {N}` | Go back N changes |
| `:earlier {N}s` / `{N}m` / `{N}h` | Go back by time (seconds, minutes, hours) |
| `:earlier {N}f` | Go back N file saves |
| `:later {N}` | Go forward N changes |
| `:later {N}s` / `{N}m` / `{N}h` | Go forward by time |
| `:later {N}f` | Go forward N file saves |
| `:undolist` | Display undo history |

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

The following Vim options are supported via `:set`, `:setlocal`, and `.godot-vimrc`:

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
| `textwidth` / `tw` | `int` | `80` | Maximum line width for formatting |
| `timeoutlen` / `tm` | `int` | `1000` | Mapping timeout in milliseconds |
| `number` / `nu` | `bool` | `true` | Show line numbers |
| `relativenumber` / `rnu` | `bool` | `true` | Show relative line numbers |
| `inccommand` / `icm` | `string` | `nosplit` | Highlight substitute match regions as you type |
| `clipboard` | `string` | `""` | Clipboard integration |
| `iskeyword` / `isk` | `string` | (default) | Characters considered part of a word |
| `whichwrap` / `ww` | `string` | `""` | Keys that wrap across lines |
| `virtualedit` / `ve` | `string` | `""` | Allow cursor beyond end of line |
| `selection` / `sel` | `string` | `inclusive` | Visual selection behavior |

---

## Settings

All settings are in **Editor > Editor Settings > Plugins > GodotVim**.

> **Note:** Indent settings (tab size, spaces vs tabs) are read from Godot's CodeEdit, not GodotVim. Configure them in **Editor > Editor Settings > Text Editor > Behavior > Indent**.

### General

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| Enabled | `bool` | `true` | **Master switch** — global, per-user editor setting (stored in `editor_settings-*.tres`, not `project.godot`). When `false` the plugin is inert: no keybindings, overlays, input handling, signal connections, filesystem-prompt interception, or `.godot-vimrc` sourcing occur. What remains connected while inert: the settings listener (so re-enable is observed), an idle one-shot mapping timer whose connection persists but never fires while input is off, filesystem Callables (plain data), the process-global panic hook, and the always-loaded native extension. To disable only one project, turn off the plugin in that project's **Project Settings → Plugins** (writes `project.godot`). |
| Log Level | `enum` | `Off` | `Off`, `Error`, `Warn`, `Info`, `Debug`, `Trace`. |

### Editor Behavior

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| Scroll Off | `int` | `5` | Minimum lines above/below cursor (0-20). |
| Text Width | `int` | `80` | Max line width for `gq` formatting. |
| Clipboard | `bool` | `false` | Sync Vim registers with system clipboard. |
| Ignore Case | `bool` | `false` | Case-insensitive search. |
| Smart Case | `bool` | `false` | Uppercase in pattern overrides Ignore Case. |
| Line Numbers | `enum` | `Hybrid` | `None`, `Absolute`, `Relative`, `Hybrid`. |
| Inccommand | `enum` | `nosplit` | Live `:s` preview. `nosplit` = enabled, `off` = disabled. |
| ~~Highlight Yank~~ | ~~`int`~~ | ~~`150`~~ | ~~Yank highlight duration in ms (0 = disabled).~~ |

### Cursor

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| Enabled | `bool` | `true` | Custom cursor overlay (disable for native caret). **NOT the master toggle — see General → Enabled.** |
| Lerp Speed | `float` | `25.0` | Smooth movement speed (higher = snappier). |
| Underline Height | `float` | `4.0` | Replace-mode underline height in pixels. |
| Normal Color | `Color` | `#FFFFFF` | Cursor color in Normal mode. |
| Insert Color | `Color` | `#55FF7F` | Cursor color in Insert mode. |
| Visual Color | `Color` | `#FFB855` | Cursor color in Visual mode. |
| Replace Color | `Color` | `#FF333399` | Cursor color in Replace mode. |
| Operator Mode Color | `Color` | `#FFB855` | Cursor color in Operator-pending mode. |
| Command Mode Color | `Color` | `#FFFFFF` | Cursor color in Command-line mode. |

> **Note:** Line highlighting, cursor blink, and beam width are controlled by Godot's native settings under `text_editor/appearance/caret/`.

### Key Mapping

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| Timeout Length | `int` | `1000` | Timeout for ambiguous mappings in ms. |
| Config File Path | `string` | `""` | Path to `.godot-vimrc`. Empty = auto-resolve. |
| Passthrough Keys | `string` | `""` | Comma-separated keys bypassing Vim (e.g. `<C-v>,<C-a>`). |

### Security

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| Shell Execution | `enum` | `Disabled` | Allow `:!` commands. |
| File Access Scope | `enum` | `Project Only` | Restrict file ops to `res://` and `user://`. |
| Project Vimrc | `enum` | `Sandbox` | `Disabled`, `Sandbox`, `Trusted`. |

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
| `:GodotStepOut` | `:stepout` | Step out |
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
| `:perf` | | Show keystroke performance stats |
| `:vimdebug` | | Toggle debug annotations |

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
| `<Space>do` | Step out | off |
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
| `nmap` / `nnoremap` | Normal mode mapping |
| `imap` / `inoremap` | Insert mode mapping |
| `vmap` / `vnoremap` | Visual mode mapping |
| `omap` / `onoremap` | Operator-pending mode mapping |
| `cmap` / `cnoremap` | Command-line mode mapping |
| `map` / `noremap` | Normal + Visual + Operator-pending |

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
" In .godot-vimrc — map Leader+s to save
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

## Security

GodotVim defaults to a locked-down security posture:

- **Shell execution disabled** — `:!` commands are blocked by default. Enable in EditorSettings under `security/shell_execution`.
- **File access scoped to project** — `:w`, `:r`, `:e` restricted to `res://` and `user://` paths by default.
- **Sandboxed project vimrc** — Project-level `.godot-vimrc` files have shell-invoking patterns stripped automatically. Three policies: Disabled, Sandbox (default), Trusted.

---

## Status Bar

A floating overlay anchored to the bottom-right of the editor:

- Mode indicator with per-mode background colors
- Command-line prompt with cursor
- Error and info messages
- Pending command display (showcmd: `d2` while waiting for motion)
- Recording indicator with pulse animation
- Pending mapping key display

### Status Bar Colors

All configurable in **Editor > Editor Settings > Plugins > GodotVim > Status Bar**:

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| Normal BG | `Color` | `rgb(0.5, 0.6, 0.8)` | Background in Normal mode. |
| Insert BG | `Color` | `rgb(0.6, 0.8, 0.5)` | Background in Insert mode. |
| Visual BG | `Color` | `rgb(0.8, 0.5, 0.5)` | Background in Visual mode. |
| Replace BG | `Color` | `rgb(0.9, 0.6, 0.3)` | Background in Replace mode. |
| Command BG | `Color` | `rgb(0.157, 0.173, 0.204)` | Background in Command-line mode. |
| Recording BG | `Color` | `rgb(0.9, 0.2, 0.2)` | Background while recording a macro. |
| Text FG | `Color` | `#FFFFFF` | Foreground text color. |
| Error FG | `Color` | `rgb(1.0, 0.3, 0.3)` | Foreground color for error messages. |

---

## Line Numbers

Four gutter modes selectable via EditorSettings:

- **Hybrid** (default) — Current line shows absolute number, others show relative distance
- **Relative** — All lines show distance from cursor
- **Absolute** — Standard line numbers
- **None** — No line numbers (fold icons still shown)

---

## Custom Cursor

The cursor overlay renders above Godot's native caret using a GLSL difference-blend shader:

- **Block** cursor in Normal/Visual/Operator-pending mode
- **Beam** cursor in Insert mode (configurable width)
- **Underline** cursor in Replace mode (configurable height)
- Smooth exponential-decay lerp animation between positions
- Square-wave blink when stationary
- Per-mode colors configurable in EditorSettings
