//! CPU side of the g-buffer prepass and the HDR sun-lighting pass.
//!
//! This module owns the *contract* — attachment formats, mode constants and the uniform structs
//! that mirror the WGSL in `crates/xp/shaders/`. The GPU objects live in [`crate::passes`].
//!
//! ## The shipped MRT contract
//!
//! `terrain_frag.glsl` is the only writer of draw buffers in the whole shader tree, and
//! `output_gbuffer()` documents the layout it writes (verbatim, `terrain_frag.glsl:1520-1529`):
//!
//! ```text
//!                                     R           G           B           A
//! Our final scheme:   0   albedo_r    albedo_g    albedo_b    albedo_a
//!                     1   ao          shiny       empty       unusable
//!                     2   normal_x    normal_y    -           -
//!                     3   emissive_r  emissive_g  emissive_b  emissive_a
//!                     4   unused      depth       -           -
//!
//! Not using BA of the middle two channels is intentional!  We can use RGBA_F16 on these
//! while using RGBA8888 on the albedo/emissive.  This squeezes everybody down to 16 bytes.
//! The alpha channel of the position and normal vectors was dead to us anyway, as it had to be
//! hard cut to be "in" or "out".
//! ```
//!
//! The writes themselves are `terrain_frag.glsl:1575-1579`; the sampler names that read them back
//! are `deferred_common.glsl:13-17` and the comments on `deferred_frag.glsl:536-540`
//! ("G is eye-space Z * 1024 to fit mantissa of f16", "Alpha is total scene transparency, RGB is
//! albedo", "Alpha unused, R = AO, G = specular", "RG is lambertian encoded normal"). Which
//! attachment becomes which sampler is not stated by any GLSL file — it is forced by those channel
//! comments, and it is spelled out in [`GBUFFER_SAMPLERS`].
//!
//! A real depth attachment is not declared in GLSL at all: the g-buffer carries its own eye-Z in
//! attachment 4 G, and `terrain_frag.glsl:1572-1573` ("channel 5 MUST be hard cut, not blended")
//! is why that channel gets no blending. The port adds a depth buffer anyway because
//! `terrain_vert.glsl:280-282` argues for early-Z ("we do _not_ want to eat that fill rate hit")
//! and the prepass is depth-correct geometry; that inference is marked as such in [`DEPTH_FORMAT`].

// The mode constants below mirror the packed `mode` integer `terrain_vert.glsl` writes and the
// arms `terrain_frag.glsl` branches on. Only the arms the ported prepass actually selects are
// read today; the rest are kept as the documented contract for the passes not yet ported, so
// dead code here is intentional, not unfinished naming.
#![allow(dead_code)]

use bytemuck::{Pod, Zeroable};

/// Attachment index for albedo (`data_out0`, `terrain_frag.glsl:1575`).
pub const ATTACH_ALBEDO: usize = 0;
/// Attachment index for ao/specular/fog (`data_out1`, `terrain_frag.glsl:1576`).
pub const ATTACH_MAT: usize = 1;
/// Attachment index for the encoded normal (`data_out2`, `terrain_frag.glsl:1577`).
pub const ATTACH_NORMAL: usize = 2;
/// Attachment index for emissive (`data_out3`, `terrain_frag.glsl:1578`).
pub const ATTACH_EMISSIVE: usize = 3;
/// Attachment index for position/depth + hard-cut flag (`data_out4`, `terrain_frag.glsl:1579`).
pub const ATTACH_POSITION: usize = 4;

/// Color attachment formats, in draw-buffer order, per the contract quoted above:
/// RGBA8 for albedo and emissive, RGBA16F for mat/normal/position
/// ("We can use RGBA_F16 on these while using RGBA8888 on the albedo/emissive",
/// `terrain_frag.glsl:1527-1528`).
pub const GBUFFER_FORMATS: [wgpu::TextureFormat; 5] = [
    wgpu::TextureFormat::Rgba8Unorm,  // 0 albedo
    wgpu::TextureFormat::Rgba16Float, // 1 ao / specular / fog
    wgpu::TextureFormat::Rgba16Float, // 2 normal
    wgpu::TextureFormat::Rgba8Unorm,  // 3 emissive
    wgpu::TextureFormat::Rgba16Float, // 4 position / depth
];

