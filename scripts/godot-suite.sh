#!/usr/bin/env bash
#
# Run godot-vim's behaviour cases in real headless Godot editors, one fresh
# editor per case, and compare the failing set with known-failures.txt.
#
#   ./scripts/godot-suite.sh --godot BIN        every case, library from
#                                               target/release/ (or
#                                               $CARGO_TARGET_DIR/release/)
#   ... --lib FILE                              test this library
#   ... --addon DIR                             test a ready addons/godot_vim
#                                               folder, such as a release zip's
#   ... --only a,b                              these cases only
#   ... --cases FILE                            an ad hoc case file instead of
#                                               cases.jsonl
#   ... --jobs N                                editors at once (default 4, at
#                                               most nproc)
#   ... --keep DIR                              keep projects and logs in DIR,
#                                               which must be absent or empty
#   ... --record [--out DIR]                    record vanilla Godot for the
#                                               oracle cases
#   ... --bless                                 rewrite this Godot's pins
#   ... --print ID                              print one case's snapshots as
#                                               an expect block
#   ... --realtime                              without --fixed-fps; diagnostic
#   ./scripts/godot-suite.sh --notes            the pins per Godot as Markdown
#
# GODOT=BIN works in place of --godot. HOME and the XDG directories point
# into the scratch directory, fresh for every case. Exit 0: the failing set
# is as pinned; 1: it changed, or a recording failed; 2: the run could not
# happen. Case format and rules: scripts/godot-suite/README.md.
set -euo pipefail
REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
SUITE="$REPO/scripts/godot-suite"
PINS="$SUITE/known-failures.txt"
ALLOW="$SUITE/allow-log.txt"
PROBLEM_RE='^(ERROR|WARNING|SCRIPT ERROR|SHADER ERROR|UNKNOWN ERROR):|panicked at'
PORT_BASE=26005

GODOT_BIN=${GODOT:-}; LIB=""; ADDON=""; ONLY=""; CASES="$SUITE/cases.jsonl"; CUSTOM=0
JOBS=""; KEEP=""; RECORD=0; OUT=""; BLESS=0; PRINT=""; NOTES=0; REALTIME=0
while [ $# -gt 0 ]; do
  case "$1" in
    --godot) GODOT_BIN="$2"; shift ;;
    --lib) LIB="$2"; shift ;;
    --addon) ADDON="$2"; shift ;;
    --only) ONLY="$2"; shift ;;
    --cases) CASES=$(realpath "$2"); CUSTOM=1; shift ;;
    --jobs) JOBS="$2"; shift ;;
    --keep) KEEP="$2"; shift ;;
    --record) RECORD=1 ;;
    --out) OUT="$2"; shift ;;
    --bless) BLESS=1 ;;
    --print) PRINT="$2"; shift ;;
    --notes) NOTES=1 ;;
    --realtime) REALTIME=1 ;;
    -h|--help) sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

die() { echo "$*" >&2; exit 2; }
annotate() { [ -z "${GITHUB_ACTIONS:-}" ] || echo "::$1::$2"; }

if [ "$NOTES" = 1 ]; then
  echo "## Known differences per Godot version"
  awk '!/^#/ && NF' "$PINS" | sort | while IFS= read -r line; do
    key=${line%%[[:space:]]*}
    godot=${key%%::*}; rest=${key#*::}
    reason=$(sed -n 's/^[^#]*#[[:space:]]*//p' <<< "$line")
    if [ "$godot" != "${last:-}" ]; then printf '\n### Godot %s\n\n' "$godot"; last=$godot; fi
    case "$rest" in
      *:log) echo "- \`${rest%:log}\` (log): $reason" ;;
      *) echo "- \`$rest\`: $reason" ;;
    esac
  done
  exit 0
fi

