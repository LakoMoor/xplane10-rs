#!/bin/bash
# dis.sh <start> <stop> <out> — disassemble one address range of the oracle, annotated with
# symbol and string names. Needs XP_INSTALL and a syms.txt in the current directory:
#   nm -n "$XP_INSTALL/X-Plane.app/Contents/MacOS/X-Plane" | c++filt > syms.txt
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
B="${XP_INSTALL:?set XP_INSTALL}/X-Plane.app/Contents/MacOS/X-Plane"
xcrun llvm-objdump -d --no-show-raw-insn --x86-asm-syntax=intel \
  --start-address="$1" --stop-address="$2" "$B" \
  | python3 -I "$HERE/annot.py" "$B" | tail -n +7 \
  | sed -E 's/ <__Z[^>]*>//; s/<dyld_stub_binder\+0x[0-9a-f]+>//' > "$3"
wc -l "$3"
