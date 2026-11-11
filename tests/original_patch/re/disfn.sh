#!/bin/bash
# disfn.sh <hexaddr> : disassemble one function of the oracle (until the next text symbol)
X="$XP_INSTALL/X-Plane.app/Contents/MacOS/X-Plane"
a=$(printf "%016x" $((16#${1#0x})))
end=$(awk -v a="$a" '$2=="t"||$2=="T"{ if ($1>a) print $1 }' ${RE_WORK:-.}/syms.txt | sort | head -1)
objdump -d --no-show-raw-insn --start-address=0x$a --stop-address=0x$end "$X" | tail -n +7
