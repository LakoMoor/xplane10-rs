// common.wgsl — the shared contract of the deferred chain.
//
// Ported from:
//   deferred_common.glsl:6-11   corner_eye_tl / corner_eye_tr / corner_eye_br / corner_eye_bl,
//                               far_plane ("far, 1/far, near, 1/near"), u_screen_size
//                               ("x, y, 1/x, 1/y")
//   deferred_common.glsl:38-46  eye_pos_from_z()
//   srgb.glsl:4-18              to_srgb() / to_linear() — the `#if HAS_SRGB` arm (srgb.glsl:2);
//                               the `#else` pow(2.2) arm (srgb.glsl:22-30) is compile-time false
//   prefix.glsl:48-51           mat3_cast(); `#define mat3(X) mat3_cast(X)` at prefix.glsl:53
//
// WHY THE TWO PASSES DUPLICATE THIS INSTEAD OF IMPORTING IT:
// GLSL shares code textually (`#include "prefix.glsl"` / `#include "deferred_common.glsl"`,
// deferred_common.glsl:1-2, terrain_frag.glsl:1-2, deferred_frag.glsl:1-4). WGSL has no textual
// include, and this toolchain has no module-import plumbing either: `wgpu::ShaderModuleDescriptor`
// carries only `label` + `source` (wgpu-30.0.1/src/api/shader_module.rs:223-228) and naga 30 has
// no `import` keyword. So terrain_gbuffer.wgsl and sun_light.wgsl each re-declare the subset of
// this contract they use, verbatim and with the same citations. This file is the single canonical
// statement of that contract; PassChain::new compiles it eagerly so its WGSL is validated too
// (it has no entry point and no `@group`/`@binding` declarations, so no pipeline is built from it).
//
// The five g-buffer samplers of deferred_common.glsl:13-17 (gbuffer_pos, gbuffer_tex,
// gbuffer_nrm, gbuffer_lit, gbuffer_mat) plus u_tex_ssao (:19) cannot be declared here: WGSL
// requires every `var<texture>`/`var<sampler>` to carry a `@group`/`@binding`, which this module
// deliberately stays free of. They are declared in sun_light.wgsl, which is the pass that reads
// them; u_tex_ssao is not ported (SSAO == 0, see that file's header).

// Frustum reconstruction inputs — deferred_common.glsl:6-11.
//
// In GLSL these are free-standing uniforms; WGSL has no loose uniform variables, so every pass
// folds them into its own bound uniform block and passes this struct's fields through.
struct Frustum {
    corner_eye_tl: vec3<f32>,  // deferred_common.glsl:6
    corner_eye_tr: vec3<f32>,  // deferred_common.glsl:7
    corner_eye_br: vec3<f32>,  // deferred_common.glsl:8
    corner_eye_bl: vec3<f32>,  // deferred_common.glsl:9
    far_plane: vec4<f32>,      // deferred_common.glsl:10 — far, 1/far, near, 1/near
    u_screen_size: vec4<f32>,  // deferred_common.glsl:11 — x, y, 1/x, 1/y
};

// deferred_common.glsl:38-46.
//
// "given our screen-space location in XY (ratio) and Z (depth buffer) we can do some interps to
// reconstruct our 3-d coordinate in eye space..it's just an interp in the frustum"
//
// GLSL's `mix(a, b, t)` with a scalar t is WGSL's `mix(a, b, t)` 1:1.
fn eye_pos_from_z(f: Frustum, screen_rat: vec2<f32>, z: f32) -> vec3<f32> {
    let z_scaled = z * f.far_plane.y;  // :40 — z = z * far_plane.y
    let vec_to_me_t = mix(f.corner_eye_tl, f.corner_eye_tr, screen_rat.x);  // :41
    let vec_to_me_b = mix(f.corner_eye_bl, f.corner_eye_br, screen_rat.x);  // :42
    let vec_to_me = mix(vec_to_me_b, vec_to_me_t, screen_rat.y);            // :43
    return vec_to_me * z_scaled;                                            // :45
}

// srgb.glsl:4-10.
//
// GLSL builds the mix factor as `vec3(lessThanEqual(lin, vec3(0.0031308)))` (srgb.glsl:9):
// a bvec3 widened to 0.0/1.0. WGSL's comparison yields vec3<bool> and has no widening cast, so
// the same test goes through `select(false_arm, true_arm, condition)`.
// The exponent is the shipped 0.41666, not 1/2.4 (srgb.glsl:7).
fn to_srgb(lin: vec3<f32>) -> vec3<f32> {
    return select(
        1.055 * pow(lin, vec3<f32>(0.41666)) - 0.055,  // srgb.glsl:7
        12.92 * lin,                                   // srgb.glsl:8
        lin <= vec3<f32>(0.0031308),                   // srgb.glsl:9
    );
}

// srgb.glsl:12-18 — same lessThanEqual substitution as above (srgb.glsl:17).
fn to_linear(srgb: vec3<f32>) -> vec3<f32> {
    return select(
        pow(srgb * (1.0 / 1.055) + (0.055 / 1.055), vec3<f32>(2.4)),  // srgb.glsl:15
        srgb * (1.0 / 12.92),                                         // srgb.glsl:16
        srgb <= vec3<f32>(0.04045),                                   // srgb.glsl:17
    );
}

// prefix.glsl:48-51 — `mat3_cast(mat4 m) { return mat3(m[0].xyz, m[1].xyz, m[2].xyz); }`.
//
// GLSL allows constructing a mat3 from a mat4; WGSL does not, which is why prefix.glsl:53
// `#define mat3(X) mat3_cast(X)` exists at all. WGSL indexes mat4 by column too, so the three
// column xyz swizzles are the same three basis vectors.
fn mat3_from_mat4(m: mat4x4<f32>) -> mat3x3<f32> {
    return mat3x3<f32>(m[0].xyz, m[1].xyz, m[2].xyz);
}
