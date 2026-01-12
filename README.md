# GodotVim

Vim emulation for Godot's built-in script editor.

## Installation

### From Godot Asset Library (Recommended)
1. Open Godot Editor
2. Go to **AssetLib** tab
3. Search for "GodotVim"
4. Click Download and Install

### Manual Installation
1. Download the latest release from [Releases](https://github.com/hmdfrds/godot-vim/releases)
2. Extract the `addons/godot_vim` folder into your project's `addons/` directory
3. Enable the plugin in **Project → Project Settings → Plugins**

## Features

- **Normal Mode**: Navigate and edit with Vim motions (`h`, `j`, `k`, `l`, `w`, `b`, `e`, etc.)
- **Insert Mode**: Standard text editing with `i`, `a`, `o`, `O`, etc.
- **Visual Mode**: Select text with `v`, `V` (linewise), `Ctrl+v` (block)
- **Command Mode**: Execute commands with `:` (`:w`, `:q`, `:s`, etc.)
- **Operators**: `d` (delete), `c` (change), `y` (yank), `>` / `<` (indent)
- **Text Objects**: `iw`, `aw`, `i"`, `a"`, `i(`, `a(`, etc.
- **Macros**: Record with `q{register}`, replay with `@{register}`
- **Marks**: Set with `m{letter}`, jump with `'{letter}` or `` `{letter} ``
- **Search**: `/pattern`, `n`, `N`, `*`, `#`
- **Custom Mappings**: Configure in Project Settings

## Configuration

Settings are available in **Project → Project Settings → GodotVim**:

- **Enabled**: Toggle Vim mode on/off
- **Cursor Colors**: Customize cursor color per mode
- **Key Mappings**: Add custom key mappings
- **Key Passthrough List**: Keys that bypass Vim and go to Godot

## Supported Godot Versions

- Godot 4.2+

## License

MIT License - see [LICENSE](LICENSE) for details.