/// Which g-buffer attachment each `deferred_common.glsl:13-17` sampler reads.
///
/// INFERRED, not stated: no GLSL file maps a sampler to a draw buffer. The mapping is forced by
/// the channel comments at `deferred_frag.glsl:536-540` against the writes at
/// `terrain_frag.glsl:1575-1579` — e.g. `gbuffer_pos` must be attachment 4 because `:542` reads
/// `eye_pos_pixel.y` as "eye-space Z * 1024" and `:1579` is the only write that puts eye-Z in G.
/// Indices are `[gbuffer_tex, gbuffer_mat, gbuffer_nrm, gbuffer_lit, gbuffer_pos]` in
/// attachment order; see `sun_light.wgsl`'s `@group(0) @binding(1..=5)` block.
pub const GBUFFER_SAMPLERS: [usize; 5] = [
    ATTACH_ALBEDO,   // 0 -> gbuffer_tex
    ATTACH_MAT,      // 1 -> gbuffer_mat
    ATTACH_NORMAL,   // 2 -> gbuffer_nrm
    ATTACH_EMISSIVE, // 3 -> gbuffer_lit
    ATTACH_POSITION, // 4 -> gbuffer_pos
];

/// Binding of `gbuffer_pos` (`deferred_common.glsl:13`) in the sun pass.
pub const BIND_GBUFFER_POS: u32 = 1;
/// Binding of `gbuffer_tex` (`deferred_common.glsl:14`).
pub const BIND_GBUFFER_TEX: u32 = 2;
/// Binding of `gbuffer_nrm` (`deferred_common.glsl:15`).
pub const BIND_GBUFFER_NRM: u32 = 3;
/// Binding of `gbuffer_lit` (`deferred_common.glsl:16`).
pub const BIND_GBUFFER_LIT: u32 = 4;
/// Binding of `gbuffer_mat` (`deferred_common.glsl:17`).
pub const BIND_GBUFFER_MAT: u32 = 5;

/// Binding of `tex_base` (`terrain_common.glsl:152`) in the prepass.
pub const BIND_TERRAIN_BASE: u32 = 1;
/// Binding of `tex_comp` (`terrain_common.glsl:153`).
pub const BIND_TERRAIN_COMP: u32 = 2;
/// Binding of `tex_comp2` (`terrain_common.glsl:154`).
pub const BIND_TERRAIN_COMP2: u32 = 3;
/// Binding of `tex_noise` (`terrain_common.glsl:155`).
pub const BIND_TERRAIN_NOISE: u32 = 4;
/// Binding of `tex_bump` (`terrain_common.glsl:165`).
pub const BIND_TERRAIN_BUMP: u32 = 5;

/// Depth attachment format.
///
/// INFERRED: the GLSL never declares one. Attachment 4 G already carries eye-space Z for the
/// shadow code (`deferred_frag.glsl:536`), so nothing in the shaders needs a hardware depth
/// buffer; `terrain_vert.glsl:280-282` argues that early-Z matters, and the prepass is exactly
/// the pass that wants it. `Depth32Float` rather than `Depth24Plus` because the latter is not
/// universally present on Metal, and the port requests no optional features.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

// ---------------------------------------------------------------------------------------------
// Mode constants — the `#define` block of terrain_common.glsl:8-66, as Rust values.
//
// The shipped shaders resolve these at compile time (the app injects them; no file in the install
// declares them). The ported program is one fixed variant; the variant compiled by
// `terrain_gbuffer.wgsl` is GEOM_NORMAL / LIGHTING_PIXEL / OUTPUT_GBUFFER / NORMAL_STD /
// PROJ_BASE 0 / ALPHA_BLEND / BORDER_NONE / DECAL 0 / NITE_NONE / FOG_NONE, and the values below
// are what its header comment claims. `TEXTURE_MODE` is the one knob the program resolves at
// runtime instead (see [`TerrainUniforms::texture_mode`]).
// ---------------------------------------------------------------------------------------------

