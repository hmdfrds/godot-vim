#!/usr/bin/env bash
#
# Check Android builds of the library before they ship:
#
#   READELF=<ndk>/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf \
#     ./scripts/check-android-lib.sh libgodot_vim.android.arm64.so ...
#
# Every LOAD segment aligned to 16 KB or more (devices with 16 KB pages
# refuse less), NEEDED only libc.so, libm.so and libdl.so (nothing the editor
# APK lacks), and gdext_rust_init exported. READELF defaults to readelf.
set -euo pipefail

READELF="${READELF:-readelf}"
[ $# -gt 0 ] || { echo "usage: $0 LIB..." >&2; exit 2; }

status=0
for lib in "$@"; do
  bad=0
  aligns=$("$READELF" -lW "$lib" | awk '$1 == "LOAD" { print $NF }')
  if [ -z "$aligns" ]; then
    echo "$lib: no LOAD segment" >&2
    bad=1
  fi
  for align in $aligns; do
    if [ $((align)) -lt $((0x4000)) ]; then
      echo "$lib: a LOAD segment is aligned to $align, below 16 KB" >&2
      bad=1
    fi
  done

  extra=$("$READELF" -dW "$lib" | awk '/\(NEEDED\)/ { print $NF }' | tr -d '[]' \
    | grep -vxE 'lib(c|m|dl)\.so' || true)
  if [ -n "$extra" ]; then
    echo "$lib: needs more than libc, libm and libdl: $(echo "$extra" | tr "\n" " ")" >&2
    bad=1
  fi

  if ! "$READELF" --dyn-syms -W "$lib" \
    | awk '$NF == "gdext_rust_init" { found = 1 } END { exit !found }'; then
    echo "$lib: gdext_rust_init is not exported" >&2
    bad=1
  fi

  if [ "$bad" = 0 ]; then
    echo "$lib: ok"
  else
    status=1
  fi
done
exit "$status"
