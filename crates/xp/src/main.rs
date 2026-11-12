//! Window-side driver: real shipped content through the ported g-buffer and sun-light passes.
//!
//! `xp_core` parses and assembles; this file decides what to draw and where to put it. Simulation
//! state, the dataref table, and the rest of the original's pass chain (shadows, SSAO, scattering,
//! water, decals, night textures, the post chain, `legacy.glsl`) are not here — see
//! [`docs/PORT.md`](docs/PORT.md).
//!
//! # Two ways to run
//!
//! ```text
//! xp                                        windowed, redraw as fast as the compositor allows
//! xp --headless <out.ppm>                   one 512x512 frame written as binary PPM
//! xp --install <path> --headless <out.ppm>  override XP_INSTALL
//! xp --content <relative-dir>               draw another object set instead of the C172SP
//! ```
//!
//! The headless path exists because the only frame golden this milestone can honestly produce is
//! the port's own. The oracle has never produced a screenshot in this session — macOS denies the
//! assistive access `sim/operation/screenshot` needs when driven from a `--qa_script` — so there is
//! no original image to compare against yet. The headless run prints a checksum and a triangle
//! count for exactly that reason: they are a self-golden, and they must not be read as parity.
//!
//! # What the first frame is, and what it is not
//!
//! It draws every OBJ8 object in `Aircraft/General Aviation/Cessna 172SP/objects` — the aircraft the
//! oracle itself loads — through the ported `MODE_OUTPUT_GBUFFER` prepass and the ported `MODE_SUN`
//! lighting pass, with the install's own textures bound.
//!
//! The camera is fixed and hand-placed. The **axis convention is still not pinned by the oracle**:
//! the shipped Cycle Dump legend says `x location positive aft / y location positive right / z
//! location positive up`, but the Cessna's own geometry says OBJ8 model space is different —
//! `wings.obj` spans 12.06 m along **x** (published span 11.02 m), `prop.obj` spans 1.94 m along
//! **x** (published propeller diameter 1.90 m), and nose to tail measures 8.7 m along **z**
//! (published length 8.72 m). So OBJ8 model space here is lateral-x, vertical-y, longitudinal-z
//! with z increasing aft, and the app permutes it into the body axes the legend describes. The port
//! draws OBJ8 space directly and puts the camera in it, which is the only choice that does not
//! bake in a permutation nobody has measured yet.
//!
//! The lighting uniforms are placeholders. Nothing in the install reports what the original set
//! `gl_LightSource[0]` to, and the oracle never exposed it, so [`sun_state`] says which values are
//! chosen and why rather than implying they were measured.

mod actions;
mod btnui;
mod chooser;
mod forward;
mod gbuffer;
mod mesh_gpu;
mod overlay;
mod passes;
mod special;
mod textures;
mod ui;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context as _, Result};
use gbuffer::{SunLight, SunUniforms, TerrainTextures, TerrainUniforms};
use mesh_gpu::GpuMesh;
use passes::PassChain;
use textures::{Resolution, Resolver, TextureStore};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};
use xp_core::airport::{self, StartPose};
use xp_core::geoid::LocalFrame;
use xp_core::obj8::TextureKind;
use xp_core::oracle::Probe;
use xp_core::scene;

/// Headless frame size. 512 keeps the read-back row stride (512 × 4 = 2 048) a multiple of the 256
/// that `copy_texture_to_buffer` requires, so no padding has to be stripped.
const HEADLESS_EXTENT: (u32, u32) = (512, 512);

// ---------------------------------------------------------------------------------------------
// Matrices.
//
// Column-major, the same convention `gbuffer.rs` documents for the uniform blocks: each inner
// `[f32; 4]` is one column, so `m[c][r]` is row r of column c and `m * v` is the dot of column c
// with v.
// ---------------------------------------------------------------------------------------------

type Mat4 = [[f32; 4]; 4];

const IDENTITY: Mat4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// Sixteen floats in OpenGL column-major order, as columns.
fn flat_to_mat(m: &[f32; 16]) -> Mat4 {
    let mut out = IDENTITY;
    for (c, col) in out.iter_mut().enumerate() {
        col.copy_from_slice(&m[c * 4..c * 4 + 4]);
    }
    out
}

/// An OpenGL projection for wgpu: clip z goes from GL's [−w, w] to [0, w], i.e. the z row becomes
/// (z row + w row) / 2. x and y are untouched, so an off-centre frustum stays off-centre.
fn gl_proj_to_wgpu(m: &[f32; 16]) -> Mat4 {
    let mut out = flat_to_mat(m);
    for col in out.iter_mut() {
        col[2] = (col[2] + col[3]) * 0.5;
    }
    out
}

fn mat_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = IDENTITY;
    for (j, col) in out.iter_mut().enumerate() {
        for (i, v) in col.iter_mut().enumerate() {
            *v = (0..4).map(|k| a[k][i] * b[j][k]).sum();
        }
    }
    out
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize3(a: [f32; 3]) -> Result<[f32; 3]> {
    let len = dot3(a, a).sqrt();
    if len == 0.0 {
        bail!("cannot normalise the zero vector {a:?}");
    }
    Ok([a[0] / len, a[1] / len, a[2] / len])
}

/// A right-handed look-along-negative-Z view matrix, the convention `terrain_vert.glsl` implies:
/// `terrain_frag.glsl:1579` writes `v_position_eye.z / -1024.0` into the g-buffer, and
/// `sun_light.wgsl:158` recovers the eye position with `eye_pos_pixel.y * -1024.0`, so visible
/// geometry must have negative eye-space Z.
fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> Result<Mat4> {
    let forward = normalize3(sub3(target, eye))?;
    let right = normalize3(cross3(forward, up))?;
    let true_up = cross3(right, forward);
    Ok([
        [right[0], true_up[0], -forward[0], 0.0],
        [right[1], true_up[1], -forward[1], 0.0],
        [right[2], true_up[2], -forward[2], 0.0],
        [
            -dot3(right, eye),
            -dot3(true_up, eye),
            dot3(forward, eye),
            1.0,
        ],
    ])
}

/// A right-handed perspective projection with the depth range wgpu's Metal and Vulkan backends
/// expect: eye Z of `-near` maps to device Z 0 and `-far` maps to 1.
///
/// Derived rather than copied, because the port has no math crate: solving
/// `(A·z + B) / (-z) = 0` at `z = -near` and `= 1` at `z = -far` gives `A = far/(near-far)` and
/// `B = near·far/(near-far)`.
fn perspective(fov_y_rad: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fov_y_rad * 0.5).tan();
    let a = far / (near - far);
    let b = near * far / (near - far);
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, a, -1.0],
        [0.0, 0.0, b, 0.0],
    ]
}

// ---------------------------------------------------------------------------------------------
// Camera and lighting state.
// ---------------------------------------------------------------------------------------------

/// Where the frame is seen from, in OBJ8 model space of the aircraft.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Camera {
    eye: [f32; 3],
    target: [f32; 3],
    near: f32,
    far: f32,
    fov_y_rad: f32,
    /// The oracle's own aircraft→eye and projection matrices, when a probe supplied them. They
    /// override `eye`/`target`/`fov_y_rad`: the cockpit view's projection is off-centre, which a
    /// field of view cannot express.
    exact: Option<(Mat4, Mat4)>,
}

impl Camera {
    /// The M2 view of the aircraft alone: off to the -x side, above in +y, toward the nose in z.
    /// The Cessna's geometry spans x ±6.03, y −1.35…1.81, z −1.50…7.10; the target is its middle.
    /// `tests/golden/frame/headless.txt` is pinned to these numbers.
    const AIRCRAFT: Camera = Camera {
        eye: [-13.0, 7.0, 1.0],
        target: [0.0, 0.2, 2.8],
        near: 0.1,
        far: 200.0,
        fov_y_rad: FOV_Y_RAD,
        exact: None,
    };

    /// A chase view for the airport frame: 36 m behind the tail, 10 m up and 9 m to the left,
    /// looking down the runway.
    /// The far plane has to clear a 3.7 km runway. A PLACEHOLDER viewpoint — the original's
    /// external-view camera is not ported.
    const RUNWAY: Camera = Camera {
        eye: [-9.0, 10.0, 36.0],
        target: [0.0, 0.0, -60.0],
        near: 0.5,
        far: 8000.0,
        fov_y_rad: FOV_Y_RAD,
        exact: None,
    };
}

const UP: [f32; 3] = [0.0, 1.0, 0.0];
const FOV_Y_RAD: f32 = 35.0f32.to_radians();

/// The uniform blocks for one frame.
struct FrameUniforms {
    terrain: TerrainUniforms,
    sun: SunUniforms,
}

/// The fixed-function lighting state the port feeds the two passes.
///
/// PLACEHOLDERS, not measurements. The install reads `gl_LightSource[0]` and
/// `gl_FrontLightModelProduct.sceneColor` straight out of GL, and nothing in the shipped files or
/// in any oracle output records what the app put there. Each choice below is the one that keeps
/// the ported code doing what its own comments say it does:
///
/// * `u_gamma = [1.0, 1.0]` — `terrain_gbuffer.wgsl:386` raises the tint to `u_gamma.y`, so 1.0
///   makes it an identity and the OBJ8 `ATTR_diffuse_rgb` reaches the g-buffer unchanged. The
///   original's gamma handling is a later milestone; inventing an exponent here would tint the
///   albedo twice.
/// * `u_alpha_cut = [0.5, 0.5, 0.5]` — the noise-gate model at `:357,371,375` becomes a hard 50 %
///   cut, which is what opaque OBJ8 geometry with an alpha-cut canopy wants.
///   `terrain_frag.glsl:1534` says "Real testing levels will be at zero", so the original's own
///   level is not 0.5 either; 0.5 is the value that makes the gate a gate.
/// * `material_specular = [0, 0, 0, 1]` — GL's default front material. `ATTR_shiny_rat`, which is
///   what would drive it, is parsed but not modelled, so the specular channel of attachment 1 stays
///   0 and the sun pass takes its `exponent = 1.0` arm (`sun_light.wgsl:180`).
/// * `scene_color = [0, 0, 0, 1]` — `terrain_vert.glsl:169` states "X-Plane sets the global
///   ambience to 0", and attachment 3 is the only place it is read.
/// * `fade_dist` is *not* the same value in both blocks. In the terrain block `[0, 0]` collapses
///   the fog term at `terrain_frag.glsl:391`, matching the variant's `FOG_MODE == MODE_FOG_NONE`.
///   In the sun block it must be `[0, 1]`: `deferred_frag.glsl:755-757` computes
///   `plan_rat = 1 - (1 - clamp(fade_dist.x * z + fade_dist.y, 0, 1))²`, so `[0, 0]` is the
///   *fully fogged* end (`plan_rat = 0`) and multiplies the lit albedo away to black, while
///   `[0, 1]` is the identity this slice needs while textured fog is unported.
fn sun_state(view: &Mat4) -> SunLight {
    // A direction toward the sun, in OBJ8 model space: high in +y (up), off to -x, slightly
    // toward the nose. Transformed into eye space because that is where the shader reads it.
    let to_sun = [0.35f32, 0.75, -0.55];
    let eye_dir = [
        view[0][0] * to_sun[0] + view[1][0] * to_sun[1] + view[2][0] * to_sun[2],
        view[0][1] * to_sun[0] + view[1][1] * to_sun[1] + view[2][1] * to_sun[2],
        view[0][2] * to_sun[0] + view[1][2] * to_sun[1] + view[2][2] * to_sun[2],
    ];
    SunLight {
        // w = 0: a directional light, which is what `deferred_frag.glsl:691,696` normalises.
        position: [eye_dir[0], eye_dir[1], eye_dir[2], 0.0],
        diffuse: [1.0, 1.0, 1.0, 1.0],
        ambient: [0.2, 0.2, 0.22, 1.0],
        specular: [0.0, 0.0, 0.0, 1.0],
        scene_color: [0.0, 0.0, 0.0, 1.0],
        material_diffuse: [1.0, 1.0, 1.0, 1.0],
        material_specular: [0.0, 0.0, 0.0, 1.0],
    }
}