/// `terrain_common.glsl:8`
pub const MODE_GEOM_NORMAL: u32 = 0;
/// `terrain_common.glsl:16`
pub const MODE_TEXTURE: u32 = 0;
/// `terrain_common.glsl:20`
pub const MODE_COMPOSITE: u32 = 4;
/// `terrain_common.glsl:23`
pub const MODE_DETAIL: u32 = 7;
/// `terrain_common.glsl:28`
pub const MODE_COLOR: u32 = 12;
/// `terrain_common.glsl:30`
pub const MODE_WATER_REFLECTION: u32 = 14;
/// `terrain_common.glsl:32`
pub const MODE_DECAL_NONE: u32 = 0;
/// `terrain_common.glsl:37`
pub const MODE_ALPHA_NONE: u32 = 0;
/// `terrain_common.glsl:38`
pub const MODE_ALPHA_BLEND: u32 = 1;
/// `terrain_common.glsl:39`
pub const MODE_ALPHA_COMPOSITE: u32 = 2;
/// `terrain_common.glsl:41`
pub const MODE_NORMAL_NONE: u32 = 0;
/// `terrain_common.glsl:42`
pub const MODE_NORMAL_SPEC: u32 = 1;
/// `terrain_common.glsl:43`
pub const MODE_NORMAL_STD: u32 = 2;
/// `terrain_common.glsl:44`
pub const MODE_NORMAL_ALPHA: u32 = 3;
/// `terrain_common.glsl:45`
pub const MODE_NORMAL_HDEF: u32 = 4;
/// `terrain_common.glsl:49`
pub const MODE_LIGHTING_VERTEX: u32 = 1;
/// `terrain_common.glsl:50`
pub const MODE_LIGHTING_PIXEL: u32 = 2;
/// `terrain_common.glsl:52`
pub const MODE_BORDER_NONE: u32 = 0;
/// `terrain_common.glsl:56`
pub const MODE_FOG_NONE: u32 = 0;
/// `terrain_common.glsl:58`
pub const MODE_FOG_TEXTURE: u32 = 2;
/// `terrain_common.glsl:60`
pub const MODE_NIGHT_NONE: u32 = 0;
/// `terrain_common.glsl:64`
pub const MODE_OUTPUT_NORMAL: u32 = 0;
/// `terrain_common.glsl:65`
pub const MODE_OUTPUT_DEPTH: u32 = 1;
/// `terrain_common.glsl:66`
pub const MODE_OUTPUT_GBUFFER: u32 = 2;

/// `deferred_common.glsl:2` — the lighting pass this port implements.
pub const DEFERRED_MODE_SUN: u32 = 0;
/// `deferred_common.glsl:3` — per-light additive pass, deferred to a later slice.
pub const DEFERRED_MODE_LIT: u32 = 1;
/// `deferred_common.glsl:4` — strobe/spotlight cull pass, deferred.
pub const DEFERRED_MODE_STL: u32 = 2;

/// The knobs `terrain_gbuffer.wgsl` is compiled with, as a tuple for callers that want to assert
/// they are drawing with the variant the pipeline actually is.
pub const TERRAIN_VARIANT: (u32, u32, u32, u32) = (
    MODE_GEOM_NORMAL,
    MODE_LIGHTING_PIXEL,
    MODE_OUTPUT_GBUFFER,
    MODE_NORMAL_STD,
);

// ---------------------------------------------------------------------------------------------
// Uniform structs.
//
// WGSL lays a struct out by member order with vec3/vec4/mat4 aligned to 16 bytes; Rust lays a
// `#[repr(C)]` struct out by field order with `[f32; 3]`/`[f32; 4]` aligned to 4. Every struct
// here therefore carries its padding explicitly and pins its own offsets, so the bytes written by
// `Queue::write_buffer` land where the WGSL reads them. Matrices are column-major in both
// languages (GL's convention, which `wgpu` also expects), so each inner `[f32; 4]` is one column.
// ---------------------------------------------------------------------------------------------

