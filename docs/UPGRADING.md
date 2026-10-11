# Upgrading

Notes for people arriving from an older release. Nothing here applies to a
fresh install.

## From v1.8.2

**The completion popup follows Godot again.** Six notes:

1. **Enter and Tab accept the highlighted row**, as they do with the plugin
   off. v1.8 accepted only a row you had moved onto. Which keys accept is now
   whatever **Editor Settings > Shortcuts** says for Completion Accept
   (`ui_text_completion_accept`) and Completion Replace
   (`ui_text_completion_replace`), read on every key press. On Godot 4.6.2
   and later plain Enter and Tab accept and their Shift forms replace; on 4.4
   to 4.6.1 it is the other way round, and GodotVim follows either. Keypad
   Enter, Shift+Enter, Shift+Tab, Page Up and Page Down now do what Godot does
   with the popup open, and so does any key you add there.

2. **The popup opens after Code Complete Delay** (Editor Settings > Text
   Editor > Completion), not on the first letter you type, and Esc inside the
   delay opens nothing. With Code Complete Enabled off it still never opens
   while you type; `Ctrl-Space`, `Ctrl-N` and `Ctrl-P` still open it at once.
   Backspace no longer opens it, as in Godot, where only typing starts the
   delay.

3. **Delete the lines v1.8 asked you to add.** `require_selection=` is
   ignored now, and a `<CR>` or `<Tab>` row on `editor.completion` still wins
   over your Shortcuts, so on Godot 4.4 to 4.6.1 it would accept where Godot
   replaces:

   ```vim
   panelmap editor.completion <CR> godotvim.completion.confirm require_selection=0
   panelmap editor.completion <Tab> godotvim.completion.confirm require_selection=0
   ```

4. **To keep Enter a newline and accept with Tab only** (what keeping just
   the `<Tab>` line above gave you), delete that line too and, in Editor
   Settings > Shortcuts, remove Enter and keypad Enter from Completion Accept
   (on Godot 4.4 to 4.6.1, from Completion Replace). It is Godot's own
   setting, so it also holds with the plugin off.
   `panelunmap editor.completion <Tab>` no longer frees Tab while the popup is
   open; the Shortcuts do. Enter and Tab act on an open popup first, and
   Ctrl+Space always does, so an Insert mapping of one of them does not fire
   then; to let it fire, remove the key from Completion Accept, Completion
   Replace or Completion Query.

5. **Ctrl+Space is Godot's Completion Query** in Insert, Replace and Virtual
   Replace. `panelunmap editor.completion <C-@>` no longer brings back Vim's
   `i_CTRL-@`, which pastes your previous insert and leaves Insert: the row it
   removed is gone, and the line does nothing. Remove Ctrl+Space from
   Completion Query in Shortcuts instead, and the key reaches Vim.

6. **A mapping like `inoremap jk <Esc>` no longer corrupts an accept.** A popup
   key typed while the `j` waits for its `k` inserts the `j` first and then
   acts on the popup, as Vim does. v1.8 accepted first and appended the `j`
   after the completed word.

The shipped `editor.completion` rows are now only Vim's: `Ctrl-N`, `Ctrl-P`,
`Ctrl-Y` and `Ctrl-E`, so `:panelmap` lists 26 bindings instead of 31.

**`:w` no longer writes a shader into your script.** GodotVim runs in the
Shader Editor too, but `:w` saved the text of the editor you typed it in into
the script editor's current script. With a `.gdshader` focused, `:w` replaced
the open `.gd` file with the shader's text, and `:wq`, `:x`, `ZZ`, `:wqa` and
`:save` did the same. These commands now save only when typed in the script
editor's current script. Anywhere else they stop with `E382: Cannot write`
and change no file; save a shader with the Shader Editor's own Save. If you
used `:w` in a shader before, check the script that was open at the time: it
may hold the shader's text.

**`:q` in a shader checks the shader.** It used to refuse with `E37` when the
current script had unsaved changes, although only the shader tab would close,
and it could close a shader with unsaved changes without a word. It now
checks the editor you typed it in. `:e! {file}` typed in a shader no longer
loads the current script's text into the shader; it opens the file.

