# xplane10-rs — X-Plane 10.51r2 in Rust + wgpu

<img src="docs/icon.png" width="96" alt="">

Download the macOS app from [Releases](https://github.com/LakoMoor/xplane10-rs/releases/latest); it finds your X-Plane 10 folder by itself.

A compatible runtime for the shipped **X-Plane 10.51r2** (build 105101): it loads the
original's own content and implements the original's own measured rules. There is no source
for X-Plane, so every behavior here was recovered by reading the shipped files and by running
the shipped binary under Rosetta 2. See [docs/ORACLE.md](docs/ORACLE.md).

## Status

Early development; nothing flies yet. The window shows the loading screen and then the default
aircraft's 2-D cockpit panel on runway 34R at KSEA; `[` and `]` step through the installed
aircraft (a stand-in for the original's chooser dialog).

Every menu-bar item works: the 13 views and camera moves, screenshot (`Output/<aircraft>_<n>.png`),
quit, toggles with their check marks, and the original's key bindings from `X-Plane Keys.prf`.
Items that open one of the original's windows open it with the original's frame and title; the
windows' controls are not ported yet. `--script "close;item:1504;shot:out.png;quit"` drives the
window for parity screenshots.

Self-assessed, weighted by subsystem: about **18 % implemented, 9 % verified against the
original**. The loading screen is 99.0 % pixel-identical to the original's, the menu bar 96.3 %.
See the project site (`docs/index.html`), [docs/PORT.md](docs/PORT.md) for the plan and decision
log, and [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md) for what is implemented, wired,
content-checked and compared — four different things.

```sh
export XP_INSTALL="/path/to/X-Plane 10"
cargo run -p xp                       # window
cargo run -p xp -- --list-aircraft
cargo run -p xp -- --aircraft "Aircraft/General Aviation/Baron B58/Baron_58.acf"
```

## Build and test

Needs FreeType's development files (`pkg-config freetype2`; `brew install freetype pkg-config` on macOS) for interface text.

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test                       # unit + fixture tests, no install needed
XP_INSTALL="/path/to/X-Plane 10" cargo test   # adds the full-corpus goldens
XP_INSTALL="/path/to/X-Plane 10" cargo run -q -p xp -- --headless frame.ppm   # the M2 frame
XP_INSTALL="/path/to/X-Plane 10" cargo run -q -p xp -- --headless ksea.ppm --airport KSEA --runway 34R
XP_INSTALL="/path/to/X-Plane 10" cargo run -q -p xp -- --airport KSEA --runway 34R   # same, in a window
```

Tests that need the installation are meant to **announce a skip** rather than pass quietly when
it is absent. `png_golden`, `scene_golden` and `tex_golden` do not yet: they fail without
`XP_INSTALL`.

## Regenerating the original-side goldens

Needs macOS with Rosetta 2 and the Steam shim deployed (see `docs/ORACLE.md` §1):

```sh
bash tests/original_patch/stub/build.sh
cp tests/original_patch/stub/libsteam_api.dylib "$XP_INSTALL/Resources/dlls/"
XP_INSTALL="/path/to/X-Plane 10" bash tests/original_patch/run_oracle.sh
```

## Layout

- `crates/xp-core` — deterministic, window-free: formats and rules. No GPU, no winit, no clock.
- `crates/xp` — wgpu/winit front end: the ported g-buffer prepass and sun pass, the mesh upload,
  the texture loader, and the headless frame harness that reads the result back.
- `tests/golden/` — expectations extracted from shipped content and from the running original.
- `tests/original_patch/` — the oracle driver and the Steam shim source, so goldens are reproducible.
- `reference.lock.json` — the exact build and content hashes every claim is pinned to.

## Ground rules

No `unsafe`. No `unwrap()` on I/O or on untrusted content — readers return `anyhow::Result`
naming file and line. File order is preserved everywhere. Floats follow the original's width
(ACF numbers are read as `f32` because the original reads them with `scanf("%f")`).

## What is not in this repository

- No X-Plane content. You need your own X-Plane 10.51 installation.
- The Steam shim used to boot the original for oracle runs (`tests/original_patch/stub/`) is not
  published; `docs/ORACLE.md` describes what it has to do.
- No licence has been chosen yet.

Independent project, not affiliated with or endorsed by Laminar Research.
