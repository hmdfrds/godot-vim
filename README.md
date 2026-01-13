# GodotVim

**Vim keybindings for Godot's built-in script editor.**

---

## Installation

### Godot Asset Library (Recommended)
1. Open Godot Editor → **AssetLib** tab
2. Search for **"GodotVim"**
3. Click **Download** → **Install**
4. Enable in **Project → Project Settings → Plugins**
5. **Restart the editor** for the plugin to load

### Manual Installation
1. Download the [latest release](https://github.com/hmdfrds/godot-vim/releases)
2. Extract `addons/godot_vim` into your project's `addons/` folder
3. Enable in **Project → Project Settings → Plugins**
4. **Restart the editor** for the plugin to load

---

## Modes

| Mode | Enter | Description |
|------|-------|-------------|
| **Normal** | `Esc` | Navigation and commands |
| **Insert** | `i`, `a`, `o`, `O`, `I`, `A` | Text editing |
| **Visual** | `v` | Character selection |
| **Visual Line** | `V` | Line selection |
| **Visual Block** | `Ctrl+v` | Rectangular selection |
| **Replace** | `R` | Overwrite text |
| **Command** | `:` | Ex commands |

---

## Motions

### Basic Movement
`h` `j` `k` `l` — Left, Down, Up, Right

### Word Movement
| Motion | Description |
|--------|-------------|
| `w` / `W` | Next word / WORD |
| `b` / `B` | Previous word / WORD |
| `e` / `E` | End of word / WORD |
| `ge` | End of previous word |

### Line Movement
| Motion | Description |
|--------|-------------|
| `0` | Start of line |
| `^` | First non-blank |
| `$` | End of line |

### Document Movement
| Motion | Description |
|--------|-------------|
| `gg` | First line |
| `G` | Last line |
| `{count}G` | Go to line |
| `{` / `}` | Paragraph up/down |
| `%` | Matching bracket |

### Character Find
`f{char}` `F{char}` `t{char}` `T{char}` — Find forward/backward, to/till

### Scrolling
| Key | Description |
|-----|-------------|
| `Ctrl+d` / `Ctrl+u` | Half page down/up |
| `Ctrl+f` / `Ctrl+b` | Full page down/up |
| `zz` / `zt` / `zb` | Center/Top/Bottom cursor |

---

## Operators

Operators combine with motions: `{operator}{motion}`

| Operator | Description |
|----------|-------------|
| `d` | Delete |
| `c` | Change (delete + insert) |
| `y` | Yank (copy) |
| `>` / `<` | Indent / Outdent |
| `gu` / `gU` | Lowercase / Uppercase |
| `gq` | Format |
| `J` | Join lines |

**Examples:** `dw` (delete word), `ci"` (change inside quotes), `>}` (indent paragraph)

---

## Text Objects

Use with operators: `{operator}{a/i}{object}`

| Object | Inner (`i`) | Around (`a`) |
|--------|-------------|--------------|
| `w` | Word | Word + space |
| `"` `'` `` ` `` | Inside quotes | Include quotes |
| `(` `)` | Inside parens | Include parens |
| `{` `}` | Inside braces | Include braces |
| `[` `]` | Inside brackets | Include brackets |
| `<` `>` | Inside angles | Include angles |

**Examples:** `ciw` (change word), `da"` (delete around quotes), `yi(` (yank inside parens)

---

## Registers & Clipboard

| Key | Description |
|-----|-------------|
| `"{reg}` | Use register for next operation |
| `"*` `"+` | System clipboard |
| `""` | Default register |

Enable auto-clipboard in settings to sync yank/delete with system clipboard.

---

## Macros

| Key | Description |
|-----|-------------|
| `q{a-z}` | Start recording to register |
| `q` | Stop recording |
| `@{a-z}` | Play macro |
| `@@` | Repeat last macro |

---

## Marks

| Key | Description |
|-----|-------------|
| `m{a-z}` | Set local mark |
| `'{a-z}` | Jump to mark (line) |
| `` `{a-z} `` | Jump to mark (exact position) |

---

## Search

| Key | Description |
|-----|-------------|
| `/pattern` | Search forward |
| `?pattern` | Search backward |
| `n` / `N` | Next / Previous match |
| `*` / `#` | Search word under cursor |

---

## Ex Commands

| Command | Description |
|---------|-------------|
| `:w` | Save |
| `:q` | Close tab |
| `:wq` | Save and close |
| `:e {file}` | Open file |
| `:{range}s/old/new/g` | Substitute |

---

## Configuration

All settings are in **Project → Project Settings → GodotVim**:

### General
- **Enabled** — Toggle Vim mode
- **Log Level** — Error/Warn/Info/Debug

### Cursor Colors
- Per-mode cursor colors (Normal, Insert, Visual)
- Toggle mode-based coloring

### Behavior
- **Scroll Offset** — Lines to keep visible above/below cursor
- **Highlight Current Line** — Enable line highlighting

### Clipboard
- **Yank to Clipboard** — Auto-copy yanks to system clipboard
- **Delete to Clipboard** — Auto-copy deletes to system clipboard

### Key Mappings
Custom mappings per mode:
- `imap` — Insert mode (e.g., `jj` → `<Esc>`)
- `nmap` — Normal mode
- `vmap` — Visual mode
- `cmap` — Command mode

**Key Passthrough** — Keys that bypass Vim (e.g., `Ctrl+S` for save)

---

## Requirements

- **Godot 4.5+**
- **Platforms:** Linux, Windows, macOS (Intel + Apple Silicon)

---

## License

MIT License — see [LICENSE](LICENSE)
