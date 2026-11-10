// terrain_gbuffer.wgsl — the g-buffer prepass (the only MRT writer in the shipped tree).
//
// Ported from:
//   terrain_vert.glsl:73-78    GEOM_MODE == MODE_GEOM_NORMAL && NORMAL_MODE != MODE_NORMAL_DISPLACE
//                              -> the `#else` arm (v_position_object / gl_Position / v_position_eye
//                              / v_normal_object)
//   terrain_vert.glsl:146      OUTPUT_MODE == MODE_OUTPUT_GBUFFER -> instance_tint = pow(..., u_gamma.y)
//   terrain_vert.glsl:153-155  normal normalize + mat3(u_modelview_matrix) (prefix.glsl:48-53)
//   terrain_vert.glsl:178-181  LIGHTING_MODE == MODE_LIGHTING_PIXEL -> v_color = a_color * instance_tint
//   terrain_vert.glsl:206-208  TEXTURE_MODE <= MODE_SHOULDER && !PROJ_BASE -> v_texcoord_base
//   terrain_frag.glsl:343-417  calculate_normals()
//   terrain_frag.glsl:679-1394 calculate_albedo(), restricted (see below)
//   terrain_frag.glsl:1453-1471 apply_fog_gbuffer()
//   terrain_frag.glsl:1474-1582 output_gbuffer()
//   terrain_frag.glsl:1631-1640 main() dispatch: apply_fog_gbuffer + output_gbuffer
//
// MODE KNOBS (app-injected `#define`s; terrain_common.glsl:8-66; the Rust mirror of the values
// actually compiled here lives in xp::gbuffer):
//
//   GEOM_MODE      = MODE_GEOM_NORMAL (0)        terrain_common.glsl:8
//       compile-time false: MODE_GEOM_BILLBOARD (:51-58), MODE_GEOM_PROJ_GRID (:80-98),
//       MODE_GEOM_INSTANCE (:100-119), MODE_GEOM_INSTANCE_COMP (:121-143). Those arms are the
//       reason billboard / corner_eye_* / far_plane / a_instance_* exist in the GLSL contract;
//       none of them is reachable from this variant, so none of them is ported.
//   LIGHTING_MODE  = MODE_LIGHTING_PIXEL (2)     terrain_common.glsl:50
//       compile-time false: MODE_LIGHTING_VERTEX (:157-176, the arm that reads
//       gl_LightSource[0] and gl_FrontMaterial in the vertex stage) and MODE_LIGHTING_NONE
//       (:183-185). MODE_LIGHTING_PIXEL is "a big punt - just pass the color on to the fragment
//       shader" (:179), so the vertex stage touches no lighting state at all.
//   OUTPUT_MODE    = MODE_OUTPUT_GBUFFER (2)     terrain_common.glsl:66
//       compile-time false: MODE_OUTPUT_NORMAL (:1412-1451, the forward path incl.
//       do_all_lighting at terrain_frag.glsl:1614-1621) and MODE_OUTPUT_DEPTH (:1402-1410).
//   NORMAL_MODE    = MODE_NORMAL_STD (2)         terrain_common.glsl:43
//       live: WANT_NORMAL (:341) is true and the `#else` arm of calculate_normals (:352-414).
//       compile-time false: MODE_NORMAL_NONE (:345-347), MODE_NORMAL_SPEC (:348-351),
//       MODE_NORMAL_HDEF (:372-374), MODE_NORMAL_ALPHA (:1547-1548).
//   PROJ_BASE      = 0
//       live: derivative tangent basis (terrain_frag.glsl:385-410, terrain_vert.glsl:206-208).
//       compile-time false: the app-supplied basis_S/basis_T arm (terrain_frag.glsl:379-382) and
//       the gl_ObjectPlaneS/T[0] projected-UV arm (terrain_vert.glsl:202-205) — see
//       "unportable constructs" below.
//   TEXTURE_MODE   = runtime uniform `u.texture_mode`, one of MODE_TEXTURE (0),
//                    MODE_COMPOSITE (4), MODE_COLOR (12)   terrain_common.glsl:16,20,28
//       GLSL resolved TEXTURE_MODE at compile time, one program per mode; the port collapses the
//       three retained modes into one runtime switch. compile-time false (not ported at all):
//       MODE_AUTO_VARY (:733-752), MODE_SWIZZLE (:703-720), MODE_SWIZZLE2 (:722-731),
//       MODE_DETAIL (:687-701), MODE_AUTO_HILL (:823-1090), MODE_AUTO_HILL_HEADING,
//       MODE_HEADING (:1097-1142), MODE_RUNWAY (:1144-1148), MODE_SHOULDER (:1150-1154),
//       MODE_WATER_PLANET (:1163-1177), MODE_WATER_REFLECTION (:1179-1263),
//       MODE_AUTO_VARY2 (:794-821).
//   ALPHA_MODE     = MODE_ALPHA_BLEND (1)        terrain_common.glsl:38
//       GLSL has no `#if ALPHA_MODE == MODE_ALPHA_BLEND` arm, so tex_color.a passes through.
//       compile-time false: MODE_ALPHA_NONE (:1332-1334, forces a = 1.0) and
//       MODE_ALPHA_COMPOSITE (:1336-1338, the only reader of u_alpha_width / non_munged_alpha).
//       The `#if ALPHA_MODE != MODE_ALPHA_NONE` discard at terrain_frag.glsl:1540-1543 IS live.
//   BORDER_MODE    = MODE_BORDER_NONE (0)        terrain_common.glsl:52 — :1340-1349 false
//   DECAL1/DECAL2  = 0 (MODE_DECAL_NONE)         terrain_common.glsl:32 — :1288-1326 false
//   NITE_MODE      = MODE_NIGHT_NONE (0)         terrain_common.glsl:60 — :1356-1374 false, so
//                              the `#elif TEXTURE_MODE != MODE_WATER_REFLECTION` arm
//                              (:1376-1378, lit_color = vec4(0.0)) is the live one.
//   FOG_MODE       = MODE_FOG_NONE (0)           terrain_common.glsl:56 — apply_fog_gbuffer's
//                              body (:1458-1470, MODE_FOG_TEXTURE) is compile-time false, so the
//                              call at terrain_frag.glsl:1633 is a no-op. u_screen-side tex_tfog
//                              is therefore not bound.
//   USE_CRAZY_ALPHA= 0                          terrain_common.glsl:183 — :1556-1559 false
//   USE_SHAD       = 0                          terrain_common.glsl:231 — terrain_vert.glsl:190-196 false
//   CSM_COUNT      = 0                          terrain_common.glsl:193 — :240-259 false
//   NIGHT_VISION   = 0                          terrain_frag.glsl:1380-1384 false
//   USE_TONE       = 0                          terrain_frag.glsl:4-7,1649-1651 false (post chain)
//   EXPERIMENTAL   = 0                          terrain_frag.glsl:599-606 false (dev/experimental.glsl)
//
// UNPORTABLE GLSL CONSTRUCTS AND WHAT REPLACED THEM:
//   * `gl_FragData[0..4]` via `#define data_outN` (terrain_frag.glsl:41-45) -> five
//     `@location(n)` members of one fragment-output struct; the MRT itself is the render pass.
//   * `mat3(u_modelview_matrix)` (terrain_vert.glsl:155) -> mat3_from_mat4(), prefix.glsl:48-53.
//   * `texture2D` (terrain_frag.glsl:349,365,684,757-766 etc.) -> textureSample with an explicit
//     sampler; GLSL never names one (prefix.glsl:11 maps texture2D -> texture on HAS_CORE).
//   * `dFdx`/`dFdy` (terrain_frag.glsl:385-388) -> dpdx/dpdy. Same quad-uniform semantics.
//   * `out` parameters (terrain_frag.glsl:343 `out vec4 per_pixel_normal`, :679 three outs) ->
//     WGSL has no out params, so each helper returns a struct carrying the same fields.
//   * `gl_ObjectPlaneS[0]`/`gl_ObjectPlaneT[0]` abused as eye/object-space matrix rows
//     (terrain_vert.glsl:203-204,219-222; terrain_frag.glsl:1069-1070,1103-1104,1136-1137) ->
//     only reachable from PROJ_BASE / water / heading modes, all compile-time false here.
//   * `vec4 - 0.5` (terrain_frag.glsl:763-766) -> `vec4 - vec4(0.5)`: WGSL has no vector-scalar
//     add/sub, only vector-scalar multiply.
//   * ternaries (`? :`, terrain_frag.glsl:1485,1629) -> select().
//   * assignment to a swizzle — `shuffled.x/.y/.z` (:375,:377) and `tex_color.rgb`/`tex_color.a`
//     (:781,:782) — WGSL forbids assigning to a swizzle, so each becomes one whole-vector
//     construction of the same value; the operands read are the pre-assignment ones, as in GLSL.
//   * `if (per_pixel_normal == vec4(0.0))` (:366) — GLSL treats a bvec4 condition as "all
//     components true"; WGSL's `==` yields vec4<bool>, so it goes through `all()`.
//   * `gl_FrontMaterial.specular` (terrain_frag.glsl:1499,1502) and
//     `gl_FrontLightModelProduct.sceneColor` (:1578) -> explicit uniforms (u.light.*).
//   * `a_color` stays a per-vertex attribute (terrain_vert.glsl:23). The shipped app loads the
//     OBJ8 material's attr_diffuse into it (terrain_vert.glsl:167-169 "a_color is our
//     gl_FrontMaterial.diffuse"); the port keeps the attribute rather than inventing a source.

