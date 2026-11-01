# Building GodotVim from source

Build it yourself if your distribution's glibc is older than 2.34 so the
published Linux binary will not load, if you are on Linux arm64 which no release
ships, or if you want to change the plugin.

You need Rust 1.94 or newer on the stable channel, from
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
After a rebuild, restart the editor too: the library does not reload while the
editor runs (`reloadable = false`).

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

## Building for the Android editor

Releases carry `libgodot_vim.android.arm64.so` for phones and tablets and
`libgodot_vim.android.x86_64.so` for emulators and ChromeOS. Building them
needs the Android NDK, r28 or newer: older ones link with 4 KB pages, which a
device with 16 KB pages refuses. The release uses r29, the NDK Godot builds
with. Nothing else is needed: no cargo-ndk and no C++ runtime.

```bash
rustup target add aarch64-linux-android x86_64-linux-android
NDK=~/Android/Sdk/ndk/29.0.14206865
BIN="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$BIN/aarch64-linux-android24-clang"
export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="$BIN/x86_64-linux-android24-clang"
./scripts/assemble-addon.sh --android
```

The `24` is the API level, Godot's minimum. On macOS the NDK's folder is
`darwin-x86_64`; on Windows it is `windows-x86_64` and the linkers end in
`.cmd`. `--android` builds both targets after this platform's library and
stages them as `bin/libgodot_vim.android.<arch>.so`, the names the
`.gdextension` gives them; the build itself writes `libgodot_vim.so` under
`target/<triple>/release/`. The release checks every Android library with

```bash
READELF="$BIN/llvm-readelf" ./scripts/check-android-lib.sh dist/addons/godot_vim/bin/libgodot_vim.android.*.so
```

which wants every LOAD segment aligned to 16 KB or more, nothing needed beyond
`libc.so`, `libm.so` and `libdl.so`, and `gdext_rust_init` exported.

Copy `addons/godot_vim/` into the project on the device as for any install.
The editor copies the library into its own cache before it loads it, because
Android loads no code from shared storage. 32-bit Android does not build:
gdext's prebuilt bindings are 64-bit only.

## Changing code that calls Godot

The plugin reaches Godot through godot-rust (gdext) 0.5, built against the 4.5
API by the `api-4-5` feature of the `godot` dependency. That is the
`compatibility_minimum` in `addons/godot_vim/godot_vim.gdextension`, because a
library built against a newer API refuses to start on an older Godot, and
`cargo test` fails when the two differ. gdext's default API, 4.6, does not load
on 4.5. API newer than 4.5 is called by name, in `src/bridge/godot_calls.rs`.

These rules keep new code right on gdext 0.5 and building on 0.6:

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
- No `#[func]` takes the name of a method of a base class. gdext warns about
  it in debug builds, and 0.6 will reject it.
- Use nothing gdext marks deprecated; the next major release removes it.
  `-D warnings` stops a deprecated call, but not a deprecated trait method you
  implement.
- Getters (`get_`, `is_`, `has_`) take `&self`, so bind a `Gd` without `mut`
  unless a `&mut self` method follows. The `FileAccess` stream reads
  (`get_line`, `get_8` to `get_64`, `get_buffer`, `get_csv_line`, `get_var`
  and the like) take `&mut self`; `get_as_text()` and
  `FileAccess::get_file_as_string` do not.
- `Node::get_tree()` panics outside the tree. Code that can run before
  `enter_tree`, after `exit_tree`, or on a node that may not be in the tree
  calls `get_tree_or_null()`.
- Nullable returns stay `Option` at the 4.5 API. A higher API level makes some
  of them non-null, such as `create_tween()` from 4.6 and `get_root()` from
  4.7, so raising the floor changes their call sites.
- Import `Side`, `Corner` and `EulerOrder` from `godot::builtin`, and
  `PropertyHint` from `godot::register::info`; the prelude does not have them.
- Compare a `GString` with a `&str` directly, as in `name == "Windows"`.
  `== text.into()` does not compile.
