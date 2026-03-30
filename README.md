<p align="center">
  <img src="media/icon.png" alt="GodotVim Logo" width="128" height="128" />
</p>

<h1 align="center">GodotVim</h1>

<p align="center">
  <b>Vim emulation for Godot's built-in script editor.</b>
</p>

<p align="center">
  <a href="https://godotengine.org/asset-library/asset/4666">
    <img src="https://img.shields.io/badge/Godot%20Asset%20Lib-4.5%2B-478cbf?logo=godot-engine&logoColor=white" alt="Godot Asset Library">
  </a>
  <a href="https://github.com/hmdfrds/godot-vim/actions/workflows/scan.yml">
    <img src="https://github.com/hmdfrds/godot-vim/actions/workflows/scan.yml/badge.svg" alt="VirusTotal Scan">
  </a>
  <img src="https://img.shields.io/github/license/hmdfrds/godot-vim" alt="License">
</p>

---

## Installation

### Godot Asset Library (Recommended)
1. Open your Godot project → **AssetLib** tab
2. Search **"[GodotVim](https://godotengine.org/asset-library/asset/4666)"** → **Download** → **Install**
3. **Project → Project Settings**  → **Plugins** → Enable **GodotVim**
4. **Restart the Editor** (required for full initialization)

### Upgrading from v0.x

This is a complete rewrite — settings, config format, and internals are all new.

1. **Remove the old `addons/godot_vim/` folder** from your Godot project before installing
2. **Clear old EditorSettings** (optional): old GodotVim keys in `editor_settings-4.tres` are harmless — the new version ignores them — but you can delete lines starting with `plugins/GodotVim` in that file for a clean slate. The file is located at:
   - **Windows:** `%APPDATA%\Godot\editor_settings-4.tres`
   - **Linux:** `~/.config/godot/editor_settings-4.tres`
   - **macOS:** `~/Library/Application Support/Godot/editor_settings-4.tres`
3. **Recreate key mappings**: v0.x stored mappings in EditorSettings; v1.0 uses a `.godot-vimrc` config file instead (see [Configuration](#configuration))

## Quick Start

Open any script.

| Keys | What happens |
|------|-------------|
| `i` | Enter Insert mode (type normally) |
| `Escape` | Return to Normal mode |
| `dd` | Delete the current line |
| `ci"` | Change text inside quotes |
| `/pattern` | Search forward |
| `:w` | Save the file |
| `:run` | Run the project (F5) |
| `Ctrl+h/j/k/l` | Navigate between Godot panels |

## Features

### Full Composable Vim Grammar

`d2w`, `ci"`, `gUiw`, `>ap` — operators compose with motions and text objects exactly like real Vim. Counts multiply, registers route output. Dot repeat (`.`) replays any edit faithfully, including complex multi-key sequences.

```
[count] [register] operator [count] motion/textobject
```

Motions, operators, text objects — including advanced text objects like `ib` (any bracket), `iq` (any quote), `ii` (indent), `im` (symbol), and `ie` (entire buffer). Visual block mode supports `I`/`A` for multi-line insert/append. Undo tree with time-based navigation. [Full list →](docs/REFERENCE.md#motions)

### Built for Godot

Not just Vim in an editor — Vim that speaks Godot:

- **`:run`** / **`:runcurrent`** — launch scenes without leaving the keyboard
- **`:GodotBreakpoint`** — toggle breakpoints, step through with `:next` / `:stepin`
- **`Ctrl+h/j/k/l`** — spatial panel navigation (script editor, scene tree, inspector, filesystem)
- **`j/k/h/l` in docks** — browse the scene tree, filesystem, and output with Vim keys; `/` to search
- **`gd`** — go-to-definition; **`K`** — hover docs
- **Code completion** — `Ctrl-N`/`Ctrl-P`/`Ctrl-Space` trigger and navigate completions
- **Cross-buffer jump list** — `Ctrl-O`/`Ctrl-I` switch tabs when the jump is in another buffer
- **`:zen`** — distraction-free mode

[All Godot commands →](docs/REFERENCE.md#custom-commands)

### Search and Replace

Incremental search highlighting as you type. `:s/old/new/g` highlights every match region in yellow as you type the pattern — see exactly what will be affected before you press Enter.

### Custom Cursor

A GLSL difference-blend shader renders the cursor above Godot's native caret. Block, beam, and underline shapes with smooth animation, per-mode colors, and adjustable blink speed. [Cursor settings →](docs/REFERENCE.md#cursor)

### Configuration

Create a `.godot-vimrc` at your project root or user directory:

```vim
let mapleader = " "
set timeoutlen=500

nnoremap <Leader>w :save<CR>
inoremap jk <Esc>
vnoremap < <gv
vnoremap > >gv
```


Hot-reloads on save. 20 built-in presets togglable via the `:mappings` dialog. [Config syntax →](docs/REFERENCE.md#godot-vimrc-syntax) · [All presets →](docs/REFERENCE.md#preset-mappings)

### Macros, Registers, and Marks

Record with `qa`, replay with `@a`. Named registers `"a`-`"z`, system clipboard via `"+`. Local and global marks, jump list with `Ctrl-O`/`Ctrl-I`. [Full details →](docs/REFERENCE.md#registers-and-macros)

## Troubleshooting

| Problem | Solution |
|---------|----------|
| Plugin not appearing | Ensure `addons/godot_vim/` contains `plugin.cfg`, `.gdextension`, and the compiled library. Enable in Project Settings > Plugins. |
| Key not working | Check `passthrough_keys` setting — the key may be bypassing Vim. Check `:mappings` for conflicts. |
| `.godot-vimrc` not loading | Verify the file is at `res://.godot-vimrc` or `user://.godot-vimrc`. Run `:source` to force reload. |
| Clipboard not working | Enable `editor/clipboard_enabled` in EditorSettings, or use `"+y` / `"+p`. |
| Cursor not rendering | The custom cursor uses a GLSL shader. Set `cursor/enabled = false` to fall back to native caret. |

**For bug reports:** Set **Log Level** to `Debug` in Editor Settings > GodotVim, reproduce the issue, then copy the Output panel into GitHub issue. The debug log shows every keystroke and what command was executed.

## Architecture

```
+----------------+     +----------+     +----------+
| Godot CodeEdit | <-> |  Bridge  | <-> | vim-core |
|                |     |  (gdext) |     | (Rust)   |
+----------------+     +----------+     +----------+
```

**vim-core** is a standalone Vim engine — pure Rust, zero Godot dependencies. It processes keystrokes and returns `Effect` values that the host applies. The same engine could power any editor.

[All settings](docs/REFERENCE.md#settings) · [All commands](docs/REFERENCE.md#custom-commands) · [Full reference](docs/REFERENCE.md)

## License

[MIT](LICENSE)