/// Fixed-function lighting state the shipped shaders read out of GL, made explicit.
///
/// The install reads `gl_LightSource[0]`, `gl_FrontLightModelProduct.sceneColor` and
/// `gl_FrontMaterial` directly (there is no `#version` directive anywhere, so these are the
/// built-ins of the compatibility profile). Each field names the lines the ported code reads it
/// from:
///
/// | field | GL state | read at |
/// |---|---|---|
/// | `position` | `gl_LightSource[0].position` | `deferred_frag.glsl:691,696`; `terrain_frag.glsl:1169` (water, not ported) |
/// | `diffuse` | `gl_LightSource[0].diffuse` | `deferred_frag.glsl:728`; `terrain_vert.glsl:163` (vertex lighting, not ported) |
/// | `ambient` | `gl_LightSource[0].ambient` | `deferred_frag.glsl:729`; `terrain_vert.glsl:170` (not ported) |
/// | `specular` | `gl_LightSource[0].specular` | `terrain_frag.glsl:1171,1261` (water, not ported); `terrain_vert.glsl:174` (not ported) |
/// | `scene_color` | `gl_FrontLightModelProduct.sceneColor` | `deferred_frag.glsl:734`; `terrain_frag.glsl:1578` |
/// | `material_diffuse` | `gl_FrontMaterial.diffuse` | `terrain_vert.glsl:169-170` — "a_color is our gl_FrontMaterial.diffuse"; the port keeps `a_color` as the vertex attribute (`terrain_vert.glsl:23`) and carries this field for the material-level tint the app sets per draw |
/// | `material_specular` | `gl_FrontMaterial.specular` | `terrain_frag.glsl:1499,1502` |
///
/// `sun_light.wgsl` reads `position`, `diffuse`, `ambient` and `scene_color`;
/// `terrain_gbuffer.wgsl` reads `material_specular` and `scene_color`. The rest is carried because
/// the task names those GL reads as the fixed-function surface to replace, and the next slice
/// (per-light additive lighting, water) is the one that consumes them.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct SunLight {
    /// `gl_LightSource[0].position` — eye-space direction/position; w is 0 for a directional sun.
    pub position: [f32; 4],
    /// `gl_LightSource[0].diffuse`.
    pub diffuse: [f32; 4],
    /// `gl_LightSource[0].ambient`.
    pub ambient: [f32; 4],
    /// `gl_LightSource[0].specular`.
    pub specular: [f32; 4],
    /// `gl_FrontLightModelProduct.sceneColor` — the global ambient term.
    pub scene_color: [f32; 4],
    /// `gl_FrontMaterial.diffuse`.
    pub material_diffuse: [f32; 4],
    /// `gl_FrontMaterial.specular`.
    pub material_specular: [f32; 4],
}

/// `SunLight` is seven `vec4`s: 112 bytes, 16-aligned, in both languages.
const _: [(); 112] = [(); std::mem::size_of::<SunLight>()];

/// Uniform block of `terrain_gbuffer.wgsl`.
///
/// Field order and padding must match the WGSL struct of the same name; the WGSL side cites the
/// GLSL line each member comes from, and the offsets below pin the layout against a mismatch.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct TerrainUniforms {
    /// `terrain_common.glsl:72` — used at `terrain_vert.glsl:76,155`.
    pub u_modelview_matrix: [[f32; 4]; 4],
    /// `terrain_common.glsl:73` — used at `terrain_vert.glsl:75`.
    pub u_mvp_matrix: [[f32; 4]; 4],
    /// `terrain_common.glsl:77` — used at `terrain_frag.glsl:758,775,776`.
    pub tex_params: [f32; 4],
    /// `terrain_common.glsl:78` — used at `terrain_frag.glsl:763-766`.
    pub tex_params2: [f32; 4],
    /// `terrain_common.glsl:79` — used at `terrain_frag.glsl:759,775,776,784`.
    pub tex_params3: [f32; 4],
    /// `terrain_common.glsl:92` — used at `terrain_frag.glsl:365,375`.
    pub u_bump_scale: [f32; 2],
    /// `terrain_common.glsl:98` — used at `terrain_frag.glsl:1576`.
    pub fade_dist: [f32; 2],
    /// `terrain_common.glsl:107` — used at `terrain_vert.glsl:146` and `terrain_frag.glsl:1575`.
    pub u_gamma: [f32; 2],
    /// Padding: WGSL aligns the `vec3` below to 16, so the gap is carried explicitly rather than
    /// left to `repr(C)`, which aligns `[f32; 3]` to 4 and would put the field 8 bytes early.
    pub _pad0: [u8; 8],
    /// `terrain_common.glsl:69` — used at `terrain_frag.glsl:1485,1537,1541`.
    pub u_alpha_cut: [f32; 3],
    /// The `TEXTURE_MODE` knob (`terrain_common.glsl:16-30`), resolved at runtime here instead of
    /// at compile time as GLSL does. Only [`MODE_TEXTURE`], [`MODE_COMPOSITE`] and [`MODE_COLOR`]
    /// have a ported arm.
    pub texture_mode: u32,
    /// The fixed-function lighting state this pass reads (`terrain_frag.glsl:1499,1502,1578`).
    pub light: SunLight,
}

