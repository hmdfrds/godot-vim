#!/usr/bin/env bash
#
# Godot's undo history from a real CodeEdit, for the mock's differential
# test (src/testing/undo_trace_tests.rs), and the time of undo() bursts.
#
#   ./scripts/godot-undo-trace.sh --godot BIN            run steps.txt and
#                                                        compare with
#                                                        recorded/<godot>.txt
#   ./scripts/godot-undo-trace.sh --godot BIN --record   write it instead
#   ./scripts/godot-undo-trace.sh --godot BIN --burst N  time N undo() calls,
#                                                        each one edit of a
#                                                        2,000-line script
#
# GODOT=BIN works in place of --godot. Exit 0 when the trace matches (or was
# written, or the burst restored the text), 1 when it differs, 2 when Godot
# could not run. Scratch space is a temp dir under TMPDIR.
set -euo pipefail
REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
DIR="$REPO/scripts/godot-undo-trace"

GODOT_BIN=${GODOT:-}; RECORD=0; BURST=""
while [ $# -gt 0 ]; do
  case "$1" in
    --godot) GODOT_BIN="$2"; shift ;;
    --record) RECORD=1 ;;
    --burst) BURST="$2"; shift ;;
    -h|--help) sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done
[ -n "$GODOT_BIN" ] || { echo "no Godot binary: pass --godot or set GODOT" >&2; exit 2; }
[ -z "$BURST" ] || [[ "$BURST" =~ ^[1-9][0-9]*$ ]] || { echo "--burst needs a positive number" >&2; exit 2; }

WORK=$(mktemp -d); trap 'rm -rf "${WORK:?}"' EXIT
mkdir -p "$WORK/project" "$WORK/home"
cp "$DIR/project.godot" "$DIR/trace.gd" "$WORK/project/"

LIMIT=()
command -v timeout >/dev/null && LIMIT=(timeout -k 10 60)
godot() {
  env -i PATH="$PATH" LANG=C.UTF-8 HOME="$WORK/home" XDG_CONFIG_HOME="$WORK/home/.config" \
    XDG_DATA_HOME="$WORK/home/.local/share" XDG_CACHE_HOME="$WORK/home/.cache" \
    ${LIMIT[@]+"${LIMIT[@]}"} "$GODOT_BIN" "$@"
}

# The label the suite uses: 4.7.2-stable, 4.8-dev7.
VERSION=$(godot --headless --version 2>/dev/null | tail -n 1)
IFS=. read -ra VP <<< "$VERSION"
if [[ "${VP[2]:-}" =~ ^[0-9]+$ ]]; then
  LABEL="${VP[0]}.${VP[1]}.${VP[2]}-${VP[3]}"
else
  LABEL="${VP[0]}.${VP[1]}-${VP[2]:-}"
fi
[[ "$LABEL" =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?-[a-z]+[0-9]*$ ]] || { echo "cannot read the Godot version from: $VERSION" >&2; exit 2; }

if [ -n "$BURST" ]; then
  code=0
  godot --headless --path "$WORK/project" --script res://trace.gd -- --burst "$BURST" > "$WORK/log" 2>&1 || code=$?
  line=$(grep '^BURST ' "$WORK/log" || true)
  [ -n "$line" ] || { echo "godot $LABEL: no BURST line (exit $code)" >&2; cat "$WORK/log" >&2; exit 2; }
  echo "godot $LABEL $line"
  [ "$code" -eq 0 ] || exit 1
  exit 0
fi

code=0
godot --headless --path "$WORK/project" --script res://trace.gd -- \
  --steps "$DIR/steps.txt" --out "$WORK/trace.txt" > "$WORK/log" 2>&1 || code=$?
if [ "$code" -ne 0 ] || [ ! -s "$WORK/trace.txt" ]; then
  echo "godot $LABEL: the trace did not run (exit $code)" >&2; cat "$WORK/log" >&2; exit 2
fi
{ echo "# godot $LABEL"; cat "$WORK/trace.txt"; } > "$WORK/recorded.txt"
grep -E '^(ERROR|WARNING|SCRIPT ERROR):' -A1 "$WORK/log" || true

OUT="$DIR/recorded/$LABEL.txt"
if [ "$RECORD" = 1 ]; then
  cp "$WORK/recorded.txt" "$OUT"
  echo "godot $LABEL: wrote $OUT"
  exit 0
fi
[ -f "$OUT" ] || { echo "godot $LABEL: no recording at $OUT; run with --record" >&2; exit 1; }
if diff -u "$OUT" "$WORK/recorded.txt"; then
  echo "godot $LABEL: matches $OUT"
else
  echo "godot $LABEL: differs from $OUT"
  exit 1
fi
