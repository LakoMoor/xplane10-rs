#!/usr/bin/env bash
# shot.sh <name> [cockpit|menubar|loading] — render the port's parity frame (the oracle's own chase camera and aircraft
# pose, 1024x768) and save it as docs/parity/<name>.png, printing how far it is from the
# oracle frame tests/golden/original/frames/chase_default.png.
set -euo pipefail
: "${XP_INSTALL:?set XP_INSTALL}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
G="$ROOT/tests/golden/original/frames"
OUT="$ROOT/docs/parity"; mkdir -p "$OUT"
TMP="$(mktemp -d)"
VIEW="chase_default"; EXTRA=""
if [ "${2:-}" = "cockpit" ]; then VIEW="cockpit_default"; EXTRA="--panel"; fi
if [ "${2:-}" = "menubar" ]; then VIEW="cockpit_menubar"; EXTRA="--panel --menubar"; fi
if [ "${2:-}" = "loading" ]; then
  (cd "$ROOT" && cargo run -q -p xp -- --headless "$TMP/f.ppm" --loading --size 1024x768 >/dev/null)
  python3 -I "$ROOT/tests/original_patch/ppm_diff.py" "$TMP/f.ppm" "$G/loading.png" "$OUT/$1.png"
  rm -rf "$TMP"; exit 0
fi
(cd "$ROOT" && cargo run -q -p xp -- --headless "$TMP/f.ppm" --airport KSEA --runway 34R \
   --oracle-probe "$G/$VIEW.probe.txt" --size 1024x768 --renderer forward $EXTRA | grep -E "objects|draw calls|textures|coverage|panel")
python3 -I "$ROOT/tests/original_patch/ppm_diff.py" "$TMP/f.ppm" "$G/$VIEW.png" "$OUT/$1.png"
rm -rf "$TMP"