/// Layout pins: WGSL puts `u_alpha_cut` (vec3, 16-aligned) at 208 and the nested `SunLight` at
/// 224, for a 336-byte block.
const _: [(); 64] = [(); std::mem::offset_of!(TerrainUniforms, u_mvp_matrix)];
const _: [(); 128] = [(); std::mem::offset_of!(TerrainUniforms, tex_params)];
const _: [(); 176] = [(); std::mem::offset_of!(TerrainUniforms, u_bump_scale)];
const _: [(); 192] = [(); std::mem::offset_of!(TerrainUniforms, u_gamma)];
const _: [(); 208] = [(); std::mem::offset_of!(TerrainUniforms, u_alpha_cut)];
const _: [(); 220] = [(); std::mem::offset_of!(TerrainUniforms, texture_mode)];
const _: [(); 224] = [(); std::mem::offset_of!(TerrainUniforms, light)];
const _: [(); 336] = [(); std::mem::size_of::<TerrainUniforms>()];

/// Size of the terrain uniform block, for `min_binding_size`.
pub const TERRAIN_UNIFORM_SIZE: u64 = std::mem::size_of::<TerrainUniforms>() as u64;

/// Uniform block of `sun_light.wgsl`.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct SunUniforms {
    /// `deferred_common.glsl:6` — `corner_eye_tl`. Declared `vec3` in GLSL; widened to a `vec4`
    /// here so the block needs no implicit padding (WGSL aligns a vec3 to 16 anyway).
    pub corner_eye_tl: [f32; 4],
    /// `deferred_common.glsl:7`.
    pub corner_eye_tr: [f32; 4],
    /// `deferred_common.glsl:8`.
    pub corner_eye_br: [f32; 4],
    /// `deferred_common.glsl:9`.
    pub corner_eye_bl: [f32; 4],
    /// `deferred_common.glsl:10` — far, 1/far, near, 1/near; `.y` is used at `:40`.
    pub far_plane: [f32; 4],
    /// `deferred_common.glsl:11` — x, y, 1/x, 1/y; `.zw` is used at `deferred_frag.glsl:534`.
    pub u_screen_size: [f32; 4],
    /// `deferred_frag.glsl:26` — "Global master, 1-global master, cloud level, 1-cloud level"
    /// (`terrain_common.glsl:139`); `.x`/`.y` are read at `deferred_frag.glsl:673`.
    pub shadow_master: [f32; 4],
    /// `deferred_frag.glsl:21` — read at `:755` as the textured-fog fade.
    pub fade_dist: [f32; 2],
    /// Padding: WGSL puts the 16-aligned `SunLight` at 128, after an 8-byte `vec2` at 112.
    pub _pad0: [u8; 8],
    /// `deferred_frag.glsl:691,696,728,729,734`.
    pub light: SunLight,
    /// `deferred_vert.glsl:5` — read at `deferred_vert.glsl:48`.
    pub u_mvp_matrix: [[f32; 4]; 4],
}

/// Layout pins for the 304-byte sun block.
const _: [(); 64] = [(); std::mem::offset_of!(SunUniforms, far_plane)];
const _: [(); 80] = [(); std::mem::offset_of!(SunUniforms, u_screen_size)];
const _: [(); 96] = [(); std::mem::offset_of!(SunUniforms, shadow_master)];
const _: [(); 112] = [(); std::mem::offset_of!(SunUniforms, fade_dist)];
const _: [(); 128] = [(); std::mem::offset_of!(SunUniforms, light)];
const _: [(); 240] = [(); std::mem::offset_of!(SunUniforms, u_mvp_matrix)];
const _: [(); 304] = [(); std::mem::size_of::<SunUniforms>()];

/// Size of the sun uniform block, for `min_binding_size`.
pub const SUN_UNIFORM_SIZE: u64 = std::mem::size_of::<SunUniforms>() as u64;

/// The five material textures `terrain_gbuffer.wgsl` binds, named as `terrain_common.glsl` names
/// them. A draw that picks `MODE_TEXTURE` still has to bind all five (WGSL requires every
/// resource in the layout to be bound), so a caller without a composite/noise/bump set should bind
/// a 1x1 placeholder rather than a different pipeline.
#[derive(Debug)]
pub struct TerrainTextures<'a> {
    /// `terrain_common.glsl:152` — read at `terrain_frag.glsl:684,757`.
    pub base: &'a wgpu::TextureView,
    /// `terrain_common.glsl:153` — read at `terrain_frag.glsl:758`.
    pub comp: &'a wgpu::TextureView,
    /// `terrain_common.glsl:154` — read at `terrain_frag.glsl:759`.
    pub comp2: &'a wgpu::TextureView,
    /// `terrain_common.glsl:155` — read at `terrain_frag.glsl:763-766`.
    pub noise: &'a wgpu::TextureView,
    /// `terrain_common.glsl:165` — read at `terrain_frag.glsl:365`.
    pub bump: &'a wgpu::TextureView,
}