/// The per-frame uniform blocks for one extent.
fn frame_uniforms(extent: (u32, u32), camera: &Camera) -> Result<FrameUniforms> {
    let (near, far) = (camera.near, camera.far);
    let view = look_at(camera.eye, camera.target, UP)?;
    let proj = perspective(
        camera.fov_y_rad,
        extent.0 as f32 / extent.1 as f32,
        near,
        far,
    );
    let mvp = mat_mul(&proj, &view);
    let light = sun_state(&view);

    let half_h = far * (camera.fov_y_rad * 0.5).tan();
    let half_w = half_h * (extent.0 as f32 / extent.1 as f32);

    // `corner_eye_*` are the vectors from each frustum corner at the far plane back to the eye —
    // `deferred_common.glsl:6-9` names them `vec_to_me`, and `sun_light.wgsl:124-128` mixes them by
    // screen ratio and scales by z/far. GL's fragment origin is bottom-left; wgpu's is top-left, so
    // the port fills tl↔bl and tr↔br swapped. Getting this backwards mirrors the reconstructed
    // world position about the horizontal midline, which shows up immediately as lighting that
    // flips halfway down the frame.
    let (tl, tr, br, bl) = (
        [half_w, -half_h, far],
        [-half_w, -half_h, far],
        [-half_w, half_h, far],
        [half_w, half_h, far],
    );

    Ok(FrameUniforms {
        terrain: TerrainUniforms {
            u_modelview_matrix: view,
            u_mvp_matrix: mvp,
            // Only the MODE_COMPOSITE arm reads these (`:268-293`), and no OBJ8 draw selects it.
            tex_params: [0.0; 4],
            tex_params2: [0.0; 4],
            tex_params3: [0.0; 4],
            // .x scales the bump-map ST, .y scales the tangent-space deviation; both at full.
            u_bump_scale: [1.0, 1.0],
            fade_dist: [0.0, 0.0],
            u_gamma: [1.0, 1.0],
            _pad0: [0; 8],
            u_alpha_cut: [0.5, 0.5, 0.5],
            texture_mode: gbuffer::MODE_TEXTURE,
            light,
        },
        sun: SunUniforms {
            corner_eye_tl: [tl[0], tl[1], tl[2], 1.0],
            corner_eye_tr: [tr[0], tr[1], tr[2], 1.0],
            corner_eye_br: [br[0], br[1], br[2], 1.0],
            corner_eye_bl: [bl[0], bl[1], bl[2], 1.0],
            far_plane: [far, 1.0 / far, near, 1.0 / near],
            u_screen_size: [
                extent.0 as f32,
                extent.1 as f32,
                1.0 / extent.0 as f32,
                1.0 / extent.1 as f32,
            ],
            // `shadow_level = static_ao * .x + .y` (`:197`); CSM and cloud shadows are not ported,
            // so the master passes the g-buffer's AO straight through.
            shadow_master: [1.0, 0.0, 1.0, 0.0],
            // `[0, 1]`, not `[0, 0]`: see the fade_dist note on `sun_state`.
            fade_dist: [0.0, 1.0],
            _pad0: [0; 8],
            light,
            // The sun pass draws a full-screen quad whose vertices are already in NDC
            // (`passes.rs` QUAD_VERTICES), so it wants the identity, not the camera matrix.
            u_mvp_matrix: IDENTITY,
        },
    })
}

// ---------------------------------------------------------------------------------------------
// Scene.
// ---------------------------------------------------------------------------------------------

/// One OBJ8 object, uploaded, with a terrain bind group per texture slot.
struct SceneObject {
    mesh: GpuMesh,
    /// Indexed by `scene::DrawCall::texture`.
    groups: Vec<wgpu::BindGroup>,
    /// For a draw that names no slot.
    fallback: wgpu::BindGroup,
    triangles: usize,
    /// Lowest vertex of the object in model space; the airport frame rests the gear on it.
    min_y: f32,
    /// Vertex positions kept on the CPU, for resting the aircraft on the ground under a pose
    /// that is not level.
    positions: Vec<[f32; 3]>,
    /// The resolved texture views, one per slot, and the stand-in for a slot-less draw; kept
    /// so a second renderer can bind them without resolving the files again.
    views: Vec<wgpu::TextureView>,
    missing: wgpu::TextureView,
}

/// Load and upload every OBJ8 object under `dir`.
///
/// A file that is not OBJ8 (the install ships 100 Wavefront `.obj` files beside the 3 197 OBJ8
/// ones) or that fails to assemble is reported and skipped: one damaged object must not stop the
/// frame, and the reason it stopped has to be printed rather than swallowed.
fn load_scene(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    chain: &PassChain,
    install: &Path,
    dir: &Path,
    resolver: &mut Resolver,
    store: &mut TextureStore,
) -> Result<Vec<SceneObject>> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let rd = std::fs::read_dir(&d)
            .with_context(|| format!("reading {}", d.display()))?
            .filter_map(|e| e.ok())
            .collect::<Vec<_>>();
        for e in rd {
            let p = e.path();
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                stack.push(p);
            } else if p.extension().and_then(|s| s.to_str()) == Some("obj") {
                files.push(p);
            }
        }
    }
    files.sort();

    let mut objects = Vec::new();
    let mut skipped = Vec::new();

    for path in files {
        let rel = path.strip_prefix(install).unwrap_or(&path);
        let (_obj, mesh) = match scene::load(&path) {
            Ok(v) => v,
            Err(e) => {
                skipped.push(format!("{}: {e}", rel.display()));
                continue;
            }
        };
        let obj_dir = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("{} has no parent directory", path.display()))?;
        objects.push(upload_object(
            device,
            queue,
            chain,
            &mesh,
            obj_dir,
            &rel.display().to_string(),
            resolver,
            store,
        )?);
    }

    if !skipped.is_empty() {
        log::warn!("{} object(s) skipped:", skipped.len());
        for s in skipped.iter().take(20) {
            log::warn!("  {s}");
        }
        if skipped.len() > 20 {
            log::warn!("  … and {} more", skipped.len() - 20);
        }
    }
    Ok(objects)
}

/// Upload one assembled mesh and build a terrain bind group per texture slot.
///
/// `tex_dir` is the directory the mesh's texture paths are relative to: the `.obj` file's own
/// directory for OBJ8, `Resources/bitmaps/runways` for runway pavement.
#[allow(clippy::too_many_arguments)]
fn upload_object(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    chain: &PassChain,
    mesh: &scene::Mesh,
    tex_dir: &Path,
    label: &str,
    resolver: &mut Resolver,
    store: &mut TextureStore,
) -> Result<SceneObject> {
    let zero = store.placeholder_zero().clone();
    // The stand-in for an unresolvable `tex_base`. It must be opaque: `terrain_gbuffer.wgsl:375`
    // discards a fragment whose alpha falls to the cut level, so a transparent stand-in would
    // make the geometry vanish instead of showing the gap.
    let missing = store.placeholder().clone();

    let triangles = mesh.triangle_count();
    let min_y = mesh
        .vertex_bytes
        .chunks_exact(scene::VERTEX_STRIDE)
        .map(|v| f32::from_le_bytes([v[4], v[5], v[6], v[7]]))
        .fold(f32::INFINITY, f32::min);
    let positions: Vec<[f32; 3]> = mesh
        .vertex_bytes
        .chunks_exact(scene::VERTEX_STRIDE)
        .map(|v| {
            [
                f32::from_le_bytes([v[0], v[1], v[2], v[3]]),
                f32::from_le_bytes([v[4], v[5], v[6], v[7]]),
                f32::from_le_bytes([v[8], v[9], v[10], v[11]]),
            ]
        })
        .collect();
    let normal_slot = mesh
        .textures
        .iter()
        .position(|t| t.kind == TextureKind::Normal);
    let mesh =
        GpuMesh::new(device, queue, mesh, label).with_context(|| format!("uploading {label}"))?;

    let mut views: Vec<wgpu::TextureView> = Vec::with_capacity(mesh.textures.len());
    for slot in &mesh.textures {
        let res = resolver
            .resolve(tex_dir, &slot.path)
            .with_context(|| format!("resolving {:?} from {label}", slot.path))?;
        if let Resolution::Ambiguous(p) = &res {
            log::warn!(
                "{label}: {} exists as both .dds and .png; the port takes {} (original choice unmeasured)",
                slot.path,
                p.display()
            );
        }
        views.push(
            store
                .get(device, queue, &res)
                .with_context(|| format!("loading {} from {label}", slot.path))?,
        );
    }
    // The object's own TEXTURE_NORMAL slot, if it has one, is the bump map for every draw of
    // it: OBJ8 does not switch the normal map per descriptor.
    let bump = normal_slot
        .and_then(|i| views.get(i).cloned())
        .unwrap_or_else(|| zero.clone());

    let groups = views
        .iter()
        .map(|base| {
            chain.terrain_bind_group(
                device,
                &TerrainTextures {
                    base,
                    comp: &zero,
                    comp2: &zero,
                    noise: &zero,
                    bump: &bump,
                },
            )
        })
        .collect();
    let fallback = chain.terrain_bind_group(
        device,
        &TerrainTextures {
            base: &missing,
            comp: &zero,
            comp2: &zero,
            noise: &zero,
            bump: &zero,
        },
    );

    Ok(SceneObject {
        mesh,
        groups,
        fallback,
        triangles,
        min_y,
        positions,
        views,
        missing,
    })
}

