# Static-analysis helpers for the oracle binary

REA (`mcp__rea__*`) cannot serve this 53 MB Mach-O: its provider is Ghidra headless with the
default auto-analysis, re-imported per call, and every call (`search_strings`, `xrefs`,
`procedure_pseudo_code`, `analyze_function` — tried twice on 2026-11-10) ends in
`provider_timeout` at 330 s. The same Ghidra install works when driven directly **without**
auto-analysis, decompiling only the functions asked for; the binary keeps its symbol table
(51 544 symbols, 29 843 in `__text`), so the names are all there.

```sh
G=/opt/homebrew/Cellar/ghidra/12.1.4/libexec/support/analyzeHeadless
# first run: import (about two minutes), then decompile the listed entry points
$G <proj dir> xp -import "$B" -noanalysis -scriptPath . \
   -postScript DecompileFns.java <out dir> 0x1006b8c70 0x1000e13d0
# later runs reuse the project
$G <proj dir> xp -process X-Plane -noanalysis -scriptPath . \
   -postScript DecompileFns.java <out dir> 0x1002003a0
```

Decompiler output is a hint about behaviour, read against the disassembly; nothing here is
transliterated from it.

```sh
export XP_INSTALL="/path/to/X-Plane 10"
B="$XP_INSTALL/X-Plane.app/Contents/MacOS/X-Plane"
nm -n "$B" | c++filt > syms.txt                     # symbol map used by the scripts
python3 -I fastref.py "$B" 0x101187840              # who references an address (RIP-relative)
bash dis.sh 0x100292720 0x100295030 gps.asm         # annotated disassembly of a range
python3 -I gps_reference.py "$XP_INSTALL"           # independent reference for tests/golden/gpsdb
```

Addresses are for the pinned build only (`reference.lock.json`, sha `6b0ce1e5…`).

| What | Address |
|---|---|
| `"Added %d airports to the GPS database"` | `0x101187840` (file offset `0x1187840`) |
| `GarminUnits::GarminUnits(...)` — builds the GPS airport list | `0x100292720` |
| its call site in `veh_class::load_veh3` | `0x100224ee5` |
| `AIRAC<Airport>::addExternalDataPoint` — refuses known identifiers | `0x100024760` |
| `vec_rwy_struct::build_cache` — runway length and heading | `0x1006b8c70` |
| `vec_apt_struct::get_longest_rwy_mtrs` | `0x1000e1260` |
| `UTL_geoid::set_geoid_to_planet_earth` — 111 120 m per degree | `0x10031dfe0` |
| `IO_apt(std::string, std::set<std::string>&)` — the apt.dat reader, not yet ported | `0x1006ca340` |
| `IO_apt_all_scenery_packs()` | `0x1006d7e90` |
| `finish_apt`, `IO_apt_fix_broken` | `0x1006c4050`, `0x1006c99f0` |
| `build_rwy_layout(int, int, int, float(*)[3], float, UTL_geoid*)` — runway corners | `0x1000e13d0` |
| `get_rwy_end(int, int, int, float*, float*, float*, UTL_geoid*)` — end midpoint at airport elevation | `0x1000e17f0` |
| `UTL_geoid::set_center(float lon, float lat)` — local frame centre | `0x10031e4c0` |
| `start_loc_spec::update_loc_spec_for_type_start()` — start position, decompiled (2 578 lines), not yet ported | `0x1002003a0` |

