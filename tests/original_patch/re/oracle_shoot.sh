#!/bin/bash
# shoot.sh <name> [env assignments...] — run the original with the probe, keep its screenshot
W="${RE_WORK:?set RE_WORK}"; XPI="${XP_INSTALL:?set XP_INSTALL}"; name=$1; shift
cd "$XPI"; rm -f Output/*_1.png
env XP_PROBE_OUT=$W/oracle/$name.probe.txt "$@" arch -x86_64 "$XPI/X-Plane.app/Contents/MacOS/X-Plane" --no_sound --no_joystick --force_run --pref:_show_qfl_on_start=0 --qa_script=$W/oracle/qa_default.txt > $W/oracle/$name.stdout 2>&1 &
pid=$!; for i in $(seq 1 160); do perl -e 'select(undef,undef,undef,1)'; kill -0 $pid 2>/dev/null || break; done
kill -0 $pid 2>/dev/null && kill $pid
f=$(ls Output/*_1.png 2>/dev/null | head -1); [ -n "$f" ] && mv "$f" $W/oracle/$name.png && echo "$name: ok (${i}s)" || echo "$name: no screenshot (${i}s)"