/// The airport every flight starts at (the oracle's default flight).
pub(crate) const START_AIRPORT: &str = "KSEA";

/// Where the aircraft starts: an airport identifier and a runway end, as the oracle logs them
/// (`Init dat_p0 type:loc_tak apt:KSEA rwy:34R`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Start {
    airport: String,
    runway: String,
}

/// Directory the runway pavement paths of [`airport::pavement_texture`] are relative to.
const RUNWAY_BITMAPS: &str = "Resources/bitmaps/runways";

/// The aircraft, and — when a start location is given — the airport's runways under it.
///
/// Everything is expressed in the aircraft's model space, so one uniform block serves the whole
/// frame: the runway corners come out of the original's local frame
/// ([`xp_core::geoid::runway_layout`]) and are carried into model space on the CPU by
/// [`airport::StartPose::to_model`]. The frame is centred on the start point itself; how the
/// original picks its centre is not recovered, and a centre 16 km away tilts the ground 0.0025 rad
/// against the aircraft's vertical.
#[allow(clippy::too_many_arguments)]
fn build_scene(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    chain: &PassChain,
    install: &Path,
    content: &str,
    start: Option<&Start>,
    probe: Option<&Probe>,
    resolver: &mut Resolver,
    store: &mut TextureStore,
) -> Result<(Vec<SceneObject>, Camera)> {
    let dir = install.join(content);
    // An aircraft need not have OBJ8 objects (many shipped ones are drawn from the ACF's own
    // body geometry, which is not ported). Without a start location there is then nothing to
    // show; with one, the airport and the panel still are.
    let mut objects = match dir.is_dir() {
        true => load_scene(device, queue, chain, install, &dir, resolver, store)?,
        false => Vec::new(),
    };
    if objects.is_empty() {
        if start.is_none() {
            bail!("no OBJ8 objects loaded from {}", dir.display());
        }
        log::warn!("no OBJ8 objects under {}", dir.display());
    }
    let Some(start) = start else {
        return Ok((objects, Camera::AIRCRAFT));
    };

    let (file, apt) = airport::find_in_install(install, &start.airport)?;
    let runways = airport::runways(&apt)?;
    let rwy = runways
        .iter()
        .find(|r| r.name1 == start.runway || r.name2 == start.runway)
        .with_context(|| format!("{} has no runway end {:?}", start.airport, start.runway))?;
    let e = &rwy.extent;
    let (lat, lon) = if rwy.name1 == start.runway {
        (e.lat1, e.lon1)
    } else {
        (e.lat2, e.lon2)
    };
    let (mesh, camera) = match probe {
        None => {
            let frame = LocalFrame::earth(lon as f32, lat as f32);
            let alt = airport::elevation_m(&apt)?;
            // PLACEHOLDER: the gear's rest height comes from the flight model, which is not
            // ported; the lowest vertex of the aircraft's geometry stands on the pavement.
            let ground_y = objects
                .iter()
                .map(|o| o.min_y)
                .fold(f32::INFINITY, f32::min);
            let pose = StartPose::on_runway_end(&frame, rwy, &start.runway, alt, ground_y)?;
            let mesh = airport::build_runway_mesh(
                &frame,
                &runways,
                alt,
                &|p| pose.to_model(p),
                [0.0, 1.0, 0.0],
            )?;
            (mesh, Camera::RUNWAY)
        }
        Some(probe) => {
            // The aircraft's pose and the camera are the ORACLE's, read from its own matrices.
            let (clon, clat) = xp_core::geoid::region_center(lat, lon);
            let frame = LocalFrame::earth(clon, clat);
            // PLACEHOLDER for the terrain mesh: the original lays the runway on the DSF terrain
            // (the oracle's aircraft sits 34 m below the airport's header elevation at 34R).
            // Until DSF elevation is ported the pavement is put at the one altitude that passes
            // through the aircraft's lowest point along the frame's vertical.
            let lowest_world_y = objects
                .iter()
                .flat_map(|o| o.positions.iter())
                .map(|p| probe.model_to_world(*p)[1])
                .fold(f64::INFINITY, f64::min);
            // No geometry to stand on: PLACEHOLDER of 1.35 m under the model origin, the
            // default Cessna's own figure.
            let lowest_world_y = match lowest_world_y.is_finite() {
                true => lowest_world_y,
                false => probe.model_to_world([0.0, -1.35, 0.0])[1],
            };
            let mut alt = airport::elevation_m(&apt)?;
            for _ in 0..3 {
                let y = f64::from(frame.to_local(lat, lon, alt)[1]);
                alt += (lowest_world_y - y) as f32;
            }
            log::info!("pavement altitude fitted under the oracle's aircraft: {alt:.2} m");
            let mesh = airport::build_runway_mesh(
                &frame,
                &runways,
                alt,
                &|p| probe.world_to_model(p),
                probe.world_dir_to_model([0.0, 1.0, 0.0]),
            )?;
            let (near, far) = probe.near_far();
            let camera = Camera {
                eye: probe.eye_in_model(),
                // `g_acf_mv` is a pure translation in the chase view: the eye looks along the
                // aircraft's own −z with the aircraft's own up.
                target: [0.0, 0.0, 0.0],
                near,
                far,
                fov_y_rad: probe.fov_y_rad(),
                exact: Some((flat_to_mat(&probe.acf_mv), gl_proj_to_wgpu(&probe.proj))),
            };
            (mesh, camera)
        }
    };
    log::info!(
        "{} from {}: {} runway(s), start on {} at {lat:.8} {lon:.8}",
        start.airport,
        file.strip_prefix(install).unwrap_or(&file).display(),
        runways.len(),
        start.runway,
    );
    let label = format!("{} runways", start.airport);
    objects.push(upload_object(
        device,
        queue,
        chain,
        &mesh,
        &install.join(RUNWAY_BITMAPS),
        &label,
        resolver,
        store,
    )?);
    Ok((objects, camera))
}

/// Record the two ported passes for one frame into `target`.
fn draw_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    chain: &PassChain,
    objects: &[SceneObject],
    target: &wgpu::TextureView,
    uniforms: &FrameUniforms,
) -> Result<()> {
    let clear = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
    let mut enc = device.create_command_encoder(&Default::default());

    // Pass 1: the g-buffer MRT. Five color attachments plus the depth attachment the prepass
    // pipeline was built against.
    {
        let attachments: Vec<Option<wgpu::RenderPassColorAttachment<'_>>> = chain
            .gbuffer_color_views()
            .iter()
            .map(|view| {
                Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: clear,
                        store: wgpu::StoreOp::Store,
                    },
                })
            })
            .collect();
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("g-buffer prepass"),
            color_attachments: &attachments,
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: chain.depth_view(),
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        for o in objects {
            o.mesh
                .draw(&mut pass, &mut |pass, d: &scene::DrawCall| {
                    let group = d.texture.map_or(&o.fallback, |t| &o.groups[t]);
                    chain.draw_prepass(queue, pass, group, &uniforms.terrain, d.topology);
                })
                .with_context(|| format!("drawing {}", o.mesh.source))?;
        }
    }

    // Pass 2: the full-screen sun pass. No depth attachment — the pipeline has none, because
    // MODE_SUN recovers eye-Z from attachment 4 G instead.
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sun light"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: clear,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        chain.draw_sun_light(queue, &mut pass, &uniforms.sun);
    }

    queue.submit([enc.finish()]);
    Ok(())
}

/// The 2-D panel as a back-to-front list of bitmaps with their top-left screen positions:
/// the panel background, then the base bitmap of every instrument, found through the original's
/// instrument table. See `xp_core::panel` for what is fitted and what is missing.
fn load_panel(
    gpu: &Gpu,
    install: &Path,
    aircraft_dir: &Path,
    store: &mut TextureStore,
) -> Result<Vec<ui::Layer>> {
    let mut layers = Vec::new();
    // The aircraft's own panel bitmap. An aircraft without one uses a stock panel chosen by its
    // cockpit type in the original (`cockpit/-panels-/Panel_General.png` and its siblings are
    // named in the binary); that choice is not ported, so such an aircraft gets no background.
    let own = aircraft_dir.join("cockpit/-PANELS-");
    let background = std::fs::read_dir(&own).ok().and_then(|rd| {
        let mut names: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        names.sort();
        names.into_iter().find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case("panel.png"))
        })
    });
    match background {
        Some(path) => layers.push(ui::Layer::bitmap(
            store.get_ui(&gpu.device, &gpu.queue, &path)?,
            xp_core::panel::PANEL_ORIGIN_PX,
            PANEL_TINT,
        )),
        None => log::warn!(
            "{} has no cockpit/-PANELS-/Panel.png; stock panels are not ported",
            aircraft_dir.display()
        ),
    }

    let acf_path = std::fs::read_dir(aircraft_dir)
        .with_context(|| format!("listing {}", aircraft_dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().and_then(|e| e.to_str()) == Some("acf"))
        .with_context(|| format!("no .acf in {}", aircraft_dir.display()))?;
    let acf = xp_core::acf::Acf::load(&acf_path)?;
    let instruments = xp_core::panel::instruments(&acf)?;
    let index = xp_core::panel::BitmapIndex::build(install, aircraft_dir)?;
    let mut unresolved = Vec::new();
    for ins in &instruments {
        let Some(path) = index.base(ins.ins_type) else {
            unresolved.push(ins.name.as_str());
            continue;
        };
        let view = store.get_ui(&gpu.device, &gpu.queue, path)?;
        let size = view.texture().size();
        let c = ins.screen_center();
        layers.push(ui::Layer::bitmap(
            view,
            [
                (c[0] - size.width as f32 * 0.5).round(),
                (c[1] - size.height as f32 * 0.5).round(),
            ],
            PANEL_TINT,
        ));
    }
    println!(
        "panel         : {} instruments, {} drawn, {} unresolved",
        instruments.len(),
        instruments.len() - unresolved.len(),
        unresolved.len()
    );
    log::info!("unresolved instrument bitmaps: {unresolved:?}");
    Ok(layers)
}

