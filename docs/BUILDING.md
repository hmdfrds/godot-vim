# Building GodotVim from source

Build it yourself if your distribution's glibc is older than 2.34 so the
published Linux binary will not load, if you are on Linux arm64 which no release
ships, or if you want to change the plugin.

You need Rust 1.87 or newer on the stable channel, from
[rustup](https://rustup.rs), and a C toolchain for the linker: `build-essential`
on Debian and Ubuntu, the Xcode Command Line Tools on macOS, Visual Studio Build
Tools with the C++ workload on Windows. You do not need a Godot binary, clang,
LLVM, Python or SCons; the Godot bindings come from a prebuilt API description,
so there is no bindgen step. Every dependency is a public repository, so no
credentials and no GitHub account are involved.

```bash
git clone --depth 1 https://github.com/hmdfrds/godot-vim
cd godot-vim
cargo build --release --locked
```

The library lands in `target/release/`: `libgodot_vim.so` on Linux,
`godot_vim.dll` on Windows (MSVC), `libgodot_vim.dylib` on macOS.

Then assemble the addon. This repository is not a Godot project, and
`addons/godot_vim/bin/` is gitignored, so the folder you install is produced,
not checked in:

```bash
./scripts/assemble-addon.sh --no-build
```

That writes `dist/addons/godot_vim/`, complete with the library, `plugin.cfg`,
the `.gdextension`, and the docs. It is the same script the release workflow
runs to build the published zip, so what you get is what a release contains.
Copy the `addons` folder into your project:

```bash
cp -r dist/addons <your-project>/
```

The result should be:

```
<your-project>/addons/godot_vim/
├── plugin.cfg
├── godot_vim.gd
├── godot_vim.gdextension
├── bin/libgodot_vim.so
└── docs/
```

Without `--no-build` the script runs `cargo build --release --locked` first, so
`./scripts/assemble-addon.sh` on its own is the whole procedure. Pass
`--out DIR` to stage somewhere other than `dist/`.

Enable it in **Project Settings > Plugins** and restart the editor, the same as
any other install. The `.debug` and `.release` entries in the `.gdextension`
refer to the Godot build, not the Rust profile, so one artifact serves both.

To check a build without opening your own project, run
`./scripts/godot-smoke.sh --godot <path-to-godot>`. It installs the library in a
throwaway project, starts a headless editor, opens a script, and fails if the
plugin does not attach or anything prints an error or warning. CI runs it on
every Godot version in its matrix, from 4.5, the oldest the plugin supports.

**What to expect.** Roughly five minutes from a cold cache, about 700 MB in
`target/` and 210 MB in `~/.cargo`. Peak memory during the link approaches 3 GB,
because the release profile uses fat LTO with a single codegen unit. One warning,
`method get_text is never used`, appears in release builds only and is harmless.
Build with `--release`: a debug build of this library is about 190 MB and its
`target/` runs into gigabytes.

**What not to change.** Keep `--locked` and do not run `cargo update`.
`compact_str` is pinned to 0.7 to match the version vim-core exports across the
API boundary, and the reason is written above the dependency in `Cargo.toml`. Do
not change `panic = "unwind"` in `[profile.release]`; `src/safety.rs` catches
panics at every signal-handler boundary so a Rust panic cannot unwind across the
FFI into Godot, and that needs unwinding to exist.

**Platform notes.**

- `cargo build --release --target <triple>` writes to `target/<triple>/release/`,
  not `target/release/`. The release workflow always passes `--target`, so a path
  copied out of it points at a directory a plain build never creates.
- On Windows, close the Godot editor before rebuilding. Windows keeps the loaded
  DLL locked and the link step fails with `LNK1104`.
- On macOS a single-architecture build loads fine on your own machine. `lipo` is
  only needed to ship one file covering both:

  ```bash
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  cargo build --release --locked --target aarch64-apple-darwin
  cargo build --release --locked --target x86_64-apple-darwin
  lipo -create \
    target/aarch64-apple-darwin/release/libgodot_vim.dylib \
    target/x86_64-apple-darwin/release/libgodot_vim.dylib \
    -output libgodot_vim.dylib
  ```

- Under WSL, keep the clone on the Linux filesystem (`~/...`) rather than under
  `/mnt/c`, where DrvFs makes cargo's small-file work pathologically slow.
  `CARGO_INCREMENTAL` is not a factor here: cargo disables incremental
  compilation for release builds, and `lto = "fat"` with `codegen-units = 1`
  would defeat it anyway.

To work on the Vim engine rather than the plugin, use the `[patch]` snippet in
[vim-core's README](https://github.com/hmdfrds/vim-core#readme).

## Changing code that calls Godot

The plugin reaches Godot through godot-rust (gdext), built against the 4.5
API. That is the `compatibility_minimum` in
`addons/godot_vim/godot_vim.gdextension`, because a library built against a
newer API refuses to start on an older Godot, and `cargo test` fails when the
two differ. API newer than 4.5 is called by name, in
`src/bridge/godot_calls.rs`.

These rules keep new code building on gdext 0.5 and 0.6 too:

- Every `#[derive(GodotClass)]` is followed by `#[class(tool, ...)]`. All of
  the plugin's classes run in the editor, where gdext 0.6 will turn a class
  without `tool` into an inert placeholder. `cargo test` checks this.
- Only `safety::engine_loaded()` calls into `godot::sys`, gdext's private API,
  which changes without notice. Ask it whether the bindings are loaded.
  `cargo test` checks this too.
- Connect engine signals by name, with `connect` or `connect_flags`, not
  through typed signals, whose handlers panic when an argument is an object
  freed before the call. A `DEFERRED` handler takes such an argument as a
  `Variant`, as `src/plugin/signals.rs` explains.
- No `#[func]` takes the name of a method of a base class. gdext 0.5 warns
  about it in debug builds, and 0.6 will reject it.
- Use nothing gdext marks deprecated, such as `on_level_init` or
  `Callable::from_local_fn`; 0.5 removed all of it. `-D warnings` stops a
  deprecated call, but not a deprecated trait method you implement.
- Import `Side`, `Corner` and `EulerOrder` from `godot::builtin`. gdext 0.5's
  prelude does not have them.
- Compare a `GString` with `GString::from(text)`. `== text.into()` does not
  compile on gdext 0.5.
- Read a file whole with `get_as_text()` or `FileAccess::get_file_as_string`.
  The stream reads (`get_line`, `get_8` to `get_64`, `get_buffer`,
  `get_csv_line`, `get_var` and the like) take `&self` on gdext 0.4 and
  `&mut self` on 0.5, so no binding for them builds cleanly on both.