**The Sandbox strips every mapping that reaches `:source` or `:!`.** Under
the default Project Vimrc policy, a mapping in a committed
`res://.godot-vimrc` was stripped only when a space followed `:source` or
the `!` came right after the `:`. `:source<CR>`, `:silent !cmd`,
`:noh<Bar>!cmd` and `:<Space>so file` passed. They are now stripped too,
with the usual `" [sandbox] stripped` comment. Put such a mapping in
`user://.godot-vimrc` if you want it.

## From v1.8.0

**Textwidth now defaults to `0`, as in Vim.** With the old default of 80,
typing in Insert mode on a line longer than 80 characters broke the line,
because vim-core's `formatoptions` contains `t`. In GDScript that splits a
statement, and the break could also misplace the characters typed next.

1. **If you never changed Textwidth**, it moves to `0` the first time this
   version loads: when you restart the editor, or when you disable and enable
   the plugin again under Project Settings > Plugins. Nothing breaks lines
   while you type, and `gq` formats at 79 columns.
2. **If you set another width**, such as 100, it is kept. Typing past it
   now wraps only comments in GDScript and shaders, as described below. Set
   it to `0` to stop wrapping entirely.
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

**GDScript wraps only comments, never code.** GodotVim now detects each
script's filetype and runs Vim's filetype plugins for it, transcribed from
Vim 9.1 (see [Filetype Plugins](REFERENCE.md#filetype-plugins)). With a
Textwidth above `0`:

- GDScript, shader and C# code no longer breaks while you type. `#`, `##`,
  `//` and `///` comment lines break at the last blank before the width
  and continue with the same indent and leader. For GDScript and shaders
  this deliberately differs from Vim's `gdscript.vim` and `gdshader.vim`,
  which let code wrap.
- JSON and `.cfg` values never wrap. Markdown and text wrap as prose, as in
  Vim.
- The break no longer misplaces or deletes the characters you type, and it
  happens where Vim breaks: when you type a non-blank character past the
  width, at the last blank before the cursor.

Nothing wraps unless you set a width, since the default is now `0`.

**`formatoptions` and `comments` can be set.** `:set fo-=t`,
`:setlocal fo+=t` and `:set comments^=b:##` work as in Vim, including in a
`.godot-vimrc`. `:setlocal` changes typing as well as `gq`, and a
change to an Editor Setting wins over an earlier `:set` everywhere, in every
script.

**`commentstring` is per script.** It used to be copied from the script
editor's comment delimiters into the global value on every tab switch, so a
shader's `//` could reach the next text file. Each script with comment
delimiters (GDScript, shaders, C#) now gets its own value from them once,
so a `set commentstring` in your vimrc reaches only scripts without
delimiters, such as text files. A `:setlocal commentstring` you make
overrides it in that script and stays with it.

**To keep the old behaviour** (Vim's defaults in every script, so code wraps
with a width set), turn off **Godot Vim > Editor > Filetype Plugin**, or put
`filetype plugin off` in your `.godot-vimrc`. The commentstring still comes
from the script editor either way.

## From v1.7.x

The completion popup keys now resolve through the same pipeline as every
other panel binding, and three long-standing defects went with the old
transport. Four notes:

1. **Tab and Enter accepted only a candidate you chose.** v1.9.0 reverted
   this: they accept the highlighted row again, as in Godot, and follow your
   Editor Settings > Shortcuts. See [From v1.8.2](#from-v182), which also
   says which v1.8 lines to delete.

2. **Ctrl-Space, Ctrl-N and Ctrl-P now open the popup.** They never did: the
   old gate read a `CodeEdit` flag Godot's script editor never sets, so
   Ctrl-Space fell through to Vim's `i_CTRL-@`, which pastes your previous
   insert and exits Insert mode. If you relied on that, remove Ctrl+Space
   from Completion Query in Editor Settings > Shortcuts (since v1.9.0; v1.8
   used `panelunmap editor.completion <C-@>`).

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