struct VsIn {
    // terrain_vert.glsl:21 declares a_vertex as vec4; the shipped mesh upload provides three
    // floats (crates/xp/src/mesh_gpu.rs:32-48) and only GEOM_MODE_BILLBOARD/INSTANCE arms read
    // a fourth component, so vec4(a_vertex, 1.0) reproduces every live arm.
    @location(0) a_vertex: vec3<f32>,
    @location(1) a_normal: vec3<f32>,   // terrain_vert.glsl:22
    @location(2) a_texcoord0: vec2<f32>, // terrain_vert.glsl:33 -> v_texcoord_base (:207)
    @location(3) a_color: vec4<f32>,     // terrain_vert.glsl:23 -> v_color (:180)
};

// Varyings, in the declaration order of terrain_common.glsl:208-221.
struct VsOut {
    @builtin(position) clip_position: vec4<f32>,  // gl_Position, terrain_vert.glsl:75
    @location(0) v_position_object: vec4<f32>,    // terrain_common.glsl:208
    @location(1) v_position_eye: vec4<f32>,       // terrain_common.glsl:209
    @location(2) v_normal_eye: vec3<f32>,         // terrain_common.glsl:210
    @location(3) v_color: vec4<f32>,              // terrain_common.glsl:212
    // terrain_common.glsl:219 declares this varying only for TEXTURE_MODE != MODE_COLOR; the
    // merged program always writes it and the MODE_COLOR arm simply never reads it.
    @location(4) v_texcoord_base: vec2<f32>,      // terrain_common.glsl:220
};

// prefix.glsl:48-53 (duplicated from common.wgsl — see the note there about imports).
fn mat3_from_mat4(m: mat4x4<f32>) -> mat3x3<f32> {
    return mat3x3<f32>(m[0].xyz, m[1].xyz, m[2].xyz);
}

// Fixed-function lighting state that the shipped shaders read out of GL, made explicit.
// Field-by-field citations live in crates/xp/src/gbuffer.rs (SunLight).
struct SunLight {
    position: vec4<f32>,          // gl_LightSource[0].position
    diffuse: vec4<f32>,           // gl_LightSource[0].diffuse
    ambient: vec4<f32>,           // gl_LightSource[0].ambient
    specular: vec4<f32>,          // gl_LightSource[0].specular
    scene_color: vec4<f32>,       // gl_FrontLightModelProduct.sceneColor
    material_diffuse: vec4<f32>,  // gl_FrontMaterial.diffuse
    material_specular: vec4<f32>, // gl_FrontMaterial.specular
};

// The subset of terrain_common.glsl this variant actually touches. Every field names its line.
struct TerrainUniforms {
    u_modelview_matrix: mat4x4<f32>,  // terrain_common.glsl:72  (terrain_vert.glsl:76,155)
    u_mvp_matrix: mat4x4<f32>,        // terrain_common.glsl:73  (terrain_vert.glsl:75)
    tex_params: vec4<f32>,            // terrain_common.glsl:77  (terrain_frag.glsl:758,775,776)
    tex_params2: vec4<f32>,           // terrain_common.glsl:78  (terrain_frag.glsl:763-766)
    tex_params3: vec4<f32>,           // terrain_common.glsl:79  (terrain_frag.glsl:759,775,776,784)
    u_bump_scale: vec2<f32>,          // terrain_common.glsl:92  (terrain_frag.glsl:365,375)
    fade_dist: vec2<f32>,             // terrain_common.glsl:98  (terrain_frag.glsl:1576)
    u_gamma: vec2<f32>,               // terrain_common.glsl:107 (terrain_vert.glsl:146, terrain_frag.glsl:1575)
    u_alpha_cut: vec3<f32>,           // terrain_common.glsl:69  (terrain_frag.glsl:1485,1537,1541)
    texture_mode: u32,                // TEXTURE_MODE knob, terrain_common.glsl:16-30
    light: SunLight,                  // fixed-function replacement, see the header
};

@group(0) @binding(0) var<uniform> u: TerrainUniforms;

// Samplers, named as terrain_common.glsl names them (:152-165). Binding order is ours: the
// sources assign no texture units (no layout(binding=) anywhere in the install).
@group(0) @binding(1) var tex_base: texture_2d<f32>;   // terrain_common.glsl:152
@group(0) @binding(2) var tex_comp: texture_2d<f32>;   // terrain_common.glsl:153
@group(0) @binding(3) var tex_comp2: texture_2d<f32>;  // terrain_common.glsl:154
@group(0) @binding(4) var tex_noise: texture_2d<f32>;  // terrain_common.glsl:155
@group(0) @binding(5) var tex_bump: texture_2d<f32>;   // terrain_common.glsl:165
@group(0) @binding(6) var base_sampler: sampler;

// terrain_common.glsl:16,20,28 — the three TEXTURE_MODE arms this program keeps.
const MODE_TEXTURE: u32 = 0u;
const MODE_COMPOSITE: u32 = 4u;
const MODE_COLOR: u32 = 12u;

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;

    // terrain_vert.glsl:49 — instance_tint starts at 1.0; the instance arms (:100-143) are
    // compile-time false for GEOM_MODE == MODE_GEOM_NORMAL.
    var instance_tint: f32 = 1.0;

    // terrain_vert.glsl:74-77, the `#else` arm of the :60-78 conditional (NORMAL_MODE !=
    // MODE_NORMAL_DISPLACE, so no bathymetric displacement fetch).
    let position_object = vec4<f32>(in.a_vertex, 1.0);  // :74
    out.v_position_object = position_object;
    out.clip_position = u.u_mvp_matrix * position_object;      // :75
    out.v_position_eye = u.u_modelview_matrix * position_object; // :76
    var normal_object = in.a_normal;                           // :77

    // terrain_vert.glsl:146 — `#if OUTPUT_MODE == MODE_OUTPUT_GBUFFER`. For this variant
    // instance_tint is still 1.0, so the pow is an identity; kept because the shipped code runs it.
    instance_tint = pow(instance_tint, u.u_gamma.y);

    // terrain_vert.glsl:153 — "Normalize here should not be necessary - but in plane-maker we
    // have a giant gl-scale which hozes us."
    normal_object = normalize(normal_object);
    // terrain_vert.glsl:155 — mat3(u_modelview_matrix) is prefix.glsl:53's macro.
    out.v_normal_eye = normalize(mat3_from_mat4(u.u_modelview_matrix) * normal_object);

    // terrain_vert.glsl:178-181 — MODE_LIGHTING_PIXEL.
    out.v_color = in.a_color * instance_tint;

    // terrain_vert.glsl:206-208 — TEXTURE_MODE <= MODE_SHOULDER && !PROJ_BASE.
    out.v_texcoord_base = in.a_texcoord0.xy;

    return out;
}

// terrain_frag.glsl:343-417, NORMAL_MODE == MODE_NORMAL_STD && PROJ_BASE == 0.
//
// GLSL returns normal_eye and writes `out vec4 per_pixel_normal`; WGSL has no out parameters, so
// both come back in one struct (the caller passes them on to output_gbuffer exactly as
// terrain_frag.glsl:1599,1635-1640 does).
struct Normals {
    normal_eye: vec3<f32>,
    per_pixel_normal: vec4<f32>,
};

fn calculate_normals(v: VsOut) -> Normals {
    // :365 — texture2D(tex_bump, v_texcoord_base * u_bump_scale.x)
    var ppn = textureSample(tex_bump, base_sampler, v.v_texcoord_base * u.u_bump_scale.x);
    // :366-367 — an all-zero bump texel means "no normal map here", fall back to flat.
    // GLSL's `if (vec4 == vec4)` is true only when every component matches, so the vec4<bool>
    // WGSL's `==` produces goes through `all`.
    if (all(ppn == vec4<f32>(0.0))) {
        ppn = vec4<f32>(0.5, 0.5, 1.0, 0.0);
    }
    // :368-370
    let raw = vec3<f32>(ppn.r * 2.0 - 1.0, ppn.g * 2.0 - 1.0, ppn.b);
    // :372-374 — NORMAL_MODE == MODE_NORMAL_HDEF is compile-time false (no shuffled.rg *= b).
    // :375 and :377 — WGSL cannot assign to a swizzle, so the scaled xy and the recomputed z form
    // one construction. GLSL recomputes z from the *already scaled* xy (:375 runs before :377),
    // which is what `scaled.xy` is here.
    let scaled = vec2<f32>(raw.x * u.u_bump_scale.y, raw.y * u.u_bump_scale.y);
    let shuffled = vec3<f32>(scaled.x, scaled.y, sqrt(max(0.0, 1.0 - length(scaled))));

    // :385-388 — the !PROJ_BASE arm. dFdx/dFdy -> dpdx/dpdy.
    let q0 = dpdx(v.v_position_eye.xyz);
    let q1 = dpdy(v.v_position_eye.xyz);
    let st0 = dpdx(v.v_texcoord_base);
    let st1 = dpdy(v.v_texcoord_base);
    // :408-410 — GLSL's .s/.t are WGSL's .x/.y; the denominator is dropped exactly as :403-405
    // explains, because S and T are normalized anyway.
    let s_axis = normalize(q0 * st1.y - q1 * st0.y);
    let t_axis = normalize(-q0 * st1.x + q1 * st0.x);
    let sgn = sign(dot(cross(s_axis, t_axis), v.v_normal_eye));

    // :414
    return Normals(
        normalize(s_axis * sgn * shuffled.x + t_axis * sgn * shuffled.y + v.v_normal_eye * shuffled.z),
        ppn,
    );
}

// terrain_frag.glsl:679 — three outs plus one in, again returned as a struct.
struct Albedo {
    tex_color: vec4<f32>,
    lit_color: vec4<f32>,
    static_ao: f32,
};

// The GLSL signature (:679) also takes `in vec3 v_normal_eye_use`; its only readers are the
// MODE_WATER_PLANET (:1170) and MODE_WATER_REFLECTION (:1209-1261) arms, both compile-time
// false here, so the parameter is dropped rather than carried unused.
fn calculate_albedo(v: VsOut) -> Albedo {
    var tex_color: vec4<f32>;

    if (u.texture_mode == MODE_TEXTURE) {
        // :683-685
        tex_color = textureSample(tex_base, base_sampler, v.v_texcoord_base);
    } else if (u.texture_mode == MODE_COMPOSITE) {
        // :754-792
        tex_color = textureSample(tex_base, base_sampler, v.v_texcoord_base);                                  // :757
        let comp_color = textureSample(tex_comp, base_sampler, v.v_position_object.xz * u.tex_params.xy);     // :758
        let far_color = textureSample(tex_comp2, base_sampler, v.v_texcoord_base * u.tex_params3.ww);         // :759

        // :763-766 — four octaves of noise. GLSL's `(fetch - 0.5)` is `(fetch - vec4(0.5))` here:
        // WGSL has no vector-minus-scalar.
        let noise = (textureSample(tex_noise, base_sampler, v.v_texcoord_base * u.tex_params2.x) - vec4<f32>(0.5)) * 2.0 * u.tex_params2.z
            + (textureSample(tex_noise, base_sampler, v.v_texcoord_base * u.tex_params2.x * u.tex_params2.y) - vec4<f32>(0.5)) * 2.0 * u.tex_params2.z * u.tex_params2.w
            + (textureSample(tex_noise, base_sampler, v.v_texcoord_base * u.tex_params2.x * u.tex_params2.y * u.tex_params2.y) - vec4<f32>(0.5)) * 2.0 * u.tex_params2.z * u.tex_params2.w * u.tex_params2.w
            + (textureSample(tex_noise, base_sampler, v.v_texcoord_base * u.tex_params2.x * u.tex_params2.y * u.tex_params2.y * u.tex_params2.y) - vec4<f32>(0.5)) * 2.0 * u.tex_params2.z * u.tex_params2.w * u.tex_params2.w * u.tex_params2.w;

        // :774-777
        let control_level = smoothstep(
            -u.tex_params.z + u.tex_params3.x,
            u.tex_params.z + u.tex_params3.x,
            noise.b - u.tex_params.w * 2.0 * (comp_color.a - 0.5),
        );

        // :781-782
        // :781-782 — WGSL cannot assign to a swizzle, so the rgb mix and the alpha replacement
        // land in one construction; the values read on the right are the pre-assignment ones,
        // exactly as they are in GLSL where the two statements sit next to each other.
        tex_color = vec4<f32>(mix(comp_color.rgb, tex_color.rgb, control_level), comp_color.a);

        // :784-785
        let far_level = clamp(u.tex_params3.y * -v.v_position_eye.z + u.tex_params3.z, 0.0, 1.0);
        tex_color = mix(tex_color, far_color, far_level);

        // :789-790 compute modulator1/modulator2, whose only readers are the DECAL1/DECAL2 mixes
        // at :1305-1326 — compile-time false here, so they are dropped rather than carried dead.
    } else {
        // :1156-1158 — MODE_COLOR && LIGHTING_MODE != MODE_LIGHTING_NONE. The :1159-1161 arm
        // (MODE_COLOR && MODE_LIGHTING_NONE, tex_color = v_color) is compile-time false.
        tex_color = vec4<f32>(1.0);
    }

    // :1314 non_munged_alpha and :1336-1338 MODE_ALPHA_COMPOSITE are compile-time false
    // (ALPHA_MODE == MODE_ALPHA_BLEND), and GLSL has no MODE_ALPHA_BLEND arm at all, so
    // tex_color.a passes through untouched. :1332-1334 (MODE_ALPHA_NONE) is false too.
    // :1288-1326 decals, :1340-1349 borders, :1356-1374 night textures: all compile-time false.

    // :1376-1378 — the live arm of the night/emissive chain: no emissive layer without NITE_MODE.
    let lit_color = vec4<f32>(0.0);

    // :1380-1384 NIGHT_VISION == 0 -> compile-time false.
    // :1387-1389 MODE_DETAIL would set static_ao = modulator.a; MODE_DETAIL is not ported, so the
    // :1390-1392 arm is the live one.
    let static_ao = 1.0;

    return Albedo(tex_color, lit_color, static_ao);
}

// terrain_frag.glsl:1453-1471, called from the dispatch at :1633.
//
// FOG_MODE == MODE_FOG_NONE (terrain_common.glsl:56) makes the whole body (:1458-1470,
// MODE_FOG_TEXTURE) compile-time false, so the shipped call is a no-op and this port keeps the
// call site rather than deleting it. Enabling it needs tex_tfog (terrain_common.glsl:167) bound
// plus screen_dims_inv (terrain_common.glsl:104) — the deferred pass re-does the same fade at
// deferred_frag.glsl:755-757 anyway ("Vol-fog in the G-buffer is duplicated in the deferred
// pass", terrain_frag.glsl:1465-1468).
fn apply_fog_gbuffer(tex_color: ptr<function, vec4<f32>>, lit_color: ptr<function, vec4<f32>>) {
    // Empty body: the only statement in the shipped function is the MODE_FOG_TEXTURE arm
    // (:1458-1470), which is compile-time false. The two arguments are the GLSL `inout vec4`
    // pair (:1454-1455); WGSL has no `inout`, so they travel as `ptr<function, ...>`.
}

// terrain_frag.glsl:1474-1582. Attachment order and channel meanings are the shipped contract
// quoted in crates/xp/src/gbuffer.rs; the writes are :1575-1579.
struct GBufferOut {
    @location(0) data_out0: vec4<f32>,  // albedo, RGBA8
    @location(1) data_out1: vec4<f32>,  // ao / specular / fog, RGBA16F
    @location(2) data_out2: vec4<f32>,  // encoded normal, RGBA16F
    @location(3) data_out3: vec4<f32>,  // emissive, RGBA8
    @location(4) data_out4: vec4<f32>,  // position/depth + hard-cut flag, RGBA16F
};

fn output_gbuffer(
    v: VsOut,
    tex_color_in: vec4<f32>,
    lit_color_in: vec4<f32>,
    static_ao: f32,
    per_pixel_normal: vec4<f32>,
    v_normal_eye_use: vec3<f32>,
) -> GBufferOut {
    var tex_color = tex_color_in;
    var lit_color = lit_color_in;

    apply_fog_gbuffer(&tex_color, &lit_color);

    // :1485 — "the master cut for position and all other boolean parameters".
    let cut_pos_3d = select(0.0, 1.0, (tex_color.a + lit_color.a) > u.u_alpha_cut.z);

    // :1498-1504 — NORMAL_MODE != MODE_NORMAL_NONE is true for MODE_NORMAL_STD, so :1499 is the
    // live line; the :1502 arm (no normal map) is compile-time false. gl_FrontMaterial.specular
    // becomes u.light.material_specular.
    let specular = clamp(u.light.material_specular.r * per_pixel_normal.a, 0.0, 1.0);

    // :1500/:1503 also compute shiny_ao = floor((1.0 - static_ao) * 256.0) + specular * 0.5 and
    // the encoding comment at :1487-1496 explains it, but no attachment is ever written from
    // shiny_ao — :1576 writes static_ao and specular separately. Dropped, not invented.

    // :1531-1543 — the noise-gate alpha model.
    let effective_a = clamp(tex_color.a + lit_color.a, 0.0, 1.0);
    var a = effective_a;
    if (a > u.u_alpha_cut.y) {
        a = 1.0;
    }
    // :1540-1543 — live because ALPHA_MODE != MODE_ALPHA_NONE.
    if (a <= u.u_alpha_cut.x) {
        discard;
    }

    // :1547-1551 — MODE_NORMAL_ALPHA is compile-time false, so nrml_a = effective_a.
    let nrml_a = effective_a;
    // :1556-1559 USE_CRAZY_ALPHA == 0 -> compile-time false.

    var out: GBufferOut;
    // :1575 — "diffuse_rgb TINT is passed into v_color...thus we perma-tint the albedo on the way
    // into the g-buffer" (:1506-1507); everything is premultiplied by effective_a (:1509-1512).
    out.data_out0 = vec4<f32>(tex_color.rgb * pow(v.v_color.rgb, vec3<f32>(u.u_gamma.y)) * a, a);
    // :1576 — R = AO, G = specular, B = the textured-fog level the deferred pass would re-use.
    out.data_out1 = vec4<f32>(
        static_ao * a,
        specular * a,
        clamp(v.v_position_eye.z * u.fade_dist.x + u.fade_dist.y, 0.0, 1.0) * a,
        a,
    );
    // :1577 — Lambertian azimuthal equal-area encoding (:1561-1565).
    out.data_out2 = vec4<f32>(
        sqrt(2.0 / (1.0 + v_normal_eye_use.z)) * v_normal_eye_use.xy * nrml_a,
        0.0,
        nrml_a,
    );
    // :1578 — emissive plus a share of scene ambient, both premultiplied.
    out.data_out3 = vec4<f32>((lit_color.rgb + tex_color.rgb * u.light.scene_color.rgb) * a, a);
    // :1579 — "channel 5 MUST be hard cut, not blended" (:1572-1573); G is eye Z / 1024 so it
    // fits the half-float mantissa (:1514-1518, deferred_frag.glsl:536).
    out.data_out4 = vec4<f32>(0.0, cut_pos_3d * v.v_position_eye.z / -1024.0, 0.0, cut_pos_3d);

    return out;
}

@fragment
fn fs_main(in: VsOut) -> GBufferOut {
    // terrain_frag.glsl:1599
    let n = calculate_normals(in);
    // terrain_frag.glsl:1608 — v_normal_eye_use is the normal_eye returned above.
    let al = calculate_albedo(in);
    // terrain_frag.glsl:1614-1621 — do_all_lighting is compiled only for
    // OUTPUT_MODE == MODE_OUTPUT_NORMAL -> compile-time false.
    // terrain_frag.glsl:1631-1640 — the MODE_OUTPUT_GBUFFER dispatch arm.
    return output_gbuffer(in, al.tex_color, al.lit_color, al.static_ao, n.per_pixel_normal, n.normal_eye);
    // terrain_frag.glsl:1649-1651 tone_map(gl_FragColor.rgb) is USE_TONE == 0 -> compile-time false.
}