[ -n "$GODOT_BIN" ] || die "no Godot binary: pass --godot or set GODOT"
[ -x "$GODOT_BIN" ] || die "not an executable: $GODOT_BIN"
GODOT_BIN=$(realpath "$GODOT_BIN")
[ "$RECORD" = 0 ] || [ "$BLESS" = 0 ] || die "--record and --bless do not mix"
[ "$BLESS" = 0 ] || { [ -z "$ONLY" ] && [ "$CUSTOM" = 0 ]; } || die "--bless rewrites a whole version: drop --only and --cases"
[ -z "$OUT" ] || [ "$RECORD" = 1 ] || die "--out goes with --record"
if [ "$RECORD" = 1 ] && [ -z "$OUT" ] && [ -n "${CI:-}" ]; then
  die "--record writes into the tree; on CI pass --out DIR"
fi
[ -z "$PRINT" ] || ONLY="$PRINT"
[ -z "$LIB" ] || [ -z "$ADDON" ] || die "--lib and --addon do not mix"

# Cases: one JSON object per line, each line starting {"id": "...".
[ -f "$CASES" ] || die "no case file $CASES"
if grep -vn '^{"id": "' "$CASES" | grep -v '^[0-9]*:$' | head -3 | grep .; then
  die "$CASES: every non-empty line must start with {\"id\": \""
fi
mapfile -t ALL < <(sed -n 's/^{"id": "\([^"]*\)".*/\1/p' "$CASES")
dupes=$(printf '%s\n' "${ALL[@]}" | sort | uniq -d)
[ -z "$dupes" ] || die "duplicate case ids: $dupes"
for id in "${ALL[@]}"; do
  [[ "$id" =~ ^[a-z0-9][a-z0-9-]*$ ]] || die "case id is not kebab-case: $id"
done
case_line() { grep -m1 "^{\"id\": \"$1\"" "$CASES"; }
if [ -n "$ONLY" ]; then
  IFS=, read -ra SEL <<< "$ONLY"
  for id in "${SEL[@]}"; do
    printf '%s\n' "${ALL[@]}" | grep -qx "$id" || die "no case $id in $CASES"
  done
else
  SEL=("${ALL[@]}")