/// Draw the frame through the forward pass (`terrain_forward.wgsl`), the original's default path.
///
/// The light is the ORACLE's when a probe with the sky block is given: direct and ambient colour
/// and the sun's heading and pitch, read out of its `_skyc`. Without one there is nothing measured
/// to light the scene with, and the M2 placeholder sun is used.
#[allow(clippy::too_many_arguments)]
fn draw_forward(
    gpu: &Gpu,
    fwd: &forward::Forward,
    bindings: &ForwardBindings,
    objects: &[SceneObject],
    camera: &Camera,
    probe: Option<&Probe>,
    extent: (u32, u32),
    target: &wgpu::TextureView,
    cockpit: bool,
) -> Result<()> {
    // In the 2-D cockpit view the original does not draw the aircraft's exterior objects; the
    // runway mesh is the last object of the scene.
    let first = if cockpit {
        objects.len().saturating_sub(1)
    } else {
        0
    };
    let (view, proj) = match camera.exact {
        Some(m) => m,
        None => (
            look_at(camera.eye, camera.target, UP)?,
            perspective(
                camera.fov_y_rad,
                extent.0 as f32 / extent.1 as f32,
                camera.near,
                camera.far,
            ),
        ),
    };
    let placeholder = sun_state(&view);
    let (to_sun_model, diffuse, ambient) = match probe.and_then(|p| p.sun().map(|s| (p, s))) {
        Some((p, sun)) => {
            log::debug!(
                "oracle sun: heading {:.2} pitch {:.2}, direct {:?}, ambient {:?}",
                sun.heading_deg,
                sun.pitch_deg,
                sun.direct_rgb,
                sun.ambient_rgb
            );
            (
                p.world_dir_to_model(sun.direction_world()),
                sun.direct_rgb,
                sun.ambient_rgb,
            )
        }
        None => (
            [0.35, 0.75, -0.55],
            [
                placeholder.diffuse[0],
                placeholder.diffuse[1],
                placeholder.diffuse[2],
            ],
            [
                placeholder.ambient[0],
                placeholder.ambient[1],
                placeholder.ambient[2],
            ],
        ),
    };
    let d = to_sun_model;
    let eye_dir = [
        view[0][0] * d[0] + view[1][0] * d[1] + view[2][0] * d[2],
        view[0][1] * d[0] + view[1][1] * d[1] + view[2][1] * d[2],
        view[0][2] * d[0] + view[1][2] * d[1] + view[2][2] * d[2],
    ];
    let uniforms = forward::ForwardUniforms {
        u_mvp_matrix: mat_mul(&proj, &view),
        u_modelview_matrix: view,
        light_position: [eye_dir[0], eye_dir[1], eye_dir[2], 0.0],
        light_diffuse: [diffuse[0], diffuse[1], diffuse[2], 1.0],
        light_ambient: [ambient[0], ambient[1], ambient[2], 1.0],
        // `terrain_vert.glsl:169`: "X-Plane sets the global ambience to 0".
        scene_color: [0.0, 0.0, 0.0, 1.0],
        // `terrain_frag.glsl:1534`: "Real testing levels will be at zero".
        u_alpha_cut: [0.0, 1.0, 0.0, 0.0],
    };
    let meshes: Vec<&GpuMesh> = objects[first..].iter().map(|o| &o.mesh).collect();
    // The sky dome is not ported; black says so instead of a guessed colour.
    fwd.draw(
        &gpu.device,
        &gpu.queue,
        target,
        wgpu::Color::BLACK,
        &uniforms,
        &meshes,
        &bindings.groups[first..],
        &bindings.fallback[first..],
    )
}

/// The forward pass's bind groups for a scene, one list per object.
struct ForwardBindings {
    groups: Vec<Vec<wgpu::BindGroup>>,
    fallback: Vec<wgpu::BindGroup>,
}

impl ForwardBindings {
    fn new(gpu: &Gpu, fwd: &forward::Forward, objects: &[SceneObject]) -> Self {
        Self {
            groups: objects
                .iter()
                .map(|o| {
                    o.views
                        .iter()
                        .map(|v| fwd.bind_group(&gpu.device, v))
                        .collect()
                })
                .collect(),
            fallback: objects
                .iter()
                .map(|o| fwd.bind_group(&gpu.device, &o.missing))
                .collect(),
        }
    }
}

/// The colour the panel bitmap is multiplied by in the oracle frame, after the PNG gamma table.
///
/// FITTED: with `Panel.png` run through `display_lut` (it carries no `gAMA`), the frame is the
/// bitmap times about (0.782, 0.772, 0.765) over 2 500 texels of dark glare shield and 160 of
/// grey panel alike (each within ±0.01), and (0.809, 0.797, 0.794) on one lighter patch. Before
/// the gamma table was known the same fit gave 0.90 on grey only and could not hold on dark
/// texels. It is the daylight on the panel for that frame's sun; the panel-lighting rule is not
/// ported.
const PANEL_TINT: [f32; 4] = [0.782, 0.772, 0.765, 1.0];

// ---------------------------------------------------------------------------------------------
// GPU setup shared by both paths.
// ---------------------------------------------------------------------------------------------

struct Gpu {
    /// Kept because `Surface::get_capabilities` asks for it.
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

async fn create_gpu(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'static>>,
) -> Result<Gpu> {
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: surface,
            ..Default::default()
        })
        .await
        .context("no adapter available")?;
    log::info!(
        "adapter: {} ({:?})",
        adapter.get_info().name,
        adapter.get_info().backend
    );
    let (device, queue) = passes::request_device(&adapter, "xp-rs M2 frame")
        .await
        .context("requesting a device for the ported passes")?;
    Ok(Gpu {
        adapter,
        device,
        queue,
    })
}

/// An offscreen target the sun pass can draw into and the read-back can copy out of.
fn scene_target(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    extent: (u32, u32),
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hdr scene"),
        size: wgpu::Extent3d {
            width: extent.0.max(1),
            height: extent.1.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        // The sRGB spelling of the same texels, for passes that must blend in linear light the
        // way the original's sRGB framebuffer does (interface text).
        view_formats: &[format.add_srgb_suffix()],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Copy the scene target out and return its RGBA8 bytes, unpadded.
fn read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    extent: (u32, u32),
) -> Result<Vec<u8>> {
    // copy_texture_to_buffer requires a row stride that is a multiple of 256.
    let stride = u64::from((extent.0 * 4 + 255) & !255);
    let size = stride * u64::from(extent.1);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("frame read-back"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride as u32),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: extent.0,
            height: extent.1,
            depth_or_array_layers: 1,
        },
    );
    let index = queue.submit([enc.finish()]);

    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let failure: Arc<std::sync::Mutex<Option<String>>> = Default::default();
    {
        let (d, f) = (Arc::clone(&done), Arc::clone(&failure));
        buffer.map_async(wgpu::MapMode::Read, 0..size, move |r| {
            if let Err(e) = r {
                if let Ok(mut g) = f.lock() {
                    *g = Some(e.to_string());
                }
            }
            d.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    }
    let mut wait_for = Some(index);
    for _ in 0..200 {
        device
            .poll(wgpu::PollType::Wait {
                submission_index: wait_for.take(),
                timeout: None,
            })
            .context("waiting for the frame read-back")?;
        if done.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
    }
    if let Some(e) = failure.lock().ok().and_then(|mut g| g.take()) {
        bail!("mapping the read-back buffer failed: {e}");
    }

    let view = buffer
        .get_mapped_range(0..size)
        .context("reading the mapped frame buffer")?;
    let mut rgba = Vec::with_capacity((extent.0 * extent.1 * 4) as usize);
    for row in 0..extent.1 {
        let start = (row as u64 * stride) as usize;
        let end = start + (extent.0 * 4) as usize;
        rgba.extend_from_slice(&view[start..end]);
    }
    drop(view);
    buffer.unmap();
    Ok(rgba)
}

/// Write RGBA8 as binary PPM. PPM is uncompressed, so the port needs no deflate encoder to produce
/// a viewable frame, and no new dependency to get one.
fn write_ppm(path: &Path, extent: (u32, u32), rgba: &[u8]) -> Result<()> {
    let mut out = format!("P6\n{} {}\n255\n", extent.0, extent.1).into_bytes();
    out.reserve(rgba.len() / 4 * 3);
    for px in rgba.chunks_exact(4) {
        out.extend_from_slice(&px[..3]);
    }
    std::fs::write(path, &out).with_context(|| format!("writing {}", path.display()))
}

/// FNV-1a over the frame. A checksum, not a hash for security: it is here so a change to assembly,
/// upload or the passes shows up as a different number, and it needs no dependency.
fn checksum(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x1000_0000_0000_01b3);
    }
    h
}

fn report(objects: &[SceneObject], store: &TextureStore, rgba: &[u8]) {
    let triangles: usize = objects.iter().map(|o| o.triangles).sum();
    let draws: usize = objects.iter().map(|o| o.mesh.draws.len()).sum();
    let vertices: usize = objects.iter().map(|o| o.mesh.vertex_count).sum();
    let indices: usize = objects.iter().map(|o| o.mesh.index_count).sum();
    println!("objects drawn : {}", objects.len());
    println!("draw calls    : {draws}");
    println!("triangles     : {triangles}");
    println!("vertices      : {vertices}");
    println!("indices       : {indices}");
    println!(
        "textures      : {} uploaded, {} substituted by extension, {} missing",
        store.loaded, store.substituted, store.missing
    );
    println!("frame fnv1a64 : {:016x}", checksum(rgba));
    let mut lit = 0usize;
    let mut dark = 0usize;
    for px in rgba.chunks_exact(4) {
        let l = (px[0] as u32 + px[1] as u32 + px[2] as u32) / 3;
        if l > 24 {
            lit += 1;
        } else if l <= 8 {
            dark += 1;
        }
    }
    let total = rgba.len() / 4;
    println!(
        "coverage      : {} lit px, {} near-black px of {total}",
        lit * 100 / total,
        dark * 100 / total
    );
}

