# Upgrading

Notes for people arriving from an older release. Nothing here applies to a
fresh install.

## From v1.8.1

**GDScript wraps only comments, never code.** GodotVim now detects each
script's filetype and runs Vim's filetype plugins for it, transcribed from
Vim 9.1 (see [Filetype Plugins](REFERENCE.md#filetype-plugins)). With a
Textwidth above `0`:

- GDScript and shader code no longer breaks while you type. `#`, `##` and
  `//` comment lines break at the last blank before the width and continue
  with the same indent and leader. This deliberately differs from Vim's
  `gdscript.vim`, which lets code wrap.
- JSON never wraps. Markdown and text wrap as prose, as in Vim.
- The break no longer misplaces or deletes the characters you type, and it
  happens where Vim breaks: when you type a non-blank character past the
  width, at the last blank before the cursor.

The default Textwidth stays `0`, so nothing wraps unless you set a width.

**`formatoptions` and `comments` can be set.** `:set fo-=t`,
`:setlocal fo+=t` and `:set comments^=b:##` work as in Vim, including in a
`.godot-vimrc`. `:setlocal` now changes typing as well as `gq`, and a
change to an Editor Setting wins over an earlier `:set` everywhere, so the
caveat about local values in the v1.8.0 notes below no longer applies.

**`commentstring` is per script.** It used to be copied from the script
editor's comment delimiters into the global value on every tab switch, so a
shader's `//` could reach the next text file and a `set commentstring` in
your vimrc was overwritten. Each script now gets its own value once. A
`:setlocal commentstring` you make stays with that script.

**To keep the old behaviour** (Vim's defaults in every script, so code wraps
with a width set), turn off **Godot Vim > Editor > Filetype Plugin**, or put
`filetype plugin off` in your `.godot-vimrc`. The commentstring still comes
from the script editor either way.

## From v1.8.0

**Textwidth now defaults to `0`, as in Vim.** With the old default of 80,
typing in Insert mode on a line longer than 80 characters broke the line,
because vim-core's `formatoptions` contains `t`. In GDScript that splits a
statement, and the break could also misplace the characters typed next.

1. **If you never changed Textwidth**, it moves to `0` the first time this
   version loads: when you restart the editor, or when you disable and enable
   the plugin again under Project Settings > Plugins. Nothing breaks lines
   while you type, and `gq` formats at 79 columns.
2. **If you set another width**, such as 100, it is kept, and typing keeps
   breaking lines longer than that. Set it to `0` to stop that.
3. **If you deliberately wanted exactly 80**, set it again under
   **Godot Vim > Editor > Textwidth**. Godot saved the old default of 80
   for everyone, so an 80 stored by an earlier version cannot be told apart
   from a setting nobody touched, and it moves to `0` once. An 80 you set
   from now on is kept.

**`:set` values now survive unrelated Editor Settings changes.** Every
change to any Editor Setting used to push all of GodotVim's settings into
the engine again, and copy tab size, indent size and spaces-or-tabs from the
script editor, which undid a `:set tw=0`, `:set ignorecase` or `:set ts=8`
from your vimrc or the command line. Now a setting is pushed only when it
changes, and the indent is copied only when the script editor's own indent
changes.

Such a change sets the global value, which typing reads. For `textwidth`,
`tabstop`, `shiftwidth` and `expandtab`, a `:set` also sets a value local to
the current script, and commands that read the local value, such as `gq`
and `>>`, keep using the `:set` value in that script even after the Editor
Setting or Godot's indent changes. Switching to another script still takes
that script's indentation.

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
