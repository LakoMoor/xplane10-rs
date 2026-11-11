# PORT.md — X-Plane 10.51r2 → Rust (+ wgpu)

## 1. Source
- Origin: **no source**. Shipped install `X-Plane 10/` (macOS Mach-O x86_64 oracle
  `6b0ce1e5…`, 10.51r2 / build 105101) plus its content. Pinned in `reference.lock.json`.
- Language of the original: C++ (Mach-O symbol names recovered from crash reports and
  `strings`, e.g. `init_sim(int, char const* const*)`, `WORLD_scene_per_frame`,
  `OGL_terrain_grid`, `REN_collector::draw`).
- Platform target for the port: **macOS desktop** first (the host we can measure on).
  Linux/Windows are reachable through wgpu but unmeasured here. Web/wasm: not requested.
- Fidelity goal: **behavior-exact with measured tolerances**, aiming at 1:1. Parity is only
  claimed where a parity run exists; see `docs/COMPATIBILITY.md`.
- License: X-Plane is proprietary freeware/shareware content. The port distributes code and
  small synthetic/extracted fixtures, **not** the shipped data. RE was done on a copy of the
  user's own install for interoperability.

## 2. Scope
- In scope: the content formats the original actually loads (ACF, OBJ8, apt.dat, DSF,
  library.txt), the rendering structure the original uses (deferred GLSL 1.20 pipeline), and
  the flight model behind them.
- Out of scope for now, with reasons:
  - **Full world terrain** — `Global Scenery/` contains only a README; the continent packs are
    DLC that is not in this install. The original itself reports "no scenery installed".
  - **Sound, plugins/XPLM, multiplayer, VR, panel instruments beyond the draw path** — deferred
    to `docs/ROADMAP.md`; none are needed by the selected vertical slice.
  - **Windows oracle** — the pinned `X-Plane.exe` was deleted by an earlier session and cannot
    be recovered here; only the macOS build is measurable.

## 3. Module map (original behavior → Rust)

