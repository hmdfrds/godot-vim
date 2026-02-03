# GodotVim

<p align="center">
  <img src="icon.png" alt="GodotVim Logo" width="128" height="128" />
</p>

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
2. Search **"GodotVim"** → **Download** → **Install**
3. **Project → Project Settings → Plugins** → Enable **GodotVim**
4. **Restart the Editor** (required for full initialization)


---

## Core Vim Features

### Modal Editing
| Mode | Description |
|------|-------------|
| **Normal** | Default navigation and command mode |
| **Insert** | Text input with full editor shortcuts |
| **Visual** | Character-wise selection (`v`) |
| **Visual Line** | Line-wise selection (`V`) |
| **Visual Block** | Rectangular selection (`Ctrl-V`) |
| **Replace** | Overwrite mode (`R`) |
| **Insert Normal** | One-shot normal command (`Ctrl-O`) |

### Motions (50+)
| Category | Keys |
|----------|------|
| **Character** | `h` `j` `k` `l` |
| **Word** | `w` `b` `e` `ge` (word), `W` `B` `E` `gE` (WORD) |
| **Line** | `0` `^` `$` `g_` `\|` (go to column) |
| **Screen Line** | `gj` `gk` `g0` `g^` `g$` `gm` `gM` |
| **Document** | `gg` `G` `{count}G` `{n}%` |
| **Find** | `f{c}` `F{c}` `t{c}` `T{c}` `;` `,` |
| **Search** | `/` `?` `n` `N` `*` `#` `gn` `gN` |
| **Paragraph/Sentence** | `{` `}` `(` `)` |
| **Brackets** | `%` `[(` `])` `[{` `]}` `[[` `]]` `[]` `][` |
| **Scroll** | `Ctrl-D` `Ctrl-U` `Ctrl-F` `Ctrl-B` `Ctrl-E` `Ctrl-Y` |
| **Center** | `zz` `zt` `zb` |

### Operators
| Key | Operation |
|-----|-----------|
| `d` | Delete |
| `c` | Change (delete + insert) |
| `y` | Yank (copy) |
| `>` `<` | Indent / Unindent |
| `=` | Auto-indent |
| `gu` `gU` | Lowercase / Uppercase |
| `g~` | Toggle case |
| `J` | Join lines |

### Text Objects
| Inner | Around | Object |
|-------|--------|--------|
| `iw` | `aw` | word |
| `iW` | `aW` | WORD |
| `i"` | `a"` | double quotes |
| `i'` | `a'` | single quotes |
| `` i` `` | `` a` `` | backticks |
| `i(` `i)` `ib` | `a(` `a)` `ab` | parentheses |
| `i{` `i}` `iB` | `a{` `a}` `aB` | braces |
| `i[` `i]` | `a[` `a]` | brackets |

### Registers & Macros
- **Named registers**: `"a` - `"z` (append with `"A` - `"Z`)
- **Numbered registers**: `"0` - `"9` (yank/delete history)
- **Special registers**: `""` (default), `"_` (black hole), `"+` `"*` (system clipboard)
- **Macro recording**: `q{a-z}` to record, `q` to stop, `@{a-z}` to play, `@@` to repeat

### Marks & Jumps
- **Local marks**: `'a` - `'z` (line), `` `a `` - `` `z `` (exact position)
- **Global marks**: `'A` - `'Z` (cross-file, preserved)
- **Jump list**: `Ctrl-O` (older) / `Ctrl-I` (newer)
- **Last visual**: `gv` (reselect last visual selection)

### Ex Commands
| Command | Description |
|---------|-------------|
| `:w` `:save` | Save current file (syncs with Godot) |
| `:q` `:close` | Close current tab |
| `:bn` `:bp` | Next/previous buffer |
| `:b{n}` | Go to buffer N |
| `:{n}` `:go` | Go to line / byte |
| `:%s/old/new/g` | Substitution (with regex) |
| `:{range}y` `:{range}d` | Yank / delete range |
| `:reg` | List registers |

### Other Features
- **Dot repeat** (`.`) - Repeat last change
- **Undo/Redo** (`u` / `Ctrl-R`)
- **Number increment/decrement** (`Ctrl-A` / `Ctrl-X`)
- **Code folding** (`zo` `zc` `za` `zM` `zR`)


---

## Godot Integration

### Dock Navigation
Switch between Godot docks seamlessly with keyboard shortcuts.

| Mapping | Action |
|---------|--------|
| `Ctrl-H/J/K/L` | Navigate between docks |
| `<Space>e` | FileSystem dock |
| `<Space>o` | Scene dock |
| `<Space>i` | Inspector dock |
| `<Space>s` | Script editor |
| `` <Space>` `` | Output panel |
| `<Space>2` `<Space>3` | 2D / 3D editor |

![Dock Navigation](media/dock_nav.gif)

### Scene Control
| Mapping | Command | Action |
|---------|---------|--------|
| `<Space>r` | `:run` | Run main scene (F5) |
| `<Space>R` | `:runcurrent` | Run current scene (F6) |
| `<Space>S` | `:stop` | Stop running scene |

### Debugging
| Mapping | Command | Action |
|---------|---------|--------|
| `<Space>db` | `:GodotBreakpoint` | Toggle breakpoint |
| `<Space>dc` | `:GodotContinue` | Continue execution |
| `<Space>dn` | `:GodotNext` | Step over |
| `<Space>di` | `:GodotStepIn` | Step into |
| `<Space>do` | `:GodotStepOut` | Step out |
| `<Space>dp` | `:GodotPause` | Pause execution |

![Debug with mappings](media/debug.gif)

### Code Intelligence
| Key | Action |
|-----|--------|
| `gd` | Go to definition |
| `K` | Show documentation |

![Native Documentation](media/docs.gif)

### Editor State
| Mapping | Command | Action |
|---------|---------|--------|
| `<Space>z` | `:zen` | Enable distraction-free mode |
| `<Space>Z` | `:unzen` | Disable distraction-free mode |
| — | `:restart` | Restart editor |


---

## Configuration

Access settings via **Editor → Editor Settings → Plugins → GodotVim**:

| Section | Settings |
|---------|----------|
| **General** | Enable/disable plugin, log level |
| **Editor** | Scroll offset, line numbers (absolute/relative/hybrid), iskeyword, key passthrough |
| **Cursor** | Colors for Normal, Insert, Visual modes |
| **Clipboard** | Auto-copy to system clipboard |
| **Mapping** | Enable/disable mappings, timeout length |

### Mappings Panel
Customize all mappings in the dedicated Mappings dock (right of History tab). Recommended mappings are pre-configured. 

**Insert Mode Escapes** (disabled by default):
- `jj`, `jk`, `kj` → `<Esc>`

**Buffer Navigation** (enabled by default):
- `<Space>n` / `<Space>p` → Next / Previous buffer
- `<Space>1` - `<Space>9` → Jump to buffer 1-9

> [!NOTE]
> Mappings using `:Command` syntax don't require `<CR>`.  
> Example: `<Space>e` → `:FileSystem` (not `:FileSystem<CR>`)


---


## Known Limitations

- **Floating Script Editor**: Dock navigation may not work correctly when the Script Editor is detached/floating.


---

MIT License