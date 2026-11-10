// sun_light.wgsl — the full-screen sun-lighting pass: g-buffer in, HDR scene out.
//
// Ported from:
//   deferred_vert.glsl:47-49   DEFERRED_MODE == MODE_SUN -> gl_Position = u_mvp_matrix * a_vertex
//   deferred_frag.glsl:534-542 g-buffer fetch + eye-Z recovery
//   deferred_frag.glsl:544-553 frustum reconstruction via eye_pos_from_z (deferred_common.glsl:38-46)
//   deferred_frag.glsl:605-619 alpha == 0 early out (the MODE_SUN arm at :612-613)
//   deferred_frag.glsl:624-629 static_ao / spec_level (SSAO == 0 arm at :624) and the exponent
//   deferred_frag.glsl:643-650 albedo decode + Lambert azimuthal-equal-area normal decode
//   deferred_frag.glsl:660-673 shadow_level master multiply (:673 is the only live line)
//   deferred_frag.glsl:680-772 the MODE_SUN lighting block
//   srgb.glsl:4-18             to_linear / to_srgb, used at :609,643,768
//
// MODE KNOBS (app-injected; the report calls them out as compile-time, not runtime):
//
//   DEFERRED_MODE = MODE_SUN (0)      deferred_common.glsl:2
//       compile-time false: MODE_STL (:528-530 outputs v_color) and MODE_LIT (:582-598,774-864,
//       the per-light additive pass). The vertex stage's MODE_LIT/MODE_STL arms
//       (deferred_vert.glsl:52-101) are not ported, so `light_time`, `pos_light_eye`,
//       `rgb_light`, `direction`, `lin_atten`, `semi_width` (deferred_common.glsl:22-28) are
//       absent, and `v_color` (deferred_common.glsl:31) is absent too: MODE_SUN never writes it
//       (deferred_vert.glsl:45-104 sets it only under MODE_STL) and never reads it.
//   CSM_COUNT     = 0                 deferred_frag.glsl:34
//       compile-time false: the eight split samplers and their shadow_s/t/r/d uniforms
//       (:37-106), pcf_sample (:222-298), csm_precalc_derivatives (:302-351),
//       csm_shadow_level (:353-510), `shadow_fade` (:35), and the main-shader calls at
//       :570-580 and :663-665. shadow_level therefore stays at the g-buffer AO (:660).
//   WANT_SHAD     = 0                 deferred_frag.glsl:18
//       compile-time false: `tex_shad` (:19), the gl_ObjectPlaneS/T[5] eye-matrix abuse and the
//       dFdx/dFdy of the derived ST (:555-568), and the cloud-shadow min at :668-670.
//   SSAO          = 0                 deferred_common.glsl:22's sibling knob (deferred_frag.glsl:621)
//       compile-time false: `u_tex_ssao` (deferred_common.glsl:19) fetch at :622 and the
//       light_color *= static_ao at :736-738. static_ao comes from the g-buffer instead (:624).
//   SCATTER       = 0                 deferred_frag.glsl:29
//       compile-time false: `mv_mat_inv` (:30), `tex_amb` (:31), calc_optical_depths /
//       calc_sun_depths / out_scatter_tint / direct_light_rgb / in_scatter_add (:703-707,759),
//       i.e. the whole new_scattering.glsl include (:3). sun_direct_rgb and sun_ambient_rgb
//       therefore come from the light uniforms via the `#else` arm at :726-731, and in_scatter
//       stays vec3(0.0) (:748).
//   HAS_ARRAY_TEX = irrelevant        (deferred_frag.glsl:36,45,54,... — CSM_COUNT == 0 kills it)
//
// Declared-but-unreachable GLSL state, deliberately not ported: `u_gamma` (deferred_frag.glsl:14)
// and `skydome_tex` (:16) have no live reader in this file — the only lines that mention them
// (:881-883) are commented out — and `fog_level` (:628) is read but consumed only by commented-out
// code (:875-883). Carrying them would be invention, so they are left out.
//
// UNPORTABLE GLSL CONSTRUCTS AND WHAT REPLACED THEM:
//   * `gl_FragCoord` (:534) -> `@builtin(position)`. Same pixel-center coordinates; GL counts Y
//     from the bottom of the framebuffer and WGSL from the top, and both the writer (the terrain
//     prepass) and this reader use the same convention, so screen_rat still addresses the g-buffer
//     texels correctly without a flip.
//   * `gl_FragColor` (:613,770) -> the fragment return value.
//   * `texture2D` (:536-540) -> `textureSample` with an explicit sampler; the sources name no
//     sampler object and assign no texture unit (prefix.glsl:11 maps texture2D -> texture).
//   * `#define tex_smapN tex_smap0` sampler aliasing (:46-103) -> not needed at CSM_COUNT == 0.
//   * ternary at :629 -> an if/else, since WGSL's select() would evaluate pow() on both arms.
//   * `clamp(vec3, float, float)` (:734, :740) -> `clamp(vec3, vec3, vec3)`; WGSL has no
//     vector-scalar form of clamp, min or max.
//   * `gl_LightSource[0].{position,diffuse,ambient}` (:691,696,728,729) and
//     `gl_FrontLightModelProduct.sceneColor` (:734) -> the SunLight uniform block.
//   * `vec3(gl_LightSource[0].position)` (:696) -> `u.light.position.xyz` (GLSL's vec3(vec4)
//     constructor truncates to xyz; WGSL has no such constructor).
//   * `mat3(mat4)` (prefix.glsl:48-53) is not needed by this variant: the only reader would be
//     the SCATTER arm.

// srgb.glsl:4-18, HAS_SRGB arm; the bvec3->vec3 mix factors of :9 and :17 become select().
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

// Fixed-function lighting state made explicit; field citations live in crates/xp/src/gbuffer.rs.
struct SunLight {
    position: vec4<f32>,          // gl_LightSource[0].position
    diffuse: vec4<f32>,           // gl_LightSource[0].diffuse
    ambient: vec4<f32>,           // gl_LightSource[0].ambient
    specular: vec4<f32>,          // gl_LightSource[0].specular
    scene_color: vec4<f32>,       // gl_FrontLightModelProduct.sceneColor
    material_diffuse: vec4<f32>,  // gl_FrontMaterial.diffuse
    material_specular: vec4<f32>, // gl_FrontMaterial.specular
};

struct SunUniforms {
    // deferred_common.glsl:6-9 — the frustum corners, needed by eye_pos_from_z.
    corner_eye_tl: vec4<f32>,  // :6  (declared vec3 in GLSL; padded here to keep vec4 on 16s)
    corner_eye_tr: vec4<f32>,  // :7
    corner_eye_br: vec4<f32>,  // :8
    corner_eye_bl: vec4<f32>,  // :9
    far_plane: vec4<f32>,      // :10 — far, 1/far, near, 1/near
    u_screen_size: vec4<f32>,  // :11 — x, y, 1/x, 1/y
    shadow_master: vec4<f32>,  // deferred_frag.glsl:26 — read at :673
    fade_dist: vec2<f32>,      // deferred_frag.glsl:21 — read at :755
    light: SunLight,           // fixed-function replacement, :691,696,728,729,734
    u_mvp_matrix: mat4x4<f32>, // deferred_vert.glsl:5 — read at :48
};

// deferred_common.glsl:13-17. Attachment -> sampler mapping is the one the report derives from
// the channel semantics (terrain_frag.glsl:1575-1579 write order vs. the comments at :536-540).
@group(0) @binding(1) var gbuffer_pos: texture_2d<f32>;  // attachment 4 — "G is eye-space Z * 1024"
@group(0) @binding(2) var gbuffer_tex: texture_2d<f32>;  // attachment 0 — "RGB is albedo"
@group(0) @binding(3) var gbuffer_nrm: texture_2d<f32>;  // attachment 2 — "RG is lambertian encoded normal"
@group(0) @binding(4) var gbuffer_lit: texture_2d<f32>;  // attachment 3 — "Emissive channel"
@group(0) @binding(5) var gbuffer_mat: texture_2d<f32>;  // attachment 1 — "R = AO, G = specular"
@group(0) @binding(6) var gbuffer_sampler: sampler;
@group(0) @binding(0) var<uniform> u: SunUniforms;

// deferred_frag.glsl:11 `#define SPECULAR_GAIN 2.0`, read at :746.
const SPECULAR_GAIN: f32 = 2.0;

// deferred_common.glsl:38-46, reading the same-named fields out of the bound block.
fn eye_pos_from_z(screen_rat: vec2<f32>, z: f32) -> vec3<f32> {
    let z_scaled = z * u.far_plane.y;                                        // :40
    let vec_to_me_t = mix(u.corner_eye_tl.xyz, u.corner_eye_tr.xyz, screen_rat.x);  // :41
    let vec_to_me_b = mix(u.corner_eye_bl.xyz, u.corner_eye_br.xyz, screen_rat.x);  // :42
    let vec_to_me = mix(vec_to_me_b, vec_to_me_t, screen_rat.y);                    // :43
    return vec_to_me * z_scaled;                                                  // :45
}

// deferred_vert.glsl:47-49 — the whole MODE_SUN vertex stage is one matrix multiply.
// a_vertex is the app's full-screen quad (deferred_vert.glsl:8); PassChain supplies the two
// triangles, so no vertex stage state beyond :48 is ported.
@vertex
fn vs_main(@location(0) a_vertex: vec4<f32>) -> @builtin(position) vec4<f32> {
    return u.u_mvp_matrix * a_vertex;
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    // :534
    let screen_rat = frag_coord.xy * u.u_screen_size.zw;

    // :536-540 — the five g-buffer fetches, comments verbatim from the shipped lines.
    var eye_pos_pixel = textureSample(gbuffer_pos, gbuffer_sampler, screen_rat);  // "G is eye-space Z * 1024 to fit mantissa of f16."
    let eye_tex_pixel = textureSample(gbuffer_tex, gbuffer_sampler, screen_rat);  // "Alpha is total scene transparency, RGB is albedo"
    let eye_lit_pixel = textureSample(gbuffer_lit, gbuffer_sampler, screen_rat);  // "Emissive channel, alpha is funky"
    let eye_mat_pixel = textureSample(gbuffer_mat, gbuffer_sampler, screen_rat);  // "Alpha unused, R = AO, G = specular"
    let eye_nrm_pixel = textureSample(gbuffer_nrm, gbuffer_sampler, screen_rat);  // "RG is lambertian encoded normal"

    // :542 — "this is the original depth, from the green channel of one of the texes".
    let orig_eye_z = eye_pos_pixel.y;

    // :544-553 — the MODE_LIT arm of :546-547 discards; MODE_SUN reconstructs at the far plane.
    if (orig_eye_z == 0.0) {
        eye_pos_pixel = vec4<f32>(eye_pos_from_z(screen_rat, 100.0), 1.0);              // :549
    } else {
        eye_pos_pixel = vec4<f32>(eye_pos_from_z(screen_rat, eye_pos_pixel.y * -1024.0), 1.0);  // :553
    }

    // :555-568 WANT_SHAD == 0 and :570-580 CSM_COUNT == 0 -> both compile-time false, so no
    // derived ST and no per-split derivatives exist. :582-598 is the MODE_LIT arm -> false.

    let alpha = eye_tex_pixel.a;  // :605
    // :606-609 — "linear conversion of eye_lit_pix must be hoisted out of conditional code flow"
    // (an NVidia 353.30 driver bug), so the to_linear stays unconditional.
    let elp_lin = to_linear(eye_lit_pixel.rgb);
    if (alpha == 0.0) {
        // :610-619 — MODE_SUN arm (:612-613): the sky shows the emissive layer unchanged.
        return vec4<f32>(elp_lin, 1.0);
    }

    // :621-625 — SSAO == 0, so static_ao comes out of the g-buffer.
    let static_ao = eye_mat_pixel.r;  // :624
    let spec_level = eye_mat_pixel.g; // :627
    // :628 reads fog_level = eye_mat_pixel.b, but nothing live consumes it (:875-883 are
    // commented out), so the port does not carry it. The B channel is still written by the
    // prepass (terrain_frag.glsl:1576) for a future fog pass.
    // :629 — `(spec_level > 0.0) ? 128.0 : 1.0`.
    var exponent: f32 = 1.0;
    if (spec_level > 0.0) {
        exponent = 128.0;
    }

    // :643
    let eye_tex_day = to_linear(eye_tex_pixel.rgb);

    // :646-650 — decode the Lambert azimuthal-equal-area normal.
    let x2y2 = dot(eye_nrm_pixel.xy, eye_nrm_pixel.xy);
    let normal_eye = vec3<f32>(
        eye_nrm_pixel.xy * sqrt(max(0.0, 1.0 - x2y2 * 0.25)),
        1.0 - x2y2 * 0.5,
    );

    // :659-674 — shadow_level. :660 seeds it from AO; :663-665 (CSM) and :668-670 (cloud shadow)
    // are compile-time false; :673 is the master fade.
    let shadow_level = (static_ao * u.shadow_master.x) + u.shadow_master.y;

    // :680-696 — the MODE_SUN specular/diffuse ratios.
    // "don't use half-angle, use real reflection for specularity" (:688-689), so the reflect()
    // of the eye vector about the normal is the direction we compare the sun against (:691-693).
    let hdot = max(
        0.0,
        dot(
            u.light.position.xyz,
            normalize(reflect(eye_pos_pixel.xyz, normalize(normal_eye))),
        ),
    );
    // :694 — the max(0.0, ...) around pow is the 8800 pow(0,0) workaround explained at :682-686.
    let spec_rat = max(pow(hdot, exponent), 0.0);
    // :696
    let diff_rat = max(0.0, dot(normalize(normal_eye), normalize(u.light.position.xyz)));

    // :726-731 — the `#else` of SCATTER: plain fixed-function sun terms, now explicit uniforms.
    let sun_direct_rgb = u.light.diffuse.rgb;   // :728
    let sun_ambient_rgb = u.light.ambient.rgb;  // :729

    // :734 — scene ambient plus the sun's ambient, clamped to 2.0. GLSL's clamp(vec3, float, float)
    // has no WGSL spelling, so the bounds become vec3s.
    var light_color = clamp(
        u.light.scene_color.rgb + sun_ambient_rgb,
        vec3<f32>(0.0),
        vec3<f32>(2.0),
    );
    // :736-738 SSAO == 0 -> compile-time false.
    // :740 — GLSL's clamp(vec3, float, float) has no WGSL spelling; the bounds become vec3s.
    light_color = light_color + shadow_level * clamp(diff_rat * sun_direct_rgb, vec3<f32>(0.0), vec3<f32>(2.0));

    // :742-746 — "Temp var is a work-around for Intel HD shader compiler".
    let temp0 = eye_tex_day * light_color
        + shadow_level * SPECULAR_GAIN * spec_rat * spec_level * sun_direct_rgb;

    // :748-761 — textured-fog crossfade. in_scatter stays 0 because SCATTER == 0 (:758-760).
    var in_scatter = vec3<f32>(0.0);
    var plan_rat: f32 = 0.0;
    if (orig_eye_z != 0.0) {  // :751 — "Exclude non-filled pixels."
        plan_rat = 1.0 - clamp(u.fade_dist.x * eye_pos_pixel.z + u.fade_dist.y, 0.0, 1.0);  // :755
        plan_rat = plan_rat * plan_rat;          // :756
        plan_rat = 1.0 - plan_rat;               // :757
    }

    // :763-768 — the exact order the shipped comment describes: light in linear, convert to sRGB,
    // apply the sRGB-space fog fade, add the sRGB emissive term, go back to linear for HDR.
    let temp = to_linear(to_srgb(temp0 + in_scatter) * plan_rat + eye_lit_pixel.rgb);

    // :770
    return vec4<f32>(temp, 1.0);
}