// ---------------------------------------------------------------------------------------------
// Headless path.
// ---------------------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn run_headless(
    install: &Path,
    content: &str,
    start: Option<&Start>,
    probe: Option<&Probe>,
    extent: (u32, u32),
    forward: bool,
    panel: bool,
    menubar: bool,
    menu_open: Option<usize>,
    menu_hover: Option<(f32, f32)>,
    out: &Path,
) -> Result<()> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let gpu = pollster::block_on(create_gpu(&instance, None))?;
    let chain = PassChain::with_size(
        &gpu.device,
        wgpu::TextureFormat::Rgba8Unorm,
        extent.0,
        extent.1,
    )
    .context("building the ported pass chain")?;
    assert_layout_matches(&chain);

    let mut resolver = Resolver::new();
    let mut store = TextureStore::new(&gpu.device, &gpu.queue);
    let (objects, camera) = build_scene(
        &gpu.device,
        &gpu.queue,
        &chain,
        install,
        content,
        start,
        probe,
        &mut resolver,
        &mut store,
    )?;

    let (target, target_view) = scene_target(&gpu.device, wgpu::TextureFormat::Rgba8Unorm, extent);
    let uniforms = frame_uniforms(chain.size(), &camera)?;
    if forward {
        let fwd = forward::Forward::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm, extent);
        let bindings = ForwardBindings::new(&gpu, &fwd, &objects);
        draw_forward(
            &gpu,
            &fwd,
            &bindings,
            &objects,
            &camera,
            probe,
            extent,
            &target_view,
            panel,
        )?;
        if panel || menubar {
            let ui = ui::Ui::new(
                &gpu.device,
                &gpu.queue,
                &mut store,
                install,
                wgpu::TextureFormat::Rgba8Unorm,
            )?;
            let mut layers = Vec::new();
            if panel {
                let aircraft_dir = install.join(content).join("..");
                layers.extend(load_panel(&gpu, install, &aircraft_dir, &mut store)?);
            }
            if menubar {
                let menus = xp_core::menu::load(install, "English", &|t| ui.measure(t))?;
                layers.extend(ui.menu_bar(&gpu.device, &gpu.queue, extent.0 as f32, &menus)?);
                if let Some(m) = menu_open.and_then(|i| menus.get(i)) {
                    let hover = menu_hover.and_then(|(x, y)| xp_core::menu::item_at(m, x, y));
                    // The port's only view is forward-with-panel (item 1800).
                    layers
                        .extend(ui.drop_down(&gpu.device, &gpu.queue, m, hover, &|id| id == 1800)?);
                }
            }
            ui.draw(&gpu.device, &gpu.queue, &target, extent, &layers);
        }
    } else {
        draw_frame(
            &gpu.device,
            &gpu.queue,
            &chain,
            &objects,
            &target_view,
            &uniforms,
        )?;
    }

    if std::env::var_os("XP_PROBE").is_some() {
        // Attachments 0 (albedo) and 3 (emissive) are the two RGBA8 g-buffer slots, so they copy
        // with the 4-bytes-per-texel layout `read_back` assumes; the 16F slots need a wider stride.
        // The pass clears to `wgpu::Color::BLACK`, which carries alpha 1.0, so a non-zero texel is
        // not coverage — the RGB channels are what say whether a fragment landed here.
        for i in [0usize, 3] {
            let view = &chain.gbuffer_color_views()[i];
            let bytes = read_back(&gpu.device, &gpu.queue, view.texture(), extent)?;
            let rgb = bytes.chunks(4).filter(|t| t[..3] != [0, 0, 0]).count();
            let alpha = bytes.chunks(4).filter(|t| t[3] != 0).count();
            println!(
                "probe attachment {i}: {rgb} texels with non-zero RGB, {alpha} with non-zero alpha, of {}",
                bytes.len() / 4
            );
        }
    }
    let rgba = read_back(&gpu.device, &gpu.queue, &target, extent)?;
    if std::env::var_os("XP_PROBE").is_some() {
        let rgb = rgba.chunks(4).filter(|t| t[..3] != [0, 0, 0]).count();
        println!(
            "probe final target: {rgb} texels with non-zero RGB of {}",
            rgba.len() / 4
        );
    }
    write_ppm(out, extent, &rgba)?;
    println!("wrote {}", out.display());
    report(&objects, &store, &rgba);
    Ok(())
}

/// The version line of the loading screen for the pinned build.
const VERSION_LINE: &str = "X-Plane 10.51 (64-bit)";

/// The loading screen as one frame. `status` and `progress` are what the caller is doing; with
/// `golden` they are the ones in `tests/golden/original/frames/loading.png`, so the frame can be
/// compared with it.
fn run_loading(install: &Path, extent: (u32, u32), out: &Path) -> Result<()> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let gpu = pollster::block_on(create_gpu(&instance, None))?;
    let mut store = TextureStore::new(&gpu.device, &gpu.queue);
    let (target, _) = scene_target(&gpu.device, wgpu::TextureFormat::Rgba8Unorm, extent);
    let ui = ui::Ui::new(
        &gpu.device,
        &gpu.queue,
        &mut store,
        install,
        wgpu::TextureFormat::Rgba8Unorm,
    )?;
    // The tip and the status of the oracle frame, so the two can be compared.
    let tip = [
        "Did you know you can accurately simulate thermal activity in X-Plane?".to_string(),
        "Go to Environment > Weather, and adjust thermal coverage, tops, and climb rate."
            .to_string(),
    ];
    let layers = ui.loading(
        &gpu.device,
        &gpu.queue,
        &ui::LoadingState {
            version_line: VERSION_LINE,
            message: &ui::server_message(install),
            tip: &tip,
            status: "Loading bitmap Aircraft/General Aviation/Cessna 172SP/Cessna_172SP_chute.png.",
            progress: 137.0 / 1002.0,
        },
    )?;
    ui.draw(&gpu.device, &gpu.queue, &target, extent, &layers);
    let rgba = read_back(&gpu.device, &gpu.queue, &target, extent)?;
    write_ppm(out, extent, &rgba)?;
    println!("wrote {}", out.display());
    Ok(())
}

