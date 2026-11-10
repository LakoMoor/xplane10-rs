# ORACLE.md — how the original was made observable

Build under test: **X-Plane 10.51r2, build 105101 64-bit, compiled Oct 18 2016**, macOS
Mach-O x86_64, sha256 `6b0ce1e5…` (see `reference.lock.json`). Run on Apple Silicon through
Rosetta 2. Every claim below is from running or reading the shipped artifact, not from the port.

## 1. Making it boot

The shipped binary refuses to start without a working Steamworks layer:

| Gate | Site | Required shim behavior |
|---|---|---|
| Loader | `dlopen("Resources/dlls/libsteam_api.dylib")` then 8 `dlsym`s; any NULL → fail path (E-0016) | all 8 symbols must resolve |
| `SteamAPI_RestartAppIfNecessary` | `hl_init.cpp:1401` — TRUE ⇒ fatal "Will restart game through steam now!" (E-0017) | return **FALSE** |
| `SteamAPI_Init` | `hl_init.cpp:1420` — FALSE ⇒ fatal, exit 1 (E-0017) | return **TRUE** |
| Subscription check | `hl_init.cpp:1431` — Log.txt: `Steam app is not subscribed / Steam could not find a valid subscription for this app.` | `SteamApps()` must return an object whose called slot answers **TRUE** |

`SteamClient()` is then called and `ISteamClient` slot `+0xa8` is invoked with X-Plane's own
`SteamAPIDebugTextHook` as its argument (registers at the fault: `rdi=rax=g_client`,
`r12=<callee>`, `rsi=SteamAPIDebugTextHook`). The shim therefore returns, for every accessor,
a static object whose vtable **and** every flat slot up to `+0x200` are callable no-ops that
answer TRUE — so both `call [obj+off]` and `call [[obj]+off]` resolve.

Shim: `tests/original_patch/stub/stub_v5.c`, built by `stub/build.sh`. The original bundled
dylib is preserved as `Resources/dlls/libsteam_api.dylib.orig` (`9b650776…`); with it the
oracle blocks on `[S_API FAIL] SteamAPI_Init() failed; ipcserver init failed` instead of booting.

Iteration log (three attempts, per the doom-loop guard):

1. `stub_vt` (silent no-ops returning 0) → SIGBUS writing `0x841f0f` at `init_sim+68`.
2. `stub_log` (instrumented, returning 0) → no crash; Log.txt fatal *Steam app is not subscribed*.
   Earlier "hangs" were this fatal error sitting in a **modal alert**, not a hang.
3. `stub_v3`/`stub_v5` (subscription answers TRUE) → **boots into the sim**. `stub_v4`
   (silent leaf no-ops returning 1) still faults at `init_sim+68`; the non-leaf 5-argument
   shape is what survives, so v5 keeps that shape and only buffers the log.

## 2. What the oracle can now do

`tests/original_patch/run_oracle.sh` regenerates `tests/golden/original/`:

- `version.txt` — exactly `X-Plane 10.51r2`.
- `help.txt` — 118 lines of the real option list.
- `boot_trace.txt` — 592 normalized Log.txt lines from a scripted run: GPU capability probe
  (OpenGL 2.1 / GLSL 1.20 on Apple M4, 8 texture units, instancing, aniso 16, sRGB FBO),
  plugin load, `D/HID: HID Bridge Running`, `I/ACF: Loading airplane number 0 with
  Aircraft/General Aviation/Cessna 172SP/Cessna_172SP.acf`, `I/GPS: Added 19186 airports to
  the GPS database`, `I/FLT: Init dat_p0 type:loc_tak apt:KSEA rwy:34R`, AI traffic spawns
  with lat/lon/elevation/heading/speed, 26 `I/REN: Setting |… defaulted to 0|1` lines, 425
  `D/ATC:` lines, and a clean `----- X-Plane has shut down -----`.

Two oracle-observed content rules that the port must reproduce:

- `W/OBJ: The object Custom Scenery/KSEA Demo Area/KSEA_objects/ConCSlantRoof.obj has a first
  LOD near distance of 2000.000000 but it should start at 0!`
- `E/SCN: Failed to find resource 'test/AGtrees_test.for', referenced from file
  'Custom Scenery/KSEA Demo Area/KSEA_objects/'.`

## 3. What the oracle cannot do here (coverage limits, stated plainly)

- **No golden screenshots yet.** `--fps_test=n` requires a replay (`--load_smo=X`) and this
  install has none, so it prints its help and exits. The `sim/operation/screenshot` dataref
  set from the QA script produced no file. Only a whole-screen `screencapture` of the running
  original exists, and it is not a stable per-frame golden.
- **No UDP dataref stream.** X-Plane sends on port 49000 only when data-output items are
  selected in preferences; this install has no `Resources/preferences.txt`, and 45 s of
  listening while the sim was rendering produced 0 packets. Sim-state goldens therefore come
  from Log.txt traces for now, not from a per-frame dataref dump.