fi
if [ "$RECORD" = 1 ]; then
  REC=()
  for id in "${SEL[@]}"; do
    case_line "$id" | grep -q '"oracle": *"vanilla"' && REC+=("$id")
  done
  SEL=(${REC[@]+"${REC[@]}"})
  [ ${#SEL[@]} -gt 0 ] || die "no oracle vanilla case selected"
  VARIANT=vanilla
else
  VARIANT=plugin
fi

if [ "$VARIANT" = plugin ] && [ -z "$ADDON" ]; then
  if [ -z "$LIB" ]; then
    for cand in "${CARGO_TARGET_DIR:-$REPO/target}"/release/{libgodot_vim.so,libgodot_vim.dylib,godot_vim.dll}; do
      [ -f "$cand" ] && { LIB="$cand"; break; }
    done
    [ -n "$LIB" ] || die "no library in ${CARGO_TARGET_DIR:-$REPO/target}/release/; build it or pass --lib"
  fi
  [ -f "$LIB" ] || die "not a file: $LIB"
  LIB=$(realpath "$LIB")
fi
if [ -n "$ADDON" ]; then
  [ -f "$ADDON/addons/godot_vim/plugin.cfg" ] || die "--addon needs a folder holding addons/godot_vim/plugin.cfg"
  ADDON=$(realpath "$ADDON")
fi

if [ -n "$KEEP" ]; then
  if [ -e "$KEEP" ] && { [ ! -d "$KEEP" ] || [ -n "$(ls -A "$KEEP")" ]; }; then
    die "--keep needs an absent or empty directory: $KEEP"
  fi
  mkdir -p "$KEEP"; WORK=$(cd "$KEEP" && pwd)
else
  WORK=$(mktemp -d); trap 'rm -rf "${WORK:?}"' EXIT
fi
ROOT=$(realpath "$WORK")
mkdir -p "$WORK/logs" "$WORK/cases"

NPROC=$(nproc 2>/dev/null || echo 1)
JOBS=${JOBS:-4}
[[ "$JOBS" =~ ^[1-9][0-9]*$ ]] || die "--jobs needs a positive number"
[ "$JOBS" -le "$NPROC" ] || JOBS=$NPROC
[ "$JOBS" -le "${#SEL[@]}" ] || JOBS=${#SEL[@]}

FPS=(--fixed-fps 60)
[ "$REALTIME" = 0 ] || FPS=()

# env -i: the editor sees only these, so nothing of the developer's leaks in.
genv() {
  local d=$1; shift
  env -i PATH="$PATH" LANG=C.UTF-8 HOME="$d/home" XDG_CONFIG_HOME="$d/config" \
    XDG_DATA_HOME="$d/data" XDG_CACHE_HOME="${CACHE:-$d/cache}" "$@"
}
guard() {
  local d r
  for d in "$@"; do
    r=$(realpath "$d")
    [[ "$r" == "$ROOT"/* ]] || die "guard: $d resolves to $r, outside $ROOT"
  done
}
strip_ansi() { sed "s/$(printf '\033')\[[0-9;]*m//g"; }

mkdir -p "$WORK/v/home" "$WORK/v/config" "$WORK/v/data" "$WORK/v/cache"
VERSION=$(genv "$WORK/v" "$GODOT_BIN" --headless --version 2>/dev/null | tail -n 1)
IFS=. read -ra VP <<< "$VERSION"
if [[ "${VP[2]:-}" =~ ^[0-9]+$ ]]; then
  LABEL="${VP[0]}.${VP[1]}.${VP[2]}-${VP[3]}"
else
  LABEL="${VP[0]}.${VP[1]}-${VP[2]:-}"
fi
MINOR="${VP[0]}.${VP[1]}"
[[ "$LABEL" =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?-[a-z]+[0-9]*$ ]] || die "cannot read the Godot version from: $VERSION"
TEMPLATE_HASH=$(cat "$SUITE/project.godot" "$SUITE/main.gd" "$SUITE/main.tscn" "$SUITE/t.gdshader" "$SUITE/keys.tsv" | sha256sum | cut -d' ' -f1)

if [ "$VARIANT" = plugin ]; then
  if [ -n "$ADDON" ]; then
    mkdir -p "$WORK/addon/addons"
    cp -r "$ADDON/addons/godot_vim" "$WORK/addon/addons/"
    WHAT="addon $ADDON"
  else
    "$REPO/scripts/assemble-addon.sh" --lib "$LIB" --out "$WORK/addon" >/dev/null
    WHAT="library $LIB"
  fi
else
  WHAT="vanilla, recording"
fi
echo "godot $LABEL, $WHAT, ${#SEL[@]} cases, $JOBS editors (nproc $NPROC)"

make_base() {
  local v=$1 dir="$WORK/base/$1" plugins='"res://addons/suite_probe/plugin.cfg"'
  mkdir -p "$dir/addons"
  cp "$SUITE/project.godot" "$SUITE/main.gd" "$SUITE/main.tscn" "$SUITE/t.gdshader" "$dir/"
  cp -r "$SUITE/addons/suite_probe" "$dir/addons/"
  if [ "$v" = plugin ]; then
    cp -r "$WORK/addon/addons/godot_vim" "$dir/addons/"
    mkdir -p "$dir/.godot"
    echo "res://addons/godot_vim/godot_vim.gdextension" > "$dir/.godot/extension_list.cfg"
    plugins="\"res://addons/godot_vim/plugin.cfg\", $plugins"
  fi
  printf '\n[editor_plugins]\n\nenabled=PackedStringArray(%s)\n' "$plugins" >> "$dir/project.godot"
}
make_base "$VARIANT"
BASE="$WORK/base/$VARIANT"

# The project is imported once; every case starts from a copy of it.
mkdir -p "$WORK/imp/home" "$WORK/imp/config" "$WORK/imp/data" "$WORK/imp/cache"
guard "$WORK/imp/home" "$WORK/imp/config" "$WORK/imp/data" "$WORK/imp/cache"
code=0
genv "$WORK/imp" timeout -k 10 300 "$GODOT_BIN" --headless --editor --import \
  --lsp-port $((PORT_BASE - 2)) --dap-port $((PORT_BASE - 1)) --path "$BASE" \
  > "$WORK/logs/import.raw" 2>&1 || code=$?
strip_ansi < "$WORK/logs/import.raw" > "$WORK/logs/import.log"
if [ "$code" -ne 0 ]; then
  cat "$WORK/logs/import.log"
  die "import failed (exit $code)"
fi
if grep -E "$PROBLEM_RE" "$WORK/logs/import.log"; then
  echo "(problem lines in the import log above; cases continue)"
fi

code=0
genv "$WORK/imp" timeout -k 10 60 "$GODOT_BIN" --headless --path "$BASE" \
  --script res://addons/suite_probe/self_test.gd > "$WORK/logs/self_test.raw" 2>&1 || code=$?
strip_ansi < "$WORK/logs/self_test.raw" > "$WORK/logs/self_test.log"
if [ "$code" -ne 0 ] || ! grep -qx 'SELFTEST: PASS' "$WORK/logs/self_test.log" \
  || grep -qE "$PROBLEM_RE" "$WORK/logs/self_test.log"; then
  cat "$WORK/logs/self_test.log"
  die "compare.gd self-test failed (exit $code)"
fi

# Warm-up: one editor builds the documentation cache every case reuses,
# writes the editor settings file every case starts from, and lists the
# names of all editor settings for the fixture check.
W="$WORK/warm"
mkdir -p "$W/home" "$W/config" "$W/data" "$W/cache"
cp -a "$BASE" "$W/project"
guard "$W/home" "$W/config" "$W/data" "$W/cache"
code=0
genv "$W" SUITE_MODE=warmup SUITE_VARIANT="$VARIANT" SUITE_GODOT="$LABEL" SUITE_ROOT="$ROOT" \
  SUITE_OUT="$WORK/setting-names.txt" timeout -k 10 180 "$GODOT_BIN" --headless --editor "${FPS[@]}" \
  --lsp-port $((PORT_BASE - 2)) --dap-port $((PORT_BASE - 1)) --path "$W/project" \
  > "$WORK/logs/warmup.raw" 2>&1 || code=$?
strip_ansi < "$WORK/logs/warmup.raw" > "$WORK/logs/warmup.log"
SETTINGS="$W/config/godot/editor_settings-$MINOR.tres"
DOC=$(sed -n 's/^SUITE-WARM //p' "$WORK/logs/warmup.log")
if [ "$code" -ne 0 ] || [ -z "$DOC" ] || [ ! -f "$SETTINGS" ] || [ ! -f "$W/cache/godot/$DOC" ]; then
  cat "$WORK/logs/warmup.log"
  die "warm-up failed (exit $code)"
fi
# Paths into the warm-up's own dirs are left for each editor to fill in.
grep -vF "\"$W/" "$SETTINGS" > "$WORK/editor_settings.tres"

# fixtures/<name>/config/settings.txt holds "key = value" lines in Godot's
# text format; they replace the same keys of the base settings file.
merge_settings() {
  local tres=$1 txt=$2 key
  while IFS= read -r key; do
    grep -qxF "$key" "$WORK/setting-names.txt" || { echo "no editor setting $key"; return 1; }
  done < <(sed -n 's/^\([^#][^=]*[^ =]\) *=.*/\1/p' "$txt")
  awk -v txt="$txt" '
    BEGIN { while ((getline l < txt) > 0) if (l !~ /^#/ && l ~ /=/) { k = l; sub(/ *=.*/, "", k); set[k] = l; order[++n] = k } }
    { k = $0; sub(/ *=.*/, "", k); if (!(k in set)) print }
    END { for (i = 1; i <= n; i++) print set[order[i]] }
  ' "$tres" > "$tres.new" && mv "$tres.new" "$tres"
}

USERDIR="data/godot/app_userdata/godot-vim-suite"
MODE=run
[ "$RECORD" = 0 ] || MODE=record

run_case() {
  local k=$1 id=$2 cd="$WORK/cases/$2" code=0 fx fd t0 t1
  mkdir -p "$cd/home" "$cd/config/godot" "$cd/data"
  cp -a "$BASE" "$cd/project"
  cp "$WORK/editor_settings.tres" "$cd/config/godot/editor_settings-$MINOR.tres"
  fx=$(case_line "$id" | sed -n 's/.*"fixture": *"\([a-z0-9-]*\)".*/\1/p')
  if [ -n "$fx" ]; then
    fd="$SUITE/fixtures/$fx"
    if [ ! -d "$fd" ]; then
      echo "SUITE: FAIL $id 00000000 [\"no fixture directory $fx\"]" > "$cd/log"; return
    fi
    [ ! -d "$fd/res" ] || cp -a "$fd/res/." "$cd/project/"
    [ ! -d "$fd/user" ] || { mkdir -p "$cd/$USERDIR"; cp -a "$fd/user/." "$cd/$USERDIR/"; }
    [ ! -d "$fd/home" ] || cp -a "$fd/home/." "$cd/home/"
    if [ -f "$fd/config/editor_settings.tres" ]; then
      cp "$fd/config/editor_settings.tres" "$cd/config/godot/editor_settings-$MINOR.tres"
    fi
    if [ -f "$fd/config/settings.txt" ] \
      && ! err=$(merge_settings "$cd/config/godot/editor_settings-$MINOR.tres" "$fd/config/settings.txt"); then
      echo "SUITE: FAIL $id 00000000 [\"fixture $fx: $err\"]" > "$cd/log"; return
    fi
  fi
  guard "$cd/home" "$cd/config" "$cd/data" "$CACHE"
  t0=$(date +%s%N)
  genv "$cd" SUITE_MODE="$MODE" SUITE_CASE="$id" SUITE_CASES="$CASES" SUITE_VARIANT="$VARIANT" \
    SUITE_GODOT="$LABEL" SUITE_ROOT="$ROOT" SUITE_KEYS="$SUITE/keys.tsv" \
    SUITE_SETTING_NAMES="$WORK/setting-names.txt" SUITE_VANILLA="$SUITE/vanilla/$LABEL.jsonl" \
    SUITE_TEMPLATE_HASH="$TEMPLATE_HASH" SUITE_REALTIME="$REALTIME" \
    timeout -k 10 120 "$GODOT_BIN" --headless --editor "${FPS[@]}" \
    --lsp-port $((PORT_BASE + 2 * k)) --dap-port $((PORT_BASE + 2 * k + 1)) --path "$cd/project" \
    > "$cd/raw" 2>&1 || code=$?
  t1=$(date +%s%N)
  strip_ansi < "$cd/raw" > "$cd/log" && rm -f "$cd/raw"
  echo "$code" > "$cd/exit"
  echo $(((t1 - t0) / 1000000)) > "$cd/ms"
  # A project copy is about the size of the library; keep it only when asked.
  [ -n "$KEEP" ] || rm -rf "${cd:?}/project"
}

worker() {
  local k=$1 i
  CACHE="$WORK/cache/$k"
  mkdir -p "$CACHE/godot"
  cp "$W/cache/godot/$DOC" "$CACHE/godot/"
  for ((i = k; i < ${#SEL[@]}; i += JOBS)); do
    run_case "$k" "${SEL[$i]}"
  done
}

START=$(date +%s)
for ((k = 0; k < JOBS; k++)); do
  worker "$k" &
done
wait
ELAPSED=$(( $(date +%s) - START ))

# Allow rules for this Godot: "<godot-prefix> <case-or-*> <regex>".
allowed_for() {
  local id=$1 prefix cid re
  [ -f "$ALLOW" ] || return 0
  while read -r prefix cid re; do
    [[ -z "$prefix" || "$prefix" == \#* ]] && continue
    [[ "$LABEL" == "$prefix"* ]] || continue
    [[ "$cid" == "*" || "$cid" == "$id" ]] || continue
    printf '%s\n' "$re"
  done < "$ALLOW"
}
vanilla_lines_for() {
  local f="$SUITE/vanilla/$LABEL.log"
  [ -f "$f" ] || return 0
  sed -n "s/^$1 //p" "$f"
}
problems_of() {
  local id=$1 log="$WORK/cases/$1/log" p re
  p=$(grep -E "$PROBLEM_RE" "$log" || true)
  while IFS= read -r re; do
    [ -n "$re" ] && p=$(grep -vE -- "$re" <<< "$p" || true)
  done < <(allowed_for "$id")
  if [ -n "$p" ] && [ "$MODE" = run ]; then
    local van
    van=$(vanilla_lines_for "$id")
    [ -z "$van" ] || p=$(grep -vxF -f <(printf '%s\n' "$van") <<< "$p" || true)
  fi
  printf '%s' "$p"
}

ACTUAL="$WORK/failing.txt"; SKIPPED="$WORK/skipped.txt"; : > "$ACTUAL"; : > "$SKIPPED"
: > "$WORK/verdicts.txt"; : > "$WORK/obs.jsonl"; : > "$WORK/problems.txt"
pass=0; fail=0; skip=0; recfail=0
NORESULT=$(printf 'no result' | sha256sum | cut -c1-8)
for id in "${SEL[@]}"; do
  log="$WORK/cases/$id/log"
  code=$(cat "$WORK/cases/$id/exit" 2>/dev/null || echo "?")
  verdict=$(grep -E -m1 "^SUITE: (PASS|FAIL|SKIP|REC) $id( |$)" "$log" || true)
  grep "^SUITE-OBS " "$log" | sed 's/^SUITE-OBS //' >> "$WORK/obs.jsonl" || true
  if [ -z "$verdict" ]; then
    verdict="SUITE: FAIL $id $NORESULT [\"no result, exit $code; see cases/$id/log\"]"
  fi
  echo "$verdict" >> "$WORK/verdicts.txt"
  read -r _ word _ digest reasons <<< "$verdict"
  if [ "$MODE" = record ]; then
    [ "$word" = REC ] || { recfail=1; echo "FAIL $id: $reasons"; }
    continue
  fi
  case "$word" in
    PASS) pass=$((pass + 1)) ;;
    SKIP) skip=$((skip + 1)); echo "$id" >> "$SKIPPED" ;;
    FAIL) fail=$((fail + 1)); echo "$LABEL::$id $digest" >> "$ACTUAL"; echo "FAIL $id: $reasons" ;;
  esac
  probs=$(problems_of "$id")
  if [ "$word" != SKIP ] && { [ -n "$probs" ] || { [ "$code" != 0 ] && [ "$word" = PASS ]; }; }; then
    echo "$LABEL::$id:log" >> "$ACTUAL"
    [ -n "$probs" ] || probs="exit $code after the result"
    echo "LOG  $id: $(head -n 1 <<< "$probs")"
    awk -v id="$id" '{ print id "\t" $0 }' <<< "$probs" >> "$WORK/problems.txt"
  fi
done
sort -o "$ACTUAL" "$ACTUAL"
sort -o "$WORK/obs.jsonl" "$WORK/obs.jsonl"

ms=$(for id in "${SEL[@]}"; do cat "$WORK/cases/$id/ms" 2>/dev/null; done | sort -n)
med=$(sed -n "$(( ($(wc -l <<< "$ms") + 1) / 2 ))p" <<< "$ms")
max=$(tail -n 1 <<< "$ms")
timing="per case ${med:-?} ms median, ${max:-?} ms max"

if [ -n "$PRINT" ]; then
  sed -n "s/^{\"id\":\"$PRINT\",\"snaps\":\(.*\)}$/\"expect\": \1/p" "$WORK/obs.jsonl"
  cat "$WORK/verdicts.txt"
  exit 0
fi

if [ "$MODE" = record ]; then
  dest=${OUT:-$SUITE/vanilla}
  mkdir -p "$dest"
  jsonl="$dest/$LABEL.jsonl"; vlog="$dest/$LABEL.log"
  header=$(grep -h -m1 '^SUITE-HEADER ' "$WORK"/cases/*/log | sed 's/^SUITE-HEADER //' | head -n 1)
  [ -n "$header" ] || die "no case printed a header"
  : > "$WORK/rec.jsonl"; : > "$WORK/rec.log"; : > "$WORK/rec.ids"
  for id in "${SEL[@]}"; do
    line=$(grep -h "^SUITE-REC {\"id\":\"$id\"," "$WORK/cases/$id/log" | sed 's/^SUITE-REC //' || true)
    [ -n "$line" ] || continue
    echo "$line" >> "$WORK/rec.jsonl"
    echo "{\"id\":\"$id\"," >> "$WORK/rec.ids"
    problems_of "$id" | sed '/^$/d' | sed "s|^|$id |" >> "$WORK/rec.log"
  done
  # Lines of cases not recorded now stay, unless the header changed, which
  # makes every old line stale.
  if [ -f "$jsonl" ] && [ "$(head -n 1 "$jsonl")" = "$header" ]; then
    tail -n +2 "$jsonl" | grep -vF -f "$WORK/rec.ids" >> "$WORK/rec.jsonl" || true
    [ ! -f "$vlog" ] || awk 'FILENAME == ARGV[1] { sub(/^\{"id":"/, ""); sub(/",$/, ""); skip[$0] = 1; next } !($1 in skip)' \
      "$WORK/rec.ids" "$vlog" >> "$WORK/rec.log"
  fi
  { echo "$header"; sort "$WORK/rec.jsonl"; } > "$jsonl"
  sort "$WORK/rec.log" > "$vlog"
  [ -s "$vlog" ] || rm -f "$vlog"
  echo "recorded $(grep -c . "$WORK/rec.ids" || true) of ${#SEL[@]} cases for $LABEL into $jsonl in ${ELAPSED} s ($timing)"
  exit "$recfail"
fi

echo "${#SEL[@]} cases in ${ELAPSED} s: $pass pass, $fail fail, $skip skipped ($timing)"

# The same problem line in several cases usually means one cause.
if [ -s "$WORK/problems.txt" ]; then
  cut -f2- "$WORK/problems.txt" | sort | uniq -c | sort -rn | awk '$1 > 1' | while read -r n line; do
    echo "the same line in $n cases: $line"
  done
fi

# Pin lines: "<godot>::<case>[:log] [digest]   # reason".
pins_of() {
  [ -f "$PINS" ] || return 0
  awk -v l="$LABEL::" '!/^#/ { sub(/[[:space:]]*#.*/, ""); if (NF && index($1, l) == 1) print (NF > 1 ? $1 " " $2 : $1) }' "$PINS"
}
reason_of() {
  [ -f "$PINS" ] || return 0
  awk -v k="$1" '!/^#/ && $1 == k { if (sub(/^[^#]*#[[:space:]]*/, "")) print; exit }' "$PINS"
}

broken=$(grep -E '::(canary-[a-z0-9-]+|setup-undo-floor)( |:|$)' "$ACTUAL" || true)
if [ -n "$broken" ]; then
  echo "HARNESS BROKEN on $LABEL: $(awk '{ sub(/^[^:]*::/, "", $1); printf "%s ", $1 }' <<< "$broken")failed."
  echo "The other failures may be the harness's, not godot-vim's:"
  grep -vxF -f <(printf '%s\n' "$broken") "$ACTUAL" | sed 's/^/  /' || true
  annotate error "HARNESS BROKEN on $LABEL"
fi

if [ "$BLESS" = 1 ]; then
  touch "$PINS"
  {
    grep '^#' "$PINS" || true
    awk -v l="$LABEL::" '!/^#/ && NF && index($1, l) != 1' "$PINS"
    while read -r key digest; do
      old=$(reason_of "$key")
      printf '%s%s   # %s\n' "$key" "${digest:+ $digest}" "${old:-TODO}"
    done < "$ACTUAL"
  } > "$WORK/pins.new"
  { grep '^#' "$WORK/pins.new" || true; grep -v '^#' "$WORK/pins.new" | LC_ALL=C sort || true; } > "$PINS"
  echo "blessed $(grep -c . "$ACTUAL" || true) pins for $LABEL; write a reason over every TODO"
  exit 0
fi

# Compare with the pins for this Godot and the selected cases.
EXPECTED="$WORK/pinned.txt"
printf '%s\n' "${SEL[@]}" > "$WORK/selected.txt"
pins_of | awk 'FILENAME == ARGV[1] { sel[$0] = 1; next }
  { id = $1; sub(/^[^:]*::/, "", id); sub(/:log$/, "", id); if (id in sel) print }' "$WORK/selected.txt" - \
  | sort > "$EXPECTED"
changed=0
todo=$(awk -v l="$LABEL::" '!/^#/ && NF && index($1, l) == 1 {
  r = $0; if (!sub(/^[^#]*#[[:space:]]*/, "", r) || r == "" || r ~ /^TODO/) print }' "${PINS}" 2>/dev/null || true)
if [ -n "$todo" ]; then
  changed=1
  echo "Pins without a reason (write one after #):"
  printf '  %s\n' "${todo//$'\n'/$'\n'  }"
fi
report=""; fixes=""
while read -r key digest; do
  [ -n "$key" ] || continue
  want=$(awk -v k="$key" '$1 == k' "$EXPECTED")
  if [ -z "$want" ]; then
    report+="NEW FAILURE        $key${digest:+ $digest}"$'\n'
  elif [ "$want" != "$key${digest:+ $digest}" ]; then
    report+="CHANGED FAILURE    $key ${want#* } -> $digest"$'\n'
  else
    continue
  fi
  fixes+="  $key${digest:+ $digest}   # <reason>"$'\n'
done < "$ACTUAL"
while read -r key digest; do
  [ -n "$key" ] || continue
  awk -v k="$key" '$1 == k { f = 1 } END { exit !f }' "$ACTUAL" && continue
  id=${key#*::}; id=${id%:log}
  if grep -qx "$id" "$SKIPPED"; then
    report+="PINNED BUT SKIPPED $key"$'\n'
  else
    report+="NOW PASSING        $key"$'\n'
  fi
done < "$EXPECTED"
if [ -n "$report" ]; then
  changed=1
  echo "The failing set changed for $LABEL."
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    echo "  $line"
    annotate error "$line"
  done <<< "$report"
  if [ -n "$fixes" ]; then
    echo "Lines for known-failures.txt (add, or replace the old line):"
    printf '%s' "$fixes"
  fi
  echo "A NEW FAILURE is a regression: fix it, or pin it in the same change with its issue."
  echo "NOW PASSING means a known defect got fixed: confirm it, then delete the line."
  echo "Logs and snapshots: $WORK/cases/<id>/log and $WORK/obs.jsonl"
elif [ "$changed" = 0 ]; then
  echo "OK: $(grep -c . "$ACTUAL" || true) known failures for $LABEL, as pinned."
fi
exit "$changed"