/// The upload layout and the pipeline layout must agree, or every prepass draw is wrong in a way
/// the validation layer will not report. `PassChain` builds the pipeline and `GpuMesh` builds the
/// buffer, so the check has to live where both are visible.
fn assert_layout_matches(chain: &PassChain) {
    let want = chain.terrain_vertex_layout();
    let got = GpuMesh::vertex_layout();
    assert_eq!(
        want.array_stride, got.array_stride,
        "the terrain pipeline and the mesh upload disagree on vertex stride"
    );
    assert_eq!(
        want.attributes.len(),
        got.attributes.len(),
        "the terrain pipeline and the mesh upload disagree on attribute count"
    );
    for (w, g) in want.attributes.iter().zip(got.attributes) {
        assert_eq!(
            (w.format as u32, w.offset, w.shader_location),
            (g.format as u32, g.offset, g.shader_location),
            "vertex attribute mismatch between the pipeline and the upload"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Windowed path.
// ---------------------------------------------------------------------------------------------

/// The frame the interface is laid out for. The window shows it scaled to its own size, as the
/// original's 1024×768 window is shown at 2× on a HiDPI display.
const SCREEN: (u32, u32) = (1024, 768);

/// The oracle's cockpit camera and aircraft pose at KSEA 34R, captured with the committed frame
/// `cockpit_default.png`. It is an OBSERVATION of the original used as the window's start state
/// until the flight model and the view code can produce one: every aircraft is shown from the
/// Cessna's pilot eye, standing where the oracle's Cessna stood.
const COCKPIT_PROBE: &str =
    include_str!("../../../tests/golden/original/frames/cockpit_default.probe.txt");

/// The oracle's chase camera at the same start, for View > View is Chase.
const CHASE_PROBE: &str =
    include_str!("../../../tests/golden/original/frames/chase_default.probe.txt");

/// Every text-format `.acf` under `Aircraft/`, as paths relative to the install, sorted.
fn list_aircraft(install: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![install.join("Aircraft")];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for p in rd.filter_map(|e| e.ok().map(|e| e.path())) {
            if p.is_dir() {
                stack.push(p);
            } else if chooser::is_acf(&p) && !xp_core::acf::is_legacy_binary_file(&p) {
                if let Ok(rel) = p.strip_prefix(install) {
                    out.push(rel.to_path_buf());
                }
            }
        }
    }
    out.sort();
    out
}

/// One loaded aircraft at the start location, ready to draw.
struct Session {
    objects: Vec<SceneObject>,
    bindings: ForwardBindings,
    camera: Camera,
    panel: Vec<ui::Layer>,
}

struct Renderer {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    chain: PassChain,
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    fwd: forward::Forward,
    ui: ui::Ui,
    blit: overlay::Overlay,
    menu: Vec<ui::Layer>,
    resolver: Resolver,
    store: TextureStore,
    install: PathBuf,
    probe: Probe,
    aircraft: Vec<PathBuf>,
    current: usize,
    session: Option<Session>,
    /// Loading-screen frames shown for the current aircraft so far.
    loading_frames: u32,
    tip: Vec<String>,
    message: Vec<String>,
    /// The screen the interface is laid out on: the window's size in points, as the original
    /// takes its window's size for its screen (user's screenshot of its Open dialog in a
    /// 1920-wide window: the dialog stays 1000×761 and centred).
    screen: (u32, u32),
    /// Pointer y in screen pixels from the top, if it is over the window.
    pointer_y: Option<f32>,
    pointer_x: f32,
    /// The start-up aircraft dialog, while it is open.
    chooser: Option<chooser::Chooser>,
    /// Its layers, rebuilt only after a click changes it.
    chooser_layers: Option<Vec<ui::Layer>>,
    /// The menu bar, laid out from `Resources/menus/English/X-Plane.txt`.
    menus: Vec<xp_core::menu::Menu>,
    /// The open drop-down, and the layers drawn for it with the item that was under the pointer.
    open_menu: Option<usize>,
    menu_cache: Option<(usize, Option<usize>, Vec<ui::Layer>)>,
    /// The View menu's camera state.
    view: xp_core::view::View,
    /// The oracle chase view's projection: the symmetric one, used by every view but the panel.
    chase_proj: [f32; 16],
    /// Items whose check mark is on (toggles whose own state the port does not run yet).
    toggles: actions::Toggles,
    /// Open windows, back to front, and their layers.
    windows: Vec<actions::Win>,
    windows_cache: Option<Vec<ui::Layer>>,
    /// Every airport of the install by name (`__vec_apt`), read when a list first needs it.
    apt_index: Vec<xp_core::apt_index::AptName>,
    /// A one-line notice at the foot of the screen, and frames left to show it.
    notice: Option<(String, u32)>,
    /// File > Take Screenshot was chosen; written after the next frame.
    screenshot: bool,
    /// `X-Plane Keys.prf`: key text → command.
    key_commands: std::collections::BTreeMap<String, String>,
    /// Settings values (`X-Plane.prf` + the port's own file), the presets from `settings.txt`,
    /// and what Rendering Options shows about the GPU.
    prefs: xp_core::btnwin::Prefs,
    presets: Vec<Vec<(String, f64)>>,
    gl_info: btnui::Info,
    /// The left button is held (slider drags).
    mouse_down: bool,
    /// `--script` steps still to run (see [`actions`]), and where the next screenshot goes.
    script: std::collections::VecDeque<String>,
    shot_to: Option<PathBuf>,
    /// Set by File > Quit; the event loop exits on it.
    quit: bool,
}

impl Renderer {
    async fn new(window: Arc<Window>, install: &Path, aircraft: &Path) -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance.create_surface(Arc::clone(&window))?;
        let gpu = create_gpu(&instance, Some(&surface))
            .await
            .context("setting up the windowed GPU")?;

        let caps = surface.get_capabilities(&gpu.adapter);
        // A non-sRGB surface: the frame already holds display-encoded values, as the original's
        // screenshots do, so the surface must pass them through rather than encode them again.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
                wgpu::CompositeAlphaMode::Opaque
            } else {
                caps.alpha_modes[0]
            },
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: Default::default(),
        };
        surface.configure(&gpu.device, &config);

        const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
        let screen = logical_size(&window);
        let chain = PassChain::with_size(&gpu.device, FORMAT, SCREEN.0, SCREEN.1)
            .context("building the pass chain")?;
        let (target, target_view) = scene_target(&gpu.device, FORMAT, screen);
        let fwd = forward::Forward::new(&gpu.device, FORMAT, screen);
        let mut store = TextureStore::new(&gpu.device, &gpu.queue);
        let ui = ui::Ui::new(&gpu.device, &gpu.queue, &mut store, install, FORMAT)?;
        let menus = xp_core::menu::load(install, "English", &|t| ui.measure(t))?;
        let menu = ui.menu_bar(&gpu.device, &gpu.queue, screen.0 as f32, &menus)?;
        let menus_kept = menus.clone();
        let mut blit = overlay::Overlay::new(&gpu.device, format);
        blit.opaque = true;

        let list = list_aircraft(install);
        let current = list.iter().position(|p| p == aircraft).with_context(|| {
            format!(
                "{} is not a text-format aircraft under Aircraft/ (see --list-aircraft)",
                aircraft.display()
            )
        })?;
        // The original picks a hint at random; the port has no random source by rule, so the
        // choice follows the clock's seconds, which is the one place wall time is read.
        let cockpit = Probe::parse(COCKPIT_PROBE)?;
        let chase = Probe::parse(CHASE_PROBE)?;
        let view = xp_core::view::View::new(cockpit.acf_mv, &chase.acf_mv);
        let pick = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as usize)
            .unwrap_or(0);
        let gl_info = {
            let a = gpu.adapter.get_info();
            btnui::Info {
                vendor: format!("{:?}", a.backend),
                renderer: a.name.clone(),
                version: format!("wgpu {} - {}", a.driver, a.driver_info),
                texmem: "Total size of all loaded textures at current settings: not measured by the port".into(),
            }
        };
        Ok(Self {
            surface,
            config,
            gpu,
            chain,
            target,
            target_view,
            fwd,
            ui,
            blit,
            menu,
            resolver: Resolver::new(),
            store,
            install: install.to_path_buf(),
            probe: cockpit,
            screen,
            aircraft: list,
            current,
            session: None,
            loading_frames: 0,
            tip: ui::hint(install, pick),
            message: ui::server_message(install),
            pointer_y: None,
            pointer_x: 0.0,
            // The original opens its aircraft dialog at start-up (user's screenshot).
            chooser: Some(chooser::Chooser::new(install, aircraft)),
            chooser_layers: None,
            menus: menus_kept,
            open_menu: None,
            menu_cache: None,
            view,
            chase_proj: chase.proj,
            toggles: Default::default(),
            windows: Vec::new(),
            windows_cache: None,
            apt_index: Vec::new(),
            notice: None,
            screenshot: false,
            key_commands: actions::key_commands(install),
            script: Default::default(),
            shot_to: None,
            prefs: xp_core::btnwin::Prefs::load(install, port_settings_path()),
            presets: xp_core::btnwin::presets(
                &xp_core::read_text_lossy(install.join("Resources/settings.txt"))
                    .unwrap_or_default(),
            ),
            gl_info,
            mouse_down: false,
            quit: false,
        })
    }

    /// Top-left corner of the dialog: centred on the screen, as in the user's screenshot.
    fn dialog_origin(&self) -> [f32; 2] {
        [
            ((self.screen.0 as f32 - chooser::DIALOG_W) * 0.5).floor(),
            ((self.screen.1 as f32 - chooser::DIALOG_H) * 0.5).floor(),
        ]
    }

    /// A left click at the pointer.
    fn click(&mut self) {
        let (x, y) = (self.pointer_x, self.pointer_y.unwrap_or(-1.0));
        let o = self.dialog_origin();
        if let Some(c) = &mut self.chooser {
            let action = c.click(x - o[0], y - o[1]);
            self.chooser_layers = None;
            match action {
                chooser::Action::Open(path) => {
                    if let Some(i) = self.aircraft.iter().position(|p| *p == path) {
                        self.current = i;
                        self.session = None;
                        self.loading_frames = 0;
                        self.chooser = None;
                    } else {
                        log::warn!(
                            "{} is a legacy binary aircraft; it cannot be loaded",
                            path.display()
                        );
                    }
                }
                chooser::Action::Cancel => self.chooser = None,
                chooser::Action::None => {}
            }
            return;
        }
        // The menu bar, as `mbar_select_an_item_click` / `mbar_select_an_item` behave: a click on
        // a title opens it (or closes it if it is already open), a click on an item of the open
        // menu runs it and closes the menu, a click anywhere else closes the menu.
        if let Some(open) = self.open_menu {
            if let Some(m) = self.menus.get(open) {
                if let Some(i) = xp_core::menu::item_at(m, x, y) {
                    let item = m.items[i].clone();
                    self.open_menu = None;
                    self.menu_cache = None;
                    self.run_menu_item(&item);
                    return;
                }
            }
            match xp_core::menu::title_at(&self.menus, x, y) {
                Some(t) if t != open => self.open_menu = Some(t),
                _ => self.open_menu = None,
            }
            self.menu_cache = None;
            return;
        }
        if let Some(t) = xp_core::menu::title_at(&self.menus, x, y) {
            self.open_menu = Some(t);
            self.menu_cache = None;
            return;
        }
        self.click_windows(x, y);
    }

    fn aircraft_rel(&self) -> &Path {
        &self.aircraft[self.current]
    }

    /// Switch to the aircraft `step` places along the sorted list and show the loading screen.
    fn cycle_aircraft(&mut self, step: isize) {
        let n = self.aircraft.len() as isize;
        self.current = (self.current as isize + step).rem_euclid(n) as usize;
        self.session = None;
        self.loading_frames = 0;
    }

    fn load_session(&mut self) -> Result<Session> {
        let aircraft_dir = self
            .install
            .join(self.aircraft_rel())
            .parent()
            .context("aircraft has no directory")?
            .to_path_buf();
        let content = aircraft_dir
            .strip_prefix(&self.install)
            .unwrap_or(&aircraft_dir)
            .join("objects");
        let start = Start {
            airport: START_AIRPORT.to_string(),
            runway: "34R".to_string(),
        };
        let (objects, camera) = build_scene(
            &self.gpu.device,
            &self.gpu.queue,
            &self.chain,
            &self.install,
            &content.to_string_lossy(),
            Some(&start),
            Some(&self.probe),
            &mut self.resolver,
            &mut self.store,
        )?;
        let bindings = ForwardBindings::new(&self.gpu, &self.fwd, &objects);
        let panel = load_panel(&self.gpu, &self.install, &aircraft_dir, &mut self.store)?;
        Ok(Session {
            objects,
            bindings,
            camera,
            panel,
        })
    }

    fn resize(&mut self, w: u32, h: u32, screen: (u32, u32)) {
        if w == 0 || h == 0 {
            return;
        }
        if (w, h) != (self.config.width, self.config.height) {
            self.config.width = w;
            self.config.height = h;
            self.surface.configure(&self.gpu.device, &self.config);
        }
        if screen == self.screen || screen.0 == 0 || screen.1 == 0 {
            return;
        }
        self.screen = screen;
        let (target, view) = scene_target(&self.gpu.device, self.target.format(), screen);
        self.target = target;
        self.target_view = view;
        self.fwd.resize(&self.gpu.device, screen);
        match self.ui.menu_bar(
            &self.gpu.device,
            &self.gpu.queue,
            screen.0 as f32,
            &self.menus,
        ) {
            Ok(m) => self.menu = m,
            Err(e) => log::error!("menu bar: {e:#}"),
        }
        self.chooser_layers = None;
        self.menu_cache = None;
        self.windows_cache = None;
        self.place_windows();
    }

    /// Where the 1024×768 layouts (cockpit panel, loading screen) go on a larger screen.
    /// PLACEHOLDER placement, not observed on the original: the panel bottom-centred, the
    /// loading screen centred.
    fn panel_offset(&self) -> [f32; 2] {
        [
            ((self.screen.0 as f32 - SCREEN.0 as f32) * 0.5).floor(),
            self.screen.1 as f32 - SCREEN.1 as f32,
        ]
    }

    fn draw_scene(&mut self) -> Result<()> {
        let screen = self.screen;
        let origin = self.dialog_origin();
        if let Some(c) = &self.chooser {
            if self.chooser_layers.is_none() {
                let (device, queue) = (&self.gpu.device, &self.gpu.queue);
                let mut layers = vec![self.ui.fill(
                    [0.0, 0.0, self.screen.0 as f32, self.screen.1 as f32],
                    // The window behind the dialog, measured on the user's screenshot.
                    [13, 69, 101, 255],
                )];
                layers.extend(c.layers(&self.ui, device, queue, &mut self.store, origin)?);
                self.chooser_layers = Some(layers);
            }
            if let Some(layers) = &self.chooser_layers {
                self.ui.draw(
                    &self.gpu.device,
                    &self.gpu.queue,
                    &self.target,
                    screen,
                    layers,
                );
            }
            return Ok(());
        }
        let (device, queue) = (&self.gpu.device, &self.gpu.queue);
        match &self.session {
            None => {
                let status = format!("Loading {}.", self.aircraft_rel().display());
                let layers = self.ui.loading(
                    device,
                    queue,
                    &ui::LoadingState {
                        version_line: VERSION_LINE,
                        message: &self.message,
                        tip: &self.tip,
                        status: &status,
                        progress: (self.loading_frames as f32 * 0.25).min(1.0),
                    },
                )?;
                // The loading screen is laid out for 1024×768; centred on black on a larger one.
                let [dx, dy] = [
                    ((screen.0 as f32 - SCREEN.0 as f32) * 0.5).floor(),
                    ((screen.1 as f32 - SCREEN.1 as f32) * 0.5).floor(),
                ];
                let mut all = vec![self
                    .ui
                    .fill([0.0, 0.0, screen.0 as f32, screen.1 as f32], [0, 0, 0, 255])];
                all.extend(ui::shifted(layers, dx, dy));
                self.ui.draw(device, queue, &self.target, screen, &all);
            }
            Some(s) => {
                use xp_core::view::Mode;
                let mode = self.view.mode;
                let proj = if mode == Mode::ForwardPanel {
                    self.probe.proj
                } else {
                    self.chase_proj
                };
                let mut camera = s.camera;
                camera.exact = Some((
                    flat_to_mat(&self.view.matrix()),
                    gl_proj_to_wgpu(
                        &self
                            .view
                            .projection(&proj, screen.0 as f32 / screen.1 as f32),
                    ),
                ));
                draw_forward(
                    &self.gpu,
                    &self.fwd,
                    &s.bindings,
                    &s.objects,
                    &camera,
                    Some(&self.probe),
                    screen,
                    &self.target_view,
                    mode.is_forward(),
                )?;
                if mode == Mode::ForwardPanel {
                    let [dx, dy] = self.panel_offset();
                    let panel = ui::shifted_ref(&s.panel, dx, dy);
                    self.ui.draw(device, queue, &self.target, screen, &panel);
                }
                // PLACEHOLDER tints for the eye-wear toggles: the original's night-vision and
                // flashlight shaders are not ported, these only make the toggles visible.
                let full = [0.0, 0.0, screen.0 as f32, screen.1 as f32];
                let mut tints = Vec::new();
                if self.toggles.contains(&1833) {
                    tints.push(self.ui.fill(full, [40, 255, 60, 90]));
                }
                if self.toggles.contains(&1832) {
                    tints.push(self.ui.fill(full, [0, 0, 0, 90]));
                }
                if self.toggles.contains(&1834) {
                    tints.push(self.ui.fill(full, [255, 0, 0, 40]));
                }
                if !tints.is_empty() {
                    self.ui.draw(device, queue, &self.target, screen, &tints);
                }
                if !self.windows.is_empty() {
                    if self.windows_cache.is_none() {
                        let mut layers = Vec::new();
                        // With a settings window open the original stops drawing the world and
                        // fills the screen with the dialog-back blue (oracle frames win_*.png).
                        if self.windows.iter().any(|w| w.def.is_some()) {
                            layers.push(self.ui.fill(
                                [0.0, 0.0, screen.0 as f32, screen.1 as f32],
                                [13, 69, 101, 255],
                            ));
                        }
                        for w in &self.windows {
                            layers.extend(self.ui.window(
                                device,
                                queue,
                                w.rect,
                                w.frame,
                                &w.title,
                                w.closeable(),
                            )?);
                            if let Some(def) = &w.def {
                                layers.extend(btnui::layers(
                                    &self.ui,
                                    device,
                                    queue,
                                    def,
                                    &self.prefs,
                                    &self.gl_info,
                                    &w.state,
                                    w.place(),
                                )?);
                                let p = w.place();
                                if let Some(a) = &w.apt {
                                    let y = if def.id == 22 { 605.0 } else { 595.0 };
                                    layers.extend(special::apt_list(
                                        &self.ui,
                                        device,
                                        queue,
                                        &self.apt_index,
                                        a,
                                        &p,
                                        20.0,
                                        y,
                                    )?);
                                }
                                if let (1, Some(a)) = (def.id, &w.apt) {
                                    layers.extend(special::airp_extras(
                                        &self.ui, device, queue, a, &p,
                                    )?);
                                }
                                if def.id == 22 {
                                    layers.extend(special::qflt_pictures(
                                        &self.ui,
                                        device,
                                        queue,
                                        &mut self.store,
                                        &self.install,
                                        self.prefs.get("_qfl_tim") as i32,
                                        self.prefs.get("_qfl_wxr") as i32,
                                        &p,
                                    )?);
                                }
                                if let Some(t) = &w.tree {
                                    layers.extend(special::qflt_aircraft(
                                        &self.ui,
                                        device,
                                        queue,
                                        &mut self.store,
                                        t,
                                        &p,
                                    )?);
                                }
                                continue;
                            }
                            let pane = xp_core::menu::Rect {
                                l: w.rect.l + 10.0,
                                t: w.rect.t + 28.0,
                                r: w.rect.r - 10.0,
                                b: w.rect.b - 10.0,
                            };
                            layers.extend(self.ui.pane(pane));
                            for (i, line) in w.lines.iter().enumerate() {
                                if line.is_empty() {
                                    continue;
                                }
                                layers.push(self.ui.text_at(
                                    device,
                                    queue,
                                    line,
                                    [pane.l + 16.0, pane.t + 24.0 + 16.0 * i as f32],
                                    [51, 51, 51],
                                )?);
                            }
                        }
                        self.windows_cache = Some(layers);
                    }
                    if let Some(layers) = &self.windows_cache {
                        self.ui.draw(device, queue, &self.target, screen, layers);
                    }
                }
                if let Some((text, left)) = &mut self.notice {
                    let layers = [
                        self.ui
                            .fill([0.0, 740.0, SCREEN.0 as f32, 22.0], [0, 0, 0, 150]),
                        self.ui
                            .text_at(device, queue, text, [10.0, 755.0], [255, 255, 255])?,
                    ];
                    self.ui.draw(device, queue, &self.target, screen, &layers);
                    *left -= 1;
                    if *left == 0 {
                        self.notice = None;
                    }
                }
                // `mbar_draw_menu_bar`: the bar shows while the pointer is in the top 18 rows, or
                // while a menu is open.
                if self.pointer_y.is_some_and(|y| y < 18.0) || self.open_menu.is_some() {
                    self.ui
                        .draw(device, queue, &self.target, screen, &self.menu);
                }
                if let Some(open) = self.open_menu {
                    // With a menu open, the pointer over another title switches to it.
                    let (px, py) = (self.pointer_x, self.pointer_y.unwrap_or(-1.0));
                    let open = match xp_core::menu::title_at(&self.menus, px, py) {
                        Some(t) => t,
                        None => open,
                    };
                    self.open_menu = Some(open);
                    let hover = self
                        .menus
                        .get(open)
                        .and_then(|m| xp_core::menu::item_at(m, px, py));
                    let stale =
                        !matches!(&self.menu_cache, Some((o, h, _)) if *o == open && *h == hover);
                    if stale {
                        if let Some(m) = self.menus.get(open) {
                            let checked = |id: i32| self.checked(id);
                            let layers = self.ui.drop_down(
                                &self.gpu.device,
                                &self.gpu.queue,
                                m,
                                hover,
                                &checked,
                            )?;
                            self.menu_cache = Some((open, hover, layers));
                        }
                    }
                    if let Some((_, _, layers)) = &self.menu_cache {
                        self.ui.draw(
                            &self.gpu.device,
                            &self.gpu.queue,
                            &self.target,
                            screen,
                            layers,
                        );
                    }
                }
            }
        }
        Ok(())
    }

    fn render(&mut self) {
        use wgpu::CurrentSurfaceTexture as T;
        let frame = match self.surface.get_current_texture() {
            T::Success(f) | T::Suboptimal(f) => f,
            T::Outdated | T::Lost => {
                self.surface.configure(&self.gpu.device, &self.config);
                return;
            }
            other => {
                log::debug!("skipping frame: {other:?}");
                return;
            }
        };
        if let Err(e) = self.draw_scene() {
            log::error!("{e:#}");
        }
        if std::mem::take(&mut self.screenshot) {
            self.save_screenshot();
        }
        if self.session.is_some() || self.chooser.is_some() {
            self.script_step();
        }
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.blit.draw(
            &self.gpu.device,
            &self.gpu.queue,
            &view,
            (self.config.width, self.config.height),
            &[overlay::Quad {
                texture: &self.target_view,
                rect: [
                    0.0,
                    0.0,
                    self.config.width as f32,
                    self.config.height as f32,
                ],
                tint: [1.0; 4],
                src: None,
            }],
        );
        self.queue_present(frame);

        // Load after the loading screen has been on screen for a few frames, so it is seen.
        if self.session.is_none() && self.chooser.is_none() {
            self.loading_frames += 1;
            if self.loading_frames >= 4 {
                match self.load_session() {
                    Ok(s) => self.session = Some(s),
                    Err(e) => {
                        log::error!("loading {}: {e:#}", self.aircraft_rel().display());
                        // Stay on the loading screen rather than loop on a failing aircraft.
                        self.loading_frames = 0;
                        self.cycle_aircraft(1);
                    }
                }
            }
        }
    }

    fn queue_present(&self, frame: wgpu::SurfaceTexture) {
        self.gpu.queue.present(frame);
    }
}

