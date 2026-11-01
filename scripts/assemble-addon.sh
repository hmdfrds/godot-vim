#!/usr/bin/env bash
#
# Assemble a ready-to-copy addons/godot_vim/ folder.
#
# This is the one place the addon layout is defined. release.yml calls it to
# build the published zip, and a person building from source calls it to get
# the same folder on their own machine, so the two cannot drift.
#
#   ./scripts/assemble-addon.sh                 build with cargo, stage this
#                                               platform's library
#   ./scripts/assemble-addon.sh --no-build      stage whatever is already in
#                                               target/release/ (or --lib)
#   ./scripts/assemble-addon.sh --lib a.so --lib b.dll ...
#                                               stage the given libraries;
#                                               CI passes every platform
#   ./scripts/assemble-addon.sh --android       also build (unless --no-build)
#                                               and stage the arm64 and x86_64
#                                               Android libraries, as
#                                               libgodot_vim.android.<arch>.so
#                                               (docs/BUILDING.md has the NDK
#                                               setup)
#   ./scripts/assemble-addon.sh --out DIR       stage into DIR (default: dist/)
#   ./scripts/assemble-addon.sh --version vX.Y.Z
#                                               wrap in godot-vim-vX.Y.Z/ and
#                                               rewrite media links to that
#                                               tag (release layout)
#
# Output: <out>/addons/godot_vim/ (or <out>/godot-vim-<version>/addons/godot_vim/
# with --version). Copy the addons/ folder into a Godot project.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

BUILD=1; ANDROID=0; OUT=dist; VERSION=""; LIBS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --no-build) BUILD=0 ;;
    --android) ANDROID=1 ;;
    --lib) LIBS+=("$2"); shift ;;
    --out) OUT="$2"; shift ;;
    --version) VERSION="$2"; shift ;;
    -h|--help) sed -n '2,29p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

if [ ${#LIBS[@]} -eq 0 ]; then
  if [ "$BUILD" = 1 ]; then
    cargo build --release --locked
  fi
  # Whichever of the three this platform produced. --target builds land under
  # target/<triple>/release/, which a plain build never creates; that is why
  # this looks in target/release/ only and CI passes --lib explicitly.
  for cand in target/release/libgodot_vim.so target/release/godot_vim.dll target/release/libgodot_vim.dylib; do
    [ -f "$cand" ] && LIBS+=("$cand")
  done
  if [ ${#LIBS[@]} -eq 0 ] && [ "$ANDROID" = 0 ]; then
    echo "no library found in target/release/; run cargo build --release or pass --lib" >&2
    exit 1
  fi
fi

# Each entry is <source>:<name in bin/>. A --lib keeps its own name.
STAGED=()
for lib in ${LIBS[@]+"${LIBS[@]}"}; do
  [ -f "$lib" ] || { echo "not a file: $lib" >&2; exit 1; }
  STAGED+=("$lib:$(basename "$lib")")
done

# An Android build is libgodot_vim.so too, so it takes the name the
# .gdextension gives it rather than overwriting the Linux library.
if [ "$ANDROID" = 1 ]; then
  for pair in aarch64-linux-android:arm64 x86_64-linux-android:x86_64; do
    triple="${pair%%:*}"
    if [ "$BUILD" = 1 ]; then
      cargo build --release --locked --target "$triple"
    fi
    lib="target/$triple/release/libgodot_vim.so"
    [ -f "$lib" ] || { echo "no Android library at $lib; see docs/BUILDING.md" >&2; exit 1; }
    STAGED+=("$lib:libgodot_vim.android.${pair##*:}.so")
  done
fi

DUPLICATE=$(for entry in "${STAGED[@]}"; do echo "${entry##*:}"; done | sort | uniq -d)
if [ -n "$DUPLICATE" ]; then
  echo "two libraries would be staged as: $DUPLICATE" >&2
  exit 1
fi

# Wrap addons/ in a version-named folder for the release zip. Godot Asset
# Library's "Ignore asset root" (default checked) strips that wrapper and
# places addons/ at the project root.
if [ -n "$VERSION" ]; then
  STAGE="$OUT/godot-vim-$VERSION"
else
  STAGE="$OUT"
fi
ADDON="$STAGE/addons/godot_vim"
rm -rf "$STAGE"
mkdir -p "$ADDON/bin" "$ADDON/docs"

cp addons/godot_vim/plugin.cfg addons/godot_vim/godot_vim.gd addons/godot_vim/godot_vim.gdextension "$ADDON/"
for entry in "${STAGED[@]}"; do cp "${entry%:*}" "$ADDON/bin/${entry##*:}"; done

# EVERYTHING ships INSIDE addons/godot_vim/, nothing beside it.
#
# This is not tidiness. Asset Library strips the wrapper root, so a file
# sitting next to addons/ lands at the USER'S project root, and LICENSE and
# README.md would overwrite theirs. An addon must be self-contained: drop the
# folder in, delete the folder out, nothing else touched.
cp LICENSE README.md .godot-vimrc.example "$ADDON/"
# docs/ is copied file by file rather than cp -r, so a future internal doc
# landing in docs/ cannot silently ship to every user's project tree. Every
# file listed here is one the README links to relatively; a doc the README
# links to but this list omits is a dead link in every installed copy.
cp docs/REFERENCE.md docs/DEBUGGING.md docs/BUILDING.md "$ADDON/docs/"

# The repo README uses relative media/ paths so GitHub renders it. media/ is
# MBs of gifs and is deliberately NOT shipped, so in the SHIPPED copy rewrite
# those references to raw URLs. Pinned to the release tag when there is one,
# so an old release's images keep matching that release; main otherwise.
REF="${VERSION:-main}"
sed -i.bak "s|](media/|](https://raw.githubusercontent.com/hmdfrds/godot-vim/${REF}/media/|g; s|src=\"media/|src=\"https://raw.githubusercontent.com/hmdfrds/godot-vim/${REF}/media/|g" "$ADDON/README.md"
rm -f "$ADDON/README.md.bak"

# Fail loudly rather than producing a tree that pollutes a user project.
UNEXPECTED=$(find "$STAGE" -mindepth 1 -maxdepth 1 ! -name addons)
if [ -n "$UNEXPECTED" ]; then
  echo "files outside addons/ would land in the user's project root:" >&2
  echo "$UNEXPECTED" >&2
  exit 1
fi

echo "assembled: $ADDON"
find "$ADDON" -type f | sort | sed "s|^$STAGE/|  |"