| Original area | Evidence | Rust module | Strategy | Status |
|---|---|---|---|---|
| ACF aircraft description | E-0009/10/11 + shipped file | `xp_core::acf` | rewrite | **done, golden green** |
| OBJ8 objects | measured from `fuse.obj` + 3 195 more | `xp_core::obj8` | rewrite | **done, golden green** |
| apt.dat airports | measured from all 7 shipped files | `xp_core::aptdat` | rewrite | **done, golden green** |
| DSF terrain patches | magic + chunk framing + MD5 trailer measured on all 1 735 files | `xp_core::dsf` | rewrite | **framing done, golden green; chunk semantics unrecovered** |
| GPS airport database (`GarminUnits` ctor, `vec_rwy_struct::build_cache`) | oracle disassembly + `I/GPS: Added 19186 airports` | `xp_core::gpsdb` | rewrite | **done, differential green (19 186 = 19 186)** |
| local frame + runway layout (`UTL_geoid`, `build_rwy_layout`, `get_rwy_end`) | oracle disassembly + Ghidra decompile | `xp_core::geoid`, `xp_core::airport` | rewrite | **done; consistency-tested, no oracle output to diff against yet** |
| library.txt / scenery pack graph | shipped files + oracle `E/SCN` lines | `xp_core::scenery` | rewrite | **done, golden green** |
| content checker | oracle `E/SCN`/`W/OBJ` diagnostics | `xp_core::content` | rewrite | **done, golden green; 0 unparseable files** |
| OBJ8 → GPU buffers | measured draw-descriptor semantics | `xp_core::scene` | rewrite | **done, golden green** |
| DDS header / mip correction; PNG decode | all 1 901 `.dds` and all 6 862 `.png` measured | `xp_core::tex`, `xp_core::png` | rewrite (PNG inflate hand-written: no inflate crate is vendored, and adding one is a new dependency) | **done, golden green** |
| coordinate/units conversion | ACF geometry + oracle `I/FLT` lines | `xp_core::geo` | rewrite | planned — axis convention only partly pinned (Cycle Dump legend) |
| flight model | oracle `Cycle Dump.txt`, ACF tables, `Airfoils/` | `xp_core::flight` | rewrite | **started — wing element build-up reproduces the oracle's report (94 lines, verbatim)** |
| deferred renderer | `Resources/shaders/*.glsl` (43 entries, 52 files incl. `dev/`), Log.txt GL caps | `xp::gbuffer`, `xp::passes`, `crates/xp/shaders/*.wgsl` | rewrite (GLSL→WGSL) | **in progress — 5-attachment g-buffer prepass + sun pass ported and drawing real shipped geometry** |
| texture loading | 3 080 OBJ8 `TEXTURE` lines vs 8 763 shipped files | `xp::textures` | rewrite | **done — resolves, decodes and uploads DDS + all 8 shipped PNG layouts** |
| mesh upload | `xp_core::scene` buffers | `xp::mesh_gpu` | rewrite | **done, wired into the frame** |
| forward renderer (HDR off, the original's default) | `terrain_frag.glsl` `MODE_OUTPUT_NORMAL` arm + oracle `_skyc` light state | `xp::forward`, `crates/xp/shaders/terrain_forward.wgsl` | rewrite (GLSL→WGSL) | **in progress — per-pixel sun + ambient ported; scattering, shadows, specular, fog, sky dome not** |
| window/input | Log.txt, `WIN_run_app` | `xp::platform` (winit behind a trait) | rewrite | planned |

Order is leaves-first: formats and geometry before anything touches a window.

## 4. Dependency map

| Original | Rust | Why / risk |
|---|---|---|
| OpenGL 2.1 + GLSL 1.20 | `wgpu 30` + WGSL | The original runs on Apple's legacy GL-on-Metal. NDC z, sRGB and Y-origin differ; see §5. |
| Cocoa/Carbon window + HID (`WIN_run_app`, `D/HID`) | `winit 0.30` behind `xp::platform` | Core must stay window-free. |
| `libsteam_api.dylib` | our shim (`tests/original_patch/stub`) | Only to make the **oracle** boot; the port has no Steam dependency. |
| boost::serialization (AIRNAV db) | plain Rust readers | The oracle builds a 19 186-airport GPS database from apt.dat; we rebuild it from the same text. |

## 5. Hard problems

| Problem | Chosen approach | Notes |
|---|---|---|
| No source, so behavior must be measured | Oracle harness (`docs/ORACLE.md`) + shipped-data goldens | Parity claims are bounded by what the harness covers. |
| Original renders deferred GLSL 1.20 | Port pass-by-pass with `naga`, then hand-fix | 52 shader files; only the passes the slice executes. |
| GL→wgpu semantics | NDC z ∈ [0,1] (`perspective_rh`, never `_gl`), top-left framebuffer origin, `*Srgb` formats, baked pipelines, `bytemuck` uniform layout with explicit vec3→16 padding | The #1 visual bug class. |
| ACF/DSF order and float determinism | Order-preserving `Vec` + `BTreeMap`, never `HashMap` iteration; ACF numbers parsed as **f32** because the original reads them with `scanf("%f")` | `_z_ref 3.299999952` is `3.3_f32`. |
| Modal alerts block headless runs | Avoid them: `--qa_script` for self-termination, non-C172SP aircraft for the lesson alert | Input injection is denied by macOS here. |
| `unsafe` / `unwrap` / globals | None used; I/O returns `anyhow::Result` with file+line context | Deviations would be logged here. |

## 6. Golden tests

| Case | How the original produces it | Output | Tolerance | Port status |
|---|---|---|---|---|
| `acf/c172sp_summary.txt` | shipped `Cessna_172SP.acf` (sha `887e69ae…`) | 50 445 `P` lines, 1 435 key shapes, 1 136 `acf/*` leaves, 76/77 panel instances, value classes int=8589 float=41592 string=264, 39 verbatim scalars | exact text / exact f32 | **green** |
| `original/version.txt` | `X-Plane --version` | `X-Plane 10.51r2` | exact | captured |
| `original/help.txt` | `X-Plane --help` | 118 lines | exact | captured |
| `original/boot_trace.txt` | scripted `--qa_script` run | 592 normalized Log.txt lines incl. ACF load, 19 186-airport GPS db, `apt:KSEA rwy:34R`, AI spawns, 26 render-setting defaults, ATC trace | exact lines; sim-clock values not yet compared | captured, not yet diffed |
| `obj8/fuse_summary.txt` | shipped `fuse.obj` (sha `34f1019b…`) + `ConCSlantRoof.obj` (sha `ae3c8e8e…`) | 2 850 vertices, 10 944 indices = 3 648 triangles, 3 TRIS descriptors summing to 10 944, index range 0…2849, verbatim VT/textures; plus a complete CRLF+tab object (66 verts, 120 indices, 2 descriptors) | exact | **green** |
| `obj8` corpus sweep | every `.obj` under Aircraft/Custom Scenery/Resources | 3 196 OBJ8 files frame with zero failures; 85 Wavefront files classified as a different format, not misparsed; unmodeled directives tallied | zero framing failures | **green** |
| `aptdat/*` | shipped KSEA `apt.dat` (v1050) + default `apt.dat` (v810) | record-type counts, runway/taxi fields | exact | pending |
| `gpsdb/c172sp_summary.txt` | oracle Log.txt `I/GPS: Added 19186 airports` + `scenery packages (prioritized in this order)`; intermediates from an independent Python walk (`tests/original_patch/re/gps_reference.py`) | airports added to the GPS database, 24-pack order, f32 bit patterns of four 700 ft runways | exact | **green — differential against the oracle** |
| `dsf/corpus_summary.txt` | every `.dsf` in the install | 1 735 framed, 12 recognized as 7z-compressed rather than corrupt, MD5 trailer verified against RFC 1321 vectors | exact | **green** |
| `content/scan_summary.txt` | whole-install sweep by `cargo run -p xp-core --example xp-content` | 40 text ACF + 19 legacy binary ACF, 3 197 OBJ8, 1 735 DSF, 18 `library.txt`, 254 packs, **0 unparseable**, 277 errors all in the original's own missing-resource wording, 3 130 warnings, 31 info | exact counts | **green** |
| `scene/*` | shipped `fuse.obj`, `radar.obj`, `B58_fuselage.obj` + the whole OBJ8 corpus | 48-byte vertices, one deduplicated vertex block per draw, upload offsets, material tint per descriptor, file draw order; corpus 3 197 objects / 3 084 with `POINT_COUNTS` / 8 505 draws / 17 011 777 upload indices / 7 320 005 upload vertices / 5 667 175 triangles / 272 objects needing duplication / 243 931 extra vertices / 125 authored-tint draws | exact | **green** |
| `tex/*` | all 1 901 `.dds` | FourCC beats stale masks, mip counts corrected against the payload, 387 BC1 / 27 BC2 / 1 485 BC3 / 2 RGB24, 357 truncated chains, no DX10 header in any file | exact counts | **green** |
| `png/*` | all 6 862 `.png`, against an independent zlib reference decoder | per-file SHA-256 of the raw sample array plus a digest over all 6 862 rows; all 8 shipped format combinations and every ancillary-chunk case | exact hash per file, exact corpus digest | **green** |
| `frame/headless.txt` | the port's own headless run over `Cessna_172SP/` | 8 objects, 27 draws, 24 539 triangles, 20 406 vertices, 73 617 indices, 7 textures with 0 substitutions and 0 misses, 15 % lit / 81 % near-black | counts and coverage buckets exact; **checksum recorded as a spread** — 2 to 9 texels of 262 144 vary run to run, each ±1 in one channel | **green, self-golden — not parity** |
| `frame/ksea_34R.txt` | the port's own headless run with `--airport KSEA --runway 34R` | 9 objects, 30 draws, 24 545 triangles, 8 textures, 53 % lit | counts and coverage exact; checksum recorded, not pinned | **green, self-golden — not parity** |
| pixel parity | original screenshots | — | — | **not available yet** (see `docs/ORACLE.md` §3) |

## 7. Milestones
- [x] **M0** oracle boots headless and emits goldens; harness committed
- [x] **M1** core formats ported, headless golden tests green — ACF (text v1004 + legacy-binary
  detection), OBJ8 (incl. `LINES` index ranges), apt.dat, DSF framing, scenery/library/.agp and the
  install-wide content checker; **23/23 tests green**, `cargo fmt --check` and
  `cargo clippy --all-targets -- -D warnings` clean, **0 unparseable files** in the install
- [x] **M2** window + wgpu renders first real content through the ported passes — the 5-attachment
  g-buffer prepass and the sun pass in WGSL, drawing 8 shipped C172SP objects with 7 shipped
  textures, headless frame golden committed, windowed Metal present path verified. **46/46 tests
  green**, `cargo fmt --check` and `cargo clippy -p xp-core -p xp --all-targets -- -D warnings`
  clean. Parity against a real frame is still open: no oracle screenshot exists.
- [ ] **M3** feature parity, visual diff within tolerance — first differential against the oracle's
  own output is green: the GPS airport database (`xp_core::gpsdb`, 19 186 = 19 186) and the
  24-pack scenery order. **61/61 tests green** with `XP_INSTALL` set (59 in `xp-core`, 2 in `xp`); the first world frame draws
  (aircraft on KSEA 34R over the airport's runways, `frame/ksea_34R.txt`),
  `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` clean. Pixel parity is still
  open.
- [ ] **M4** perf ≥ original or documented, clippy clean, README

## 8. Decision log
- Format semantics were taken from the shipped files wherever the pre-existing evidence ledger
  disagreed; the contradictions are listed in `docs/COMPATIBILITY.md` rather than hidden.
- Shipped text is read through `xp_core::read_text_lossy` because one `apt.dat` is not valid UTF-8.
- Oracle is the macOS 10.51r2 build under Rosetta; the Windows PE is unrecoverable here.
- Steam is shimmed for the **oracle only**; the port itself has no Steam dependency.
- Key-shape fingerprint rule: split on `/`, then on `,`, and replace every all-digit component
  with `#`. (An earlier digits-only-segment rule gave 4 897; the current rule gives 1 435 and
  is recorded in the golden.)
- Ledger figure E-0009's "913 key shapes / 888 acf leaves" does not match either rule measured
  here; the golden now carries the numbers this parser reproduces, and the discrepancy is left
  in `docs/COMPATIBILITY.md` rather than papered over.
- `acf malformed` was replaced by a measured classification: 19 of the 59 shipped `.acf` files are
  a legacy binary serialization, not damaged text ACFs. They are detected and reported as
  themselves; the port does not model them, and the oracle's behavior on them is unmeasured.
- OBJ8 index validation was corrected after an independent interval-partition pass disagreed with
  the checker on 73 files. The checker was wrong: `LINES` consumes index positions too, and draw
  descriptors may appear out of stream order. Overlapping/gapped ranges are now warnings (13
  files), not errors.
- `library.txt` `EXPORT` is whitespace-separated (space or TAB); a one-field line is recorded as
  damage to that line rather than failing the whole file.
- The oracle ignores `--load_acf`, so M0 goldens cover only the built-in default aircraft. This is
  recorded as an open gap rather than worked around by editing the read-only install.
- The OBJ8 material tint (`ATTR_diffuse_rgb` / `ATTR_diffuse`, 83 lines in 30 files) belongs to the
  **descriptor that follows it** and persists until the next tint or `ATTR_reset`.
  Proven from `B58_fuselage.obj`, where a `TRIS` range is split by the directive and the two
  halves must therefore take different tints. `ATTR_diffuse` is treated as a synonym: same arity,
  same position, and the two spellings never co-occur in one file.
- Untinted geometry is assumed to render **white**, recorded as an inference, not a measurement:
  only 26 of 3 197 objects ever write a tint, `One_Traffic_Cone.obj` gets its entire orange from
  one directive over a shared atlas, and `Cockpit_TextLight.obj` restores white by writing
  `1.000 1.000 1.000` rather than resetting. `Cycle Dump.txt` reports aerodynamics only, so the
  oracle cannot pin this.
- Because the tint rides in the vertex stream and WGSL has no constant vertex attribute, each draw
  gets its **own deduplicated vertex block** rather than sharing one array. Corpus cost: 272 of
  3 084 objects duplicate anything, 243 931 extra vertices on 7 320 005.
- Draw order is **file order**, recovered by recording the source line of each descriptor; the
  `TRIS` and `LINES` lists lose it and `radar.obj` proves the interleaving is real.
- Textures are uploaded **linear, not sRGB**. `srgb.glsl` does the conversion in GLSL behind
  `HAS_SRGB`, `terrain_frag.glsl` does its own gamma `pow`, and the DDS gamma hint is
  `dwCaps & 0x400000` — DDSCAPS_MIPMAP — set in all 1 901 files, so it discriminates nothing.
- Two different placeholders are needed: opaque magenta for `tex_base`, and **all-zero** for
  `tex_bump`, because the shader's `ppn == vec4(0.0)` test is its "no normal map here" signal.
- The corner-eye vectors are filled with tl↔bl and tr↔br swapped relative to the GL names, because
  wgpu's fragment origin is top-left and GL's is bottom-left; getting it backwards mirrors the
  reconstructed world position about the horizontal midline.
- The sun pass draws into an offscreen target that is copied to the surface, rather than being
  retargeted per frame: the copy is same-format, and it gives the headless read-back a `COPY_SRC`.
- PNG inflate is hand-written rather than pulled from a crate, because no inflate implementation is
  present in `Cargo.lock` and adding one is a new dependency. Correctness is pinned by an
  independent reference decoder (Python `zlib`) over all 6 862 shipped PNGs, not by a library.
- Ancillary chunks are skipped by the spec rule "first letter lowercase", which also covers `iTXt`
  (present in two shipped files); unknown *critical* chunks are errors.
- The M2 frame golden is a **self-golden** — a checksum and coverage report, not a parity
  comparison — because macOS denies this session assistive access, so no oracle screenshot can be
  captured. Parity against a real frame is still open.
- The camera is placed in OBJ8 model space with **no axis permutation**, because the shipped Cessna
  meshes do not match the oracle's body-axis legend: `wings.obj` spans 12.06 m along x against a
  published 11.02 m span, `prop.obj` is 1.94 m along x against a 1.90 m propeller, and nose-to-tail
  is 8.7 m along z against a published 8.72 m length. The permutation is unmeasured, so it is not
  invented.
- Lighting uniforms are declared placeholders (`u_gamma`, `u_alpha_cut`, `material_specular`,
  `scene_color`, `fade_dist`) with the value chosen by what keeps the ported code doing what its
  own comments say. `material_specular`'s zero is GL's default; `scene_color`'s zero follows
  "X-Plane sets the global ambience to 0".
- `fade_dist` is not one value in two places. The sun block needs `[0, 1]`, because
  `deferred_frag.glsl:755-757` computes `1 - (1 - clamp(x*z + y, 0, 1))²` and multiplies the lit
  albedo by it: `[0, 0]` is the *fully fogged* end, and it silently produced an all-black frame
  while the g-buffer was correctly lit. The terrain block's `fade_dist` writes the fog level into
  attachment 1 B, where `[0, 0]` is right.
- The PNG inflate keeps the fixed alphabets per block rather than building them once. A dynamic
  block overwrites both alphabets, so a later fixed block must put them back; 529 shipped PNGs
  interleave the two block types and failed until this was fixed. The rewrite also dropped the
  lookup table for a bit-at-a-time canonical walk, which cannot get the bit order wrong.
- The windowed path draws the sun pass **into the surface texture** instead of copying an offscreen
  target into it: a surface texture is `RENDER_ATTACHMENT` only, which makes it an illegal
  destination for `copy_texture_to_texture`. The offscreen target remains for the headless
  read-back, the one place that can grant itself `COPY_SRC`.
- The headless frame is **not bit-stable**, and the golden says so instead of pretending otherwise.
  Six runs gave six checksums; pairwise they differ in 2-9 texels of 262 144, each ±1 in one
  channel. The prepass depth compare is `Less` with writes on, so coplanar triangles resolve to
  whichever rasterizes first and Metal does not specify that. The original GL pipeline has the same
  freedom, so pinning an exact image here would be pinning a driver detail, not the original's
  behavior.
- A `XP_PROBE=1` run reads the g-buffer attachments back and reports coverage per attachment. It
  exists because "the frame is black" is ambiguous between "nothing drew", "nothing lit" and
  "lighting was multiplied away", and the two runs that distinguished them were the fastest part of
  finding the `fade_dist` bug.
- The GPS airport count is a function of the **aircraft**, not of the scenery: `acf/_min_rwy_len`,
  `acf/_apt_req_plane` and `acf/_rwy_req_pave` are the constructor arguments of `GarminUnits`. The
  logged 19 186 is therefore the Cessna's number, and it counts airports the GNS430 navdata did
  *not* already contain.
- `acf/_min_rwy_len` is taken to be feet because the disassembled comparison is against metres and
  only 213.36 m reproduces the oracle (700 m gives 12 922). The f32 widths of the length formula
  are kept exactly: eight shipped strips are surveyed at 700 ft and fall on both sides of the
  threshold by under 2 mm.
- `xp_core::gpsdb` scans apt.dat as bytes and keeps only headers and runways, instead of going
  through `aptdat::AptDat`, which retains every record of a 178 MB file. The whole differential
  runs in about a third of a second.
- REA timed out on the oracle binary, so static recovery here used the binary's own symbol table
  with `llvm-objdump`; the scripts and the address list are in `tests/original_patch/re/`.
- `content_golden.rs` runs **one** install scan shared by its seven tests. Run separately and in
  parallel they peaked at 11.7 GB resident, which is what killed an install-wide `cargo test`;
  shared, the binary peaks at 3.2 GB. The remainder is `aptdat::AptDat` holding every record of the
  178 MB default `apt.dat`, which is still to be slimmed.
- `png_golden`, `scene_golden` and `tex_golden` still **fail** rather than skip when `XP_INSTALL`
  is unset, contrary to the README's promise. Recorded as an open defect, not fixed in this slice.
- The airport frame keeps **one** uniform block by expressing the runways in the aircraft's model
  space on the CPU (`StartPose::to_model`) instead of adding a per-object matrix:
  `PassChain::draw_prepass` writes its uniform buffer through the queue, so every draw of a pass
  sees the last value written, and a second transform would need a buffer per object.
- The local frame is centred on the start point. With the centre on the tile middle 16 km away the
  tangent plane tilts 0.0025 rad against the aircraft's vertical and the far end of 34R/16L rises
  3 m; the original's own centre rule is not recovered, so the choice is a placeholder.
- The device now asks for `TEXTURE_COMPRESSION_BC` when the adapter offers it. Every frame before
  this one bound PNG textures only, so the DDS upload path had compiled and never run; its first
  use turned up two validation errors (missing feature, non-physical mip copy extent).
- Ghidra is driven directly (`-noanalysis` + `DecompileFns.java`) because REA's auto-analysis of
  this binary does not finish inside its 330 s limit.
- **An oracle frame exists.** The QA script grammar was recovered by decompilation (`WAIT`, `CMND`,
  `DREF`, …), the original now screenshots and quits by itself, and an in-process probe in the
  Steam shim reads its view and projection matrices. `docs/ORACLE.md` §4.
- Every rendering update is recorded as a port screenshot in `docs/parity/NN_<what>.png`, taken
  with the oracle's own camera and aircraft pose by `tests/original_patch/shot.sh`, with the
  distance to the oracle frame printed beside it. Baseline `01_oracle_camera_and_pose`: mean
  |diff| R 109 G 114 B 121 of 255, 0.2 % of pixels within 16 — geometry lines up, nothing else does.
- The oracle's default frame is the **forward** renderer (HDR off). The port's deferred passes are
  the HDR path, so parity with the default frame needs either `terrain_frag.glsl`'s
  `MODE_OUTPUT_NORMAL` arm ported or the oracle switched to HDR for the comparison. Open decision.
- **Target path decided by the user (2026-11-10): the forward renderer**, i.e. what the original
  shows with default settings. The deferred passes stay as they are and are not the parity target.
- Parity log (`docs/parity/`, port frames taken with the oracle's camera and pose; numbers are mean
  absolute difference per channel of 255 and the share of pixels within 16 of the oracle frame):

  | Update | What changed | R | G | B | within 16 |
  |---|---|---|---|---|---|
  | `01_oracle_camera_and_pose` | oracle camera + aircraft pose, deferred passes, placeholder light | 109.3 | 114.2 | 120.8 | 0.2 % |
  | `02_forward_lighting_oracle_sun` | forward pass, the oracle's sun direction and light colours | 89.8 | 97.9 | 106.8 | 15.7 % |
  | `03_cockpit_panel_background` | **cockpit view** (compared with `cockpit_default.png`): oracle matrices incl. off-centre projection, `Panel.png` at its measured place | 44.1 | 47.2 | 49.8 | 43.5 % |
  | `04_cockpit_instrument_bases` | base bitmaps of the 27 instruments findable by instance name | 44.7 | 47.8 | 50.3 | 43.0 % |
  | `05_loading_background` | **loading screen** (compared with `loading.png`): `intro.png` at (0, −256), one texel per pixel | 25.3 | 17.9 | 14.5 | 32.0 % |
  | `06_cockpit_all_instrument_bases` | cockpit view: all 76 instrument base bitmaps, resolved through the binary's `_ins_path` table | 44.6 | 47.4 | 49.4 | 43.9 % |
  | `07_menu_bar_background` | **menu bar** (compared with `cockpit_menubar.png`): the bar from `interface.png` | 40.8 | 43.1 | 44.7 | 46.3 % |
  | `08_menu_bar_titles` | the nine menu titles through FreeType, blended in linear light | 40.7 | 43.0 | 44.6 | 46.5 % |
  | `10_loading_screen` | loading screen complete: PNG gamma, bands, text, progress bar — **99.0 % of pixels identical** | 1.1 | 1.3 | 1.4 | 98.5 % |
  | `12_cockpit_gamma_menubar` | cockpit + menu bar with the PNG gamma and font gamma tables | 38.6 | 41.0 | 42.6 | 47.6 % |

  The numbers for 01 were taken against the first oracle frame; the committed frame was replaced by
  a later run of the same script that also carries the sky block, so they are indicative.
- The forward variant ported is the plain one (`USE_SCAT = 0`, no shadows). The oracle frame shows
  it is not the whole story: its shadowed pavement is 0.34 of the lit pavement, while ambient-only
  under this formula would be 0.58. Atmospheric scattering is on by default and replaces the
  ambient term (`terrain_frag.glsl:441-455`); that arm is next.
- Directions are carried between frames through the rotation parts of the oracle matrices in f64.
  Differencing two transformed points near 50 000 m in f32 lost the third decimal of a unit vector.
- **Priority set by the user (2026-11-10): the UI must be 1:1 first** — the 2-D cockpit panel, the
  menus, and the loading screen. Sky, scattering and shadows wait; the scattering inputs are already
  probed (`u_od_scale = skyc[+0x10c]·(5.602, 9.4732, 19.6438), skyc[+0x110]`;
  `u_scatter_gain = (1, 1, 13, skyc[+0x100])`, from `OGL_terrain_shader_setup` `0x1001f2280`).
- The original's default view is the 2-D cockpit, not the chase view: `cockpit_default.png` is what
  a user sees at launch, and it is now the main comparison frame (`shot.sh <name> cockpit`).
- 2-D panel placement is FITTED to that frame (`xp_core::panel`): bitmap top-left at (0, −100),
  `POS y = 0` at screen y = 664, one texel per pixel, tint (0.902, 0.891, 0.876). The scroll and
  panel-lighting rules behind those numbers are not ported.
- Instrument 04 dropped the "within 16" share slightly (43.5 → 43.0 %): a dial face drawn without
  its needle, glass and lighting is further from the oracle's dial than bare panel was. Expected
  until the layers are in.
- Open for the UI: the binary's instrument table (49 of 76 Cessna instruments are not named after
  their instance), needles and their datarefs, instrument lighting, the menu bar and dialogs, and
  the loading screen. No oracle frame of a menu or of the loading screen exists yet.
- **Loading screen oracle frame.** The QA script only runs after loading, so the loading screen is
  captured from outside: `tests/original_patch/winid.swift` finds the original's window and
  `screencapture -l <id>` grabs that window alone, once a second during startup. The capture is
  2048×1592 (a 56-row title bar plus 1024×768 at 2×); `loading.png` is its content box-filtered to
  1024×768. It shows `intro.png`, the line `X-Plane 10.51 (64-bit)`, three text bands, a status line
  (`Loading bitmap …`) and a progress bar.
- The background picture aligns at exactly (0, 0) shift, but its colours do not match: the capture
  is brighter by a consistent power of about 0.816 (≈ 1.8 / 2.2) across dark and light samples.
  Either the original re-gammas PNGs on load or the window capture went through display colour
  management; the two are not separated yet, so **no correction is applied** and the number above
  is mostly that difference. An in-sim screenshot of a PNG-textured surface would settle it.
- **Instrument bitmaps come from the original's own table.** `ins_paths(int, …)` (`0x1000693d0`)
  builds `"cockpit/" + ins_path[ins_type]`; `_ins_path` and `_ins_enum` (916 entries each) are
  dumped into `crates/xp-core/data/ins_table.tsv` by `tests/original_patch/re/dumptab.py`. Check:
  the enum name of every one of the Cessna's 76 `PANEL_2D` types equals the name its ACF instance
  carries, and all 76 bitmaps exist on disk (the table's spelling differs from the files' in case
  only). This replaces the lookup by instance name, which found 27.
- What a dial still lacks is per-instrument: the `-1`…`-4` layer bitmaps (needles, cards, glass,
  lit variants), their rotation and translation laws and the datarefs behind them, digit and text
  displays, and the GPS screen. Each instrument type has its own draw routine in the original; none
  is ported.
- **Menu bar: the first UI element that matches the oracle pixel for pixel.** Over the bar's
  18 rows × 1024 columns, 92.2 % of pixels are identical to `cockpit_menubar.png` and 98.7 % are
  within 2 of 255 (mean 0.16, worst 33). What it took:
  - *An oracle frame of a menu.* The bar only shows under the mouse, and input injection is denied.
    `mbar_class::mbar_draw_menu_bar` (`0x100df9ab0`) draws when the pointer y at `_mnw + 0x84` is
    above screen height − 18, so the in-process probe writes that float (`XP_PROBE_MENU=bar`) and
    the original draws its own bar into its own screenshot.
  - *Background*: the cell at texels x 1…100, y 0…17 of `interface.png`, untinted, as a three-slice.
  - *Font*: `Resources/fonts/DejaVuSans.ttf`, FreeType, `FT_Set_Char_Size(size·64, 72 dpi)`,
    `FT_Load_Char(…, FT_LOAD_TARGET_LIGHT)` — the calls are the original's; the size 10.5 is fitted.
  - *Blending*: coverage blends white over the bar **in linear light** and is re-encoded (the
    original's sRGB framebuffer). Blending on encoded values is off by up to 8 per texel.
  - *Layout*: title at item x + 10, baseline on row 13, next item at previous + hinted advance + 30.
- **`unsafe` and a C dependency, logged as the rules require.** `crates/xp-ft` binds the system
  FreeType through a small C shim (`src/shim.c`) and is the only crate with `unsafe`. Reason: the
  original's glyphs come from FreeType's light auto-hinter and no Rust rasterizer in reach has it —
  an unhinted one is 40 % of the ink away on "About", FreeType with the original's calls is 0.25 of
  255 away. Risk: the system library (2.14.3 here) is newer than the embedded one, and that is
  where the remaining worst-case texels (up to 33) are expected to come from. Building now needs
  `pkg-config freetype2`.
- Menu titles are a constant list read off the oracle frame; drop-down menus, hover highlight and
  the original's own menu tables are not ported. `XP_PROBE_MENU=<n>` (open menu `n`) is written but
  untested.
- **PNG gamma is the original's, read from its loader.** `TEX_load_png` calls
  `png_set_gamma(screen, file)` with the file's `gAMA`, or with 1/1.8 (the double at
  `0x1010b5718`) when the file has none. `intro.png` and `Panel.png` are untagged and come out
  brightened by the power 0.818; `interface.png` is tagged 0.45454 and comes out unchanged. With
  the table applied the loading picture is **identical** to the oracle over all 532 480 pixels
  above the text. Only interface bitmaps go through it so far (`TextureStore::get_ui`); whether
  OBJ8 textures do is not established.
- **Font gamma, and a retraction.** Glyph coverage goes through `s_font_gamma[i] =
  (u8)(pow(i/255, 0.8)·255)` (initializer at `0x1000a71f4`, applied in `OGL_font::require_char`)
  and is blended on the encoded values. The earlier entry that explained the menu titles by
  linear-light blending was wrong: it fitted white-on-grey by coincidence and predicted dark text
  on the loading screen the wrong way round. With the table the menu strip went from 92.2 % to
  96.3 % identical.
- **Loading screen geometry is measured, its cells are found.** Bands, rules and text baselines
  are read off the oracle frame (`ui.rs`); the progress bar's two cells were located in
  `interface.png` by compositing every atlas column over the picture and matching the oracle's
  columns. The loading screen's own draw code has not been read.
- **The window runs** (`cargo run -p xp`): loading screen, then the 2-D cockpit with the menu bar
  under the pointer, drawn at 1024×768 and scaled to the window. Its start state is the oracle's
  cockpit probe embedded as data — every aircraft is shown from the Cessna's pilot eye, standing
  where the oracle's Cessna stood — because neither the view code nor the flight model can yet
  produce one. `[`/`]` cycle aircraft; that is not the original's control. The original opens an
  aircraft chooser at start-up (reported by the user; no oracle frame of it yet, the QA-script
  runs skip it) — not ported.
- **Flight model, first piece** (`xp_core::flight::wing`): strips per wing with
  `area = (c1 + c2)/2 · (semilen/els) · cos(sweep)`, `MAC = 2/3·(c1² + c1c2 + c2²)/(c1 + c2)`,
  incidence per strip, feet → metres. Derived from the report and the ACF and then checked on every
  line: 54 `Element #` lines and 8 wing areas, plus the 32 airfoil lines, match as printed. The
  joined-wing block (aspect ratio, Oswald's efficiency) is located in the decompiled
  `load_veh` (lines 1036–2499 of the Ghidra output) and not ported: the printed aspect ratios
  (9.14, 3.47, 2.66) do not follow from `(2s)²/2S` with the semi-lengths printed beside them.
- **Published** as a public GitHub repository with a Pages site from `docs/`. The Steam shim is
  kept out of it (`.gitignore`).
- **Menus: how the original's work, from its code** (`xp_core::menu`, `ui::drop_down`).
  `mbar_class` was decompiled whole (28 functions). Contents come from
  `Resources/menus/English/X-Plane.txt` (`0 <title>` opens a menu, `<id> <text>` an item, `-` a
  separator or the end); item ids are `menu · 300 + position`; "Plugins" is added at run time.
  Geometry: title = text + 30, 18 high; drop-down 2 below the bar, items 12 high, separators 2,
  width `max(40, 40 + text + key)`; frame `BTN_frame2` 0x14 = atlas cell x 78…103, y 157…183 with
  its shadow. Fills: open title and hovered item palette 0x2d at 25 %; text palette 0x2f at
  (left + 18, bottom − 2); `[key]` right-aligned 10 inside; check marks are `BTN_util_element`
  centred on texel (109, 187 | 206). Behaviour: click a title to open/close, hover another title
  to switch, click an item to run it, click elsewhere to close.
- **What every item does** is now data, not guesswork: `APP_handlemenu`'s control flow gives item →
  window (`start_win(n)`) or item → command index; the command table (`_default_commands`, 1 307
  rows of enum, path, description, default key) turns indices into names (`data/commands.tsv`,
  `data/menu_actions.tsv`, `data/menu_marks.tsv`). Key text is `get_key_for_cmnd` over
  `X-Plane Keys.prf`: modifiers, `-`, key name with a lower-case first letter (`SHIFT-space`).
- **Palette** (`data/ui_palette.tsv`): the original's interface colours, read live from
  `_colr + 0x204`. Entry 0x2b is the blue behind dialogs, (13, 69, 101).
- **Oracle frames of open menus**: the probe writes the open-menu index (`_mbar + 0x18`, guarded by
  the menu count) and the pointer (`0x102f37000/4`); with a menu open the original follows the
  pointer to whichever title it is over, so the pointer is put on the wanted title. About, File,
  Location and Environment were captured; Aircraft crashes the original in
  `MACIBM_post_event_proc` under the probe and is not captured.
- **Result**: bar strip 96.3 % identical and 99.8 % within 4/255; About and File drop-downs
  identical in geometry to the pixel (frame edges, text origin, separators, keys); the body differs
  only by the sky behind its 92 %-opaque frame, which the port does not draw yet — with that
  compensated, 5 of 2 247 pixels of About differ by more than 12.
- **Start-up**: with the user's own preferences the original opens *QuickFlight Setup* at start
  (`_show_qfl_on_start 1`, frame `quickflight_setup.png`); the "Open" dialog is Aircraft > Open
  Aircraft. The port still starts on the Open dialog; QuickFlight Setup is next.
- **Install discovery**: with no `XP_INSTALL`, the app looks beside itself, in the usual folders,
  on mounted volumes, then asks Spotlight for `X-Plane.app`, and remembers the hit in
  `~/Library/Application Support/xplane10-rs/install.txt`.
- **Packaging**: `packaging/make_app.sh` builds `xplane10-rs.app` with its own icon
  (`packaging/make_icon.py`, drawn from scratch) and bundled FreeType/libpng, ad-hoc signed.

