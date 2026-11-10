#!/usr/bin/env bash
# Regenerate the original-side goldens in tests/golden/original/.
#
# Requirements: macOS Apple Silicon with Rosetta 2 (the oracle binary is x86_64 Mach-O),
# and the Steam shim from stub/build.sh deployed at Resources/dlls/libsteam_api.dylib.
# Without the shim the original aborts at hl_init.cpp:1420; with a shim that answers the
# subscription check FALSE it raises a modal "Steam app is not subscribed" fatal error.
#
# Usage: XP_INSTALL="/path/to/X-Plane 10" ./run_oracle.sh
set -eu
: "${XP_INSTALL:?set XP_INSTALL to the X-Plane 10 install root}"
BIN="$XP_INSTALL/X-Plane.app/Contents/MacOS/X-Plane"
GOLDEN="$(cd "$(dirname "$0")/../golden/original" && pwd)"
HERE="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$GOLDEN"
[ -x "$BIN" ] || { echo "oracle binary missing: $BIN" >&2; exit 1; }

# 1. CLI goldens
arch -x86_64 "$BIN" --version            > "$GOLDEN/version.txt" 2>&1
arch -x86_64 "$BIN" --help               > "$GOLDEN/help.txt" 2>&1

# 2. Boot + scripted-run golden.  --qa_script makes the original self-terminate instead of
#    sitting in a modal alert; the aircraft must not be the C172SP, whose built-in lesson
#    alert (sim_scen.cpp:495) is modal.
cd "$XP_INSTALL"
rm -f Log.txt
timeout 600 arch -x86_64 "$BIN" \
  --no_sound --no_joystick --force_run \
  --load_acf="Aircraft/General Aviation/StinsonL5/L5.acf" \
  --lat=47.449888889 --lon=-122.311777778 --elev=386 --heading=160 --speed=0 \
  --qa_script="$HERE/qa_default.txt" > "$GOLDEN/run_stdout.txt" 2>&1 || true

# 3. Normalize the log: drop host-specific absolute paths and wall-clock/host identity lines.
sed -e "s|$XP_INSTALL/||g" -e '/^X-Plane started on /d' -e '/^Mac OS X: Version/d' \
    -e '/^CPU type:/d' Log.txt > "$GOLDEN/boot_trace.txt"

echo "goldens written to $GOLDEN"
wc -l "$GOLDEN"/*.txt
