#!/usr/bin/env bash
#
# Load the plugin in a headless Godot editor and fail on any error or warning
# in the editor's log.
#
#   ./scripts/godot-smoke.sh --godot BIN        test the library in
#                                               target/release/ (or
#                                               $CARGO_TARGET_DIR/release/)
#   ./scripts/godot-smoke.sh --godot BIN --lib FILE
#                                               test this library
#   ./scripts/godot-smoke.sh ... --keep DIR     leave the project and logs in
#                                               DIR instead of a temp dir; DIR
#                                               must be absent or empty
#
# GODOT=BIN works in place of --godot. The addon is assembled by
# assemble-addon.sh, the same script the release uses, into a copy of
# scripts/godot-smoke/. The editor then starts twice: on the fresh project, as
# after a manual install, and again on the imported project, as on every
# later start. Each start must print SMOKE: PASS from the probe plugin, exit 0
# and log no ERROR, WARNING or panic. HOME and the XDG directories point into
# the scratch directory, so your own editor settings and vimrc are not read.
set -euo pipefail
REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

GODOT_BIN=${GODOT:-}; LIB=""; KEEP=""
while [ $# -gt 0 ]; do
  case "$1" in
    --godot) GODOT_BIN="$2"; shift ;;
    --lib) LIB="$2"; shift ;;
    --keep) KEEP="$2"; shift ;;
    -h|--help) sed -n '2,21p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

[ -n "$GODOT_BIN" ] || { echo "no Godot binary: pass --godot or set GODOT" >&2; exit 2; }
if [ -z "$LIB" ]; then
  for cand in "${CARGO_TARGET_DIR:-$REPO/target}"/release/{libgodot_vim.so,libgodot_vim.dylib,godot_vim.dll}; do
    [ -f "$cand" ] && { LIB="$cand"; break; }
  done
  [ -n "$LIB" ] || { echo "no library in ${CARGO_TARGET_DIR:-$REPO/target}/release/; build it or pass --lib" >&2; exit 2; }
fi
[ -f "$LIB" ] || { echo "not a file: $LIB" >&2; exit 2; }
# assemble-addon.sh runs from the repository root.
LIB="$(cd "$(dirname "$LIB")" && pwd)/$(basename "$LIB")"

if [ -n "$KEEP" ]; then
  if [ -e "$KEEP" ] && { [ ! -d "$KEEP" ] || [ -n "$(ls -A "$KEEP")" ]; }; then
    echo "--keep needs an absent or empty directory: $KEEP" >&2; exit 2
  fi
  mkdir -p "$KEEP"; WORK=$(cd "$KEEP" && pwd)
else
  WORK=$(mktemp -d); trap 'rm -rf "${WORK:?}"' EXIT
fi
PROJECT="$WORK/project"
cp -r "$REPO/scripts/godot-smoke" "$PROJECT"
"$REPO/scripts/assemble-addon.sh" --lib "$LIB" --out "$WORK/addon" >/dev/null
cp -r "$WORK/addon/addons/godot_vim" "$PROJECT/addons/"

LIMIT=()
command -v timeout >/dev/null && LIMIT=(timeout -k 10 180)
godot() {
  env HOME="$WORK/home" XDG_CONFIG_HOME="$WORK/home/.config" \
    XDG_DATA_HOME="$WORK/home/.local/share" XDG_CACHE_HOME="$WORK/home/.cache" \
    ${LIMIT[@]+"${LIMIT[@]}"} "$GODOT_BIN" "$@"
}
mkdir -p "$WORK/home"
echo "godot $(godot --headless --version | tail -n 1), library $LIB"

ESC=$'\033'
failed=0
for run in fresh reopen; do
  log="$WORK/$run.log"
  code=0
  godot --headless --editor --path "$PROJECT" > "$log.raw" 2>&1 || code=$?
  sed "s/${ESC}\[[0-9;]*m//g" "$log.raw" > "$log" && rm "$log.raw"
  problems=$(grep -E -A1 '^(ERROR|WARNING|SCRIPT ERROR|SHADER ERROR|UNKNOWN ERROR):|panicked at' "$log" || true)
  if [ "$code" -eq 0 ] && grep -qx 'SMOKE: PASS' "$log" && [ -z "$problems" ]; then
    echo "$run start: pass"
    continue
  fi
  failed=1
  [ -n "${GITHUB_ACTIONS:-}" ] && echo "::error::godot smoke test failed on the $run start (exit $code)"
  echo "$run start: FAIL (exit $code)"
  grep -E '^SMOKE: ' "$log" || echo "no SMOKE: line from the probe"
  [ -z "$problems" ] || printf 'errors and warnings:\n%s\n' "$problems"
  echo "--- $log"
  cat "$log"
  echo "---"
done
exit "$failed"
