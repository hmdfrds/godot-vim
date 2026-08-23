# Upgrading

Notes for people arriving from an older release. Nothing here applies to a
fresh install.

## From v1.7.x

The completion popup keys now resolve through the same pipeline as every
other panel binding, and three long-standing defects went with the old
transport. Four notes:

1. **Tab and Enter no longer accept a candidate you did not choose.** Godot
   preselects the first row the moment a popup opens; pressing Enter to break
   a line used to accept it. They now confirm only a selection you explicitly
   moved onto (`Ctrl-N`/`Ctrl-P`, arrows, a click); otherwise the popup
   closes and the key does its ordinary job. Restore the old behaviour with:

   ```vim
   panelmap editor.completion <CR> godotvim.completion.confirm require_selection=0
   panelmap editor.completion <Tab> godotvim.completion.confirm require_selection=0
   ```

2. **Ctrl-Space, Ctrl-N and Ctrl-P now open the popup.** They never did: the
   old gate read a `CodeEdit` flag Godot's script editor never sets, so
   Ctrl-Space fell through to Vim's `i_CTRL-@`, which pastes your previous
   insert and exits Insert mode. If you relied on that, restore it with
   `panelunmap editor.completion <C-@>`.

3. **`<C-y>` and `<C-e>` are claimed while a popup is visible**: explicit
   accept and close-keeping-text, Vim's own popup keys. With no popup up they
   still reach vim-core's copy-character-above / copy-character-below
   untouched.

4. **Completion bindings now honour `:set langmap`, `<void>`, `<norepeat>`
   and `key=value` parameters**, all of which parsed and did nothing before.
   A project-level `.godot-vimrc` under the `Sandbox` policy can no longer
   bind keys on an `editor.*` surface (a committed vimrc could otherwise
   consume Escape inside Insert mode); your user-level vimrc is unaffected.

One residual: inside a `<C-x>` completion submode, Escape does not fire a
mode change in the engine, so the popup closes on the second of the two
presses you were already making.

The shipped `<Esc>` row on `editor.completion` is gone, with no behaviour
change: one press still closes the popup and leaves Insert, through the
engine. If you want the two-stage Escape (first press closes the popup and
stays in Insert, second press leaves), it is now one line:

```vim
panelmap editor.completion <Esc> godotvim.completion.dismiss
```

## From v0.x

v1.0 was a complete rewrite. Settings, the config format and the internals are
all different.

1. Remove the old `addons/godot_vim/` folder from your project before
   installing the new one.
2. Old GodotVim keys in your editor settings are harmless and ignored, but you
   can delete every line starting with `plugins/GodotVim` for a clean slate.
   The file is:
   - Windows: `%APPDATA%\Godot\editor_settings-4.tres`
   - Linux: `~/.config/godot/editor_settings-4.tres`
   - macOS: `~/Library/Application Support/Godot/editor_settings-4.tres`
3. Recreate your key mappings. v0.x stored them in editor settings; v1.x reads
   them from a `.godot-vimrc` file in your project. `:mkvimrc` writes a
   starter with every preset listed and commented out.

## From v1.6.x

With no `.godot-vimrc`, the keyset is unchanged. The keys outside the text
buffer (panel focus, dock navigation, FileSystem and debugger operations, the
completion popup) became a rebindable table in v1.7.0; the shipped defaults are
what they were. See [Panel key bindings](REFERENCE.md#panel-key-bindings-panelmap).

One difference you may notice: `Ctrl+Enter` no longer confirms a completion.
The old popup handler matched `Enter`, `Tab`, `Escape`, `Up` and `Down` while
ignoring modifiers, so every modified variant was swallowed too. `<CR>` now
means `<CR>`, and modified variants reach the Vim engine. To restore the old
`Shift+Up` behaviour:

```vim
panelmap <shift> editor.completion <Up> godotvim.completion.navigate
```