- **The QA script time base is not characterized.** The script was accepted and the run
  self-terminated, but Log.txt reached sim time `0:13:50` although the script's `quit` line
  was timed `0:00:06.000`, so wall-clock vs sim-clock semantics are unresolved. Do not rely on
  the script for frame-exact determinism until this is pinned.
- **Input injection is unavailable.** macOS denies this process assistive access
  (`osascript … not allowed assistive access. (-25211)`, `not allowed to send keystrokes`), so
  a modal alert cannot be clicked away. Runs must avoid alerts instead — e.g. the C172SP
  built-in lesson alert at `sim_scen.cpp:495` is modal, so the scripted run loads the Stinson L5.
- **No world terrain.** `Global Scenery/` holds only a README, so the original itself reports
  "I found an area with no scenery installed … you will see nothing but water"
  (`REN_region.cpp:1277`). Render comparisons are limited to the demo scenery packs.

## 4. Screenshots and exact camera (added 2026-11-10; supersedes the first bullet of §3)

**The QA script grammar was wrong, which is why no screenshot was ever written.** Decompiling
`test_main_class::test_b_script` (oracle `0x1005d47c0`) shows the script is a list of keyword
lines, not `time command` pairs:

| Line | Meaning |
|---|---|
| `WAIT <seconds>` | resume that many seconds of **sim time** later |
| `NEXT_FRAME` | resume on the next frame |
| `CMND <command>` | `cmnd_class::get_cmnd_by_name` + `single_press` |
| `DREF <dataref> <value>` | write a dataref |
| `LOOK …`, `RELOAD`, `TIMER_START`, `TIMER_STOP` | present, not yet characterized |

With `tests/original_patch/qa_chase.txt` (`WAIT 30`, `CMND sim/view/chase`, `WAIT 5`,
`CMND sim/operation/screenshot`, `WAIT 3`, `CMND sim/operation/quit`) the original:

- writes `Output/Cessna_172SP_1.png`, 1024×768 8-bit RGB;
- **exits by itself** after about 77 s of wall time, Log.txt ending `0:00:38.385 … HID Bridge
  Shutdown` — so the earlier "does not terminate" and "time base unresolved" notes were both the
  bad script;
- runs the default Cessna without the modal lesson alert (`UTL_cmdline::no_alerts` keys on
  `--qa_script`).

Two runs of the same script do not give the same bytes (clouds drift, strobes flash); the frame
is a target for tolerance comparison, not a hash.

**The camera and the aircraft pose of that frame are read exactly.** `stub/probe.c`, built into the
Steam shim and enabled by `XP_PROBE_OUT=<file>`, samples `g_proj`, `g_wrl_mv` and `g_acf_mv`
(`ZOOM_publish_matrices`, `0x10094e1d0`). For the committed frame
(`tests/golden/original/frames/chase_default.{png,probe.txt}`):

- projection: 60° horizontal at 4:3 (46.83° vertical), near 1.0 m, far 92 679 m;
- chase camera: 17.44 m behind the aircraft's model origin, axes equal to the aircraft's;
- aircraft: heading 0.34°, pitch 2.55°, 18 m down runway 34R from its end and 0.8 m off the
  centreline, at about 97 m MSL — 35 m below the airport header's 433 ft, because the runway
  follows the terrain;
- frame centre `(−122.5, 47.0)`: the middle of the 3 × 2 block of DSF tiles the run loads.

**HDR is off by default** (`I/REN: Setting |HDR rendering defaulted to 0`). The frame above is
therefore the original's *forward* path, while the port's two passes are the deferred (HDR) path.
`bash tests/original_patch/shot.sh <name>` renders the port with the oracle's camera and pose,
saves `docs/parity/<name>.png` and prints the per-channel distance to the oracle frame.

## 5. Sky and light state (added 2026-11-10)

The probe also dumps `_skyc` (oracle `0x102b7c540`, 0x180 bytes). The binary names its fields
itself: its static initialiser registers private datarefs against them, e.g. `skyc/sun_dir_r` at
`+0x4c`, `skyc/sun_amb_r` at `+0x2c`, `skyc/fog/near_fog_lit` at `+0x08`, `skyc/fog/far_fog_lit`
at `+0x0c`, `skyc/fog/far_dis_clip` at `+0x1c`, `skyc/vis/tone_map` at `+0x104`; the public
`sim/graphics/scenery/sun_heading_degrees` / `sun_pitch_degrees` getters read `+0x168` / `+0x170`.
For the committed frame: sun heading 235.42°, pitch 34.77°; direct light (0.780, 0.740, 0.719);
ambient (0.192, 0.193, 0.196); fog 5 633 m … 56 327 m; far clip 92 600 m.

Fields `+0x8c…+0xa8` hold two more colours, (0.501, 0.610, 0.712) and (0.281, 0.362, 0.444), that
no registered name covers. The oracle frame's sky near the horizon is (0.577, 0.670, 0.738), so
neither is the sky colour as displayed; they are not used.