/// The window's inner size in points: the original is not HiDPI-aware, so its screen is the
/// window in points and the system shows each point as scale-factor pixels.
fn logical_size(window: &Window) -> (u32, u32) {
    let s = window.inner_size().to_logical::<f64>(window.scale_factor());
    (
        s.width.round().max(1.0) as u32,
        s.height.round().max(1.0) as u32,
    )
}

struct App {
    mods: winit::keyboard::ModifiersState,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    install: PathBuf,
    aircraft: PathBuf,
    script: Vec<String>,
    /// Initial window size in points (`--size`).
    size: (u32, u32),
}

impl App {
    fn retitle(&self) {
        if let (Some(w), Some(r)) = (&self.window, &self.renderer) {
            w.set_title(&format!(
                "xplane10-rs — {} ({} of {})",
                r.aircraft_rel().display(),
                r.current + 1,
                r.aircraft.len()
            ));
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("xplane10-rs")
            .with_inner_size(winit::dpi::LogicalSize::new(self.size.0, self.size.1));
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("creating a window: {e}");
                el.exit();
                return;
            }
        };
        let renderer = match pollster::block_on(Renderer::new(
            Arc::clone(&window),
            &self.install,
            &self.aircraft,
        )) {
            Ok(r) => r,
            Err(e) => {
                log::error!("{e:#}");
                el.exit();
                return;
            }
        };
        self.window = Some(window);
        let mut renderer = renderer;
        renderer.script = std::mem::take(&mut self.script).into();
        self.renderer = Some(renderer);
        self.retitle();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => {
                if let (Some(r), Some(w)) = (&mut self.renderer, &self.window) {
                    r.resize(s.width, s.height, logical_size(w));
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let (Some(r), Some(w)) = (&mut self.renderer, &self.window) {
                    let s = w.inner_size();
                    r.resize(s.width, s.height, logical_size(w));
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(r) = &mut self.renderer {
                    r.pointer_y =
                        Some(position.y as f32 * r.screen.1 as f32 / r.config.height.max(1) as f32);
                    r.pointer_x =
                        position.x as f32 * r.screen.0 as f32 / r.config.width.max(1) as f32;
                    if r.mouse_down {
                        let x = r.pointer_x;
                        r.drag_windows(x);
                    }
                }
            }
            WindowEvent::MouseInput {
                state: winit::event::ElementState::Released,
                button: winit::event::MouseButton::Left,
                ..
            } => {
                if let Some(r) = &mut self.renderer {
                    r.mouse_down = false;
                    r.release();
                }
            }
            WindowEvent::MouseInput {
                state: winit::event::ElementState::Pressed,
                button: winit::event::MouseButton::Left,
                ..
            } => {
                if let Some(r) = &mut self.renderer {
                    r.mouse_down = true;
                    r.click();
                    if r.quit {
                        el.exit();
                    }
                }
                self.retitle();
            }
            WindowEvent::CursorLeft { .. } => {
                if let Some(r) = &mut self.renderer {
                    r.pointer_y = None;
                }
            }
            // NOT the original's control: its aircraft chooser is a dialog under the Aircraft
            // menu, which is not ported. `[` and `]` step through the installed aircraft so one
            // can be chosen at all until it is.
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state.is_pressed() => {
                let Some(r) = &mut self.renderer else {
                    return;
                };
                let code = match event.physical_key {
                    winit::keyboard::PhysicalKey::Code(c) => Some(c),
                    _ => None,
                };
                if code == Some(winit::keyboard::KeyCode::Escape) {
                    r.escape();
                    return;
                }
                let bound = code
                    .and_then(actions::key_name)
                    .is_some_and(|n| r.key(&actions::key_text(&n, self.mods)));
                if r.quit {
                    el.exit();
                }
                if bound {
                    return;
                }
                // NOT the original's control: `[` and `]` step through the installed aircraft.
                let step = match event.logical_key.as_ref() {
                    winit::keyboard::Key::Character("]") => 1,
                    winit::keyboard::Key::Character("[") => -1,
                    _ => 0,
                };
                if step != 0 {
                    r.cycle_aircraft(step);
                    self.retitle();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(r) = &mut self.renderer {
                    r.render();
                    if r.quit {
                        el.exit();
                        return;
                    }
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Entry point.
// ---------------------------------------------------------------------------------------------

/// The aircraft the original loads by default (`I/ACF: Loading airplane number 0 with …`).
const DEFAULT_AIRCRAFT: &str = "Aircraft/General Aviation/Cessna 172SP/Cessna_172SP.acf";

fn main() -> Result<()> {
    env_logger::init();
    let mut args = std::env::args().skip(1);
    let mut install = None;
    let mut content: Option<String> = None;
    let mut aircraft = PathBuf::from(DEFAULT_AIRCRAFT);
    let mut list = false;
    let mut headless: Option<PathBuf> = None;
    let mut airport_id: Option<String> = None;
    let mut runway: Option<String> = None;
    let mut probe: Option<Probe> = None;
    let mut extent = HEADLESS_EXTENT;
    let mut forward = false;
    let mut panel = false;
    let mut loading = false;
    let mut script: Vec<String> = Vec::new();
    // The original's default window: 1024×768 points.
    let mut window_size = SCREEN;
    let mut menubar = false;
    let mut menu_open: Option<usize> = None;
    let mut menu_hover: Option<(f32, f32)> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--install" => {
                install = Some(args.next().context("--install wants a path")?);
            }
            "--content" => {
                content = Some(args.next().context("--content wants a path")?);
            }
            "--aircraft" => {
                aircraft = args
                    .next()
                    .context("--aircraft wants an .acf path relative to the install")?
                    .into();
            }
            "--list-aircraft" => list = true,
            "--headless" => {
                headless = Some(
                    args.next()
                        .context("--headless wants an output path")?
                        .into(),
                );
            }
            "--airport" => {
                airport_id = Some(args.next().context("--airport wants an identifier")?);
            }
            "--runway" => {
                runway = Some(args.next().context("--runway wants a runway end")?);
            }
            "--oracle-probe" => {
                probe = Some(Probe::load(
                    args.next().context("--oracle-probe wants a probe file")?,
                )?);
            }
            "--panel" => panel = true,
            "--loading" => loading = true,
            "--script" => {
                script = args
                    .next()
                    .context("--script wants steps separated by ';'")?
                    .split(';')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "--menubar" => menubar = true,
            "--menu-open" => {
                menubar = true;
                menu_open = Some(args.next().context("--menu-open wants an index")?.parse()?);
            }
            "--menu-hover" => {
                let v = args.next().context("--menu-hover wants X,Y")?;
                let (x, y) = v.split_once(',').context("--menu-hover wants X,Y")?;
                menu_hover = Some((x.parse()?, y.parse()?));
            }
            "--renderer" => {
                forward = match args.next().as_deref() {
                    Some("forward") => true,
                    Some("deferred") => false,
                    other => bail!("--renderer wants forward or deferred, got {other:?}"),
                };
            }
            "--size" => {
                let v = args.next().context("--size wants WIDTHxHEIGHT")?;
                let (w, h) = v.split_once('x').context("--size wants WIDTHxHEIGHT")?;
                extent = (w.parse()?, h.parse()?);
                window_size = extent;
            }
            other => bail!("unrecognised argument {other:?}"),
        }
    }

    let root = match install {
        Some(p) => PathBuf::from(p),
        None => match xp_core::install_root().or_else(find_install) {
            Some(p) => p,
            None => bail!(
                "no X-Plane 10 installation found: set XP_INSTALL, pass --install, or put the \
                 app next to a folder named \"X-Plane 10\""
            ),
        },
    };
    if !root.is_dir() {
        bail!("install root {} is not a directory", root.display());
    }
    if list {
        for a in list_aircraft(&root) {
            println!("{}", a.display());
        }
        return Ok(());
    }
    // The aircraft's OBJ8 objects live beside its .acf; `--content` overrides the directory.
    let content = match content {
        Some(c) => c,
        None => aircraft
            .parent()
            .context("--aircraft has no directory")?
            .join("objects")
            .to_string_lossy()
            .into_owned(),
    };

    let start = match (airport_id, runway) {
        (Some(airport), Some(runway)) => Some(Start { airport, runway }),
        (None, None) => None,
        _ => bail!("--airport and --runway go together, e.g. --airport KSEA --runway 34R"),
    };

    if loading {
        let out = headless.context("--loading wants --headless <out.ppm>")?;
        return run_loading(&root, extent, &out);
    }

    match headless {
        Some(out) => run_headless(
            &root,
            &content,
            start.as_ref(),
            probe.as_ref(),
            extent,
            forward,
            panel,
            menubar,
            menu_open,
            menu_hover,
            &out,
        ),
        None => run_windowed(root, aircraft, script, window_size),
    }
}

/// Is `p` an X-Plane 10 installation? It must hold the menus, the aircraft and the app.
fn looks_like_install(p: &Path) -> bool {
    p.join("Resources/menus").is_dir() && p.join("Aircraft").is_dir()
}

/// Where a found installation is remembered between runs.
/// The port's own settings file: changes made in the settings windows. The X-Plane
/// installation is never written to.
fn port_settings_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(home.join("Library/Application Support/xplane10-rs/settings.prf"))
}

fn remembered_install_file() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(home.join("Library/Application Support/xplane10-rs/install.txt"))
}

/// Find an installation on this computer when none is given, so the app starts from Finder:
///
/// 1. the path remembered from the last successful search;
/// 2. a folder named `X-Plane 10` beside the app or beside any folder above the executable, or
///    one of those folders itself;
/// 3. the usual places: `~`, `~/Desktop`, `~/Documents`, `~/Downloads`, `~/Applications`,
///    `/Applications`, the Steam library, and the top of every mounted volume;
/// 4. Spotlight (`mdfind`) for any folder holding `X-Plane.app` whose version is 10.
///
/// The first hit is remembered.
fn find_install() -> Option<PathBuf> {
    let remember = |p: &Path| {
        if let Some(f) = remembered_install_file() {
            if let Some(d) = f.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(&f, p.to_string_lossy().as_bytes());
        }
    };
    if let Some(f) = remembered_install_file() {
        if let Ok(t) = std::fs::read_to_string(&f) {
            let p = PathBuf::from(t.trim());
            if looks_like_install(&p) {
                return Some(p);
            }
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        for dir in exe.ancestors().skip(1) {
            candidates.push(dir.join("X-Plane 10"));
            candidates.push(dir.to_path_buf());
        }
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for sub in [
            "",
            "Desktop",
            "Documents",
            "Downloads",
            "Applications",
            "Games",
            "Library/Application Support/Steam/steamapps/common",
        ] {
            candidates.push(home.join(sub).join("X-Plane 10"));
        }
    }
    candidates.push(PathBuf::from("/Applications/X-Plane 10"));
    if let Ok(rd) = std::fs::read_dir("/Volumes") {
        for v in rd.filter_map(|e| e.ok()) {
            candidates.push(v.path().join("X-Plane 10"));
            candidates.push(v.path().join("Games/X-Plane 10"));
        }
    }
    if let Some(p) = candidates.into_iter().find(|p| looks_like_install(p)) {
        remember(&p);
        return Some(p);
    }
    // Spotlight: every X-Plane.app on the machine; its parent is an installation.
    let out = std::process::Command::new("mdfind")
        .arg("kMDItemFSName == 'X-Plane.app'")
        .output()
        .ok()?;
    let hit = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| Path::new(l).parent().map(Path::to_path_buf))
        .find(|p| looks_like_install(p))?;
    log::info!("found X-Plane 10 with Spotlight at {}", hit.display());
    remember(&hit);
    Some(hit)
}

fn run_windowed(
    root: PathBuf,
    aircraft: PathBuf,
    script: Vec<String>,
    size: (u32, u32),
) -> Result<()> {
    let el = EventLoop::new()?;
    el.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        mods: Default::default(),
        window: None,
        renderer: None,
        install: root,
        aircraft,
        script,
        size,
    };
    el.run_app(&mut app)?;
    Ok(())
}
