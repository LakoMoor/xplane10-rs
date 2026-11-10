// terrain_forward.wgsl — the original's FORWARD path for lit, textured geometry.
//
// The oracle's default settings have HDR off (`I/REN: Setting |HDR rendering defaulted to 0`),
// so the frame it shows is `terrain_frag.glsl` with OUTPUT_MODE == MODE_OUTPUT_NORMAL, not the
// g-buffer arm that terrain_gbuffer.wgsl ports. This file ports that arm for the variant
//
//   TEXTURE_MODE = MODE_TEXTURE, LIGHTING_MODE = MODE_LIGHTING_PIXEL, NORMAL_MODE = MODE_NORMAL_NONE,
//   HAS_SPEC = 0, USE_SCAT = 0, USE_SHAD = 0, CSM_COUNT = 0, USE_LITE = 0, FOG_MODE = MODE_FOG_NONE
//
// Ported from:
//   terrain_vert.glsl:74-77,153-155,178-181,206-208   position, eye normal, v_color, base ST
//   terrain_frag.glsl:436-439,471                     calculate_sun_lighting(), non-scatter arm
//   terrain_frag.glsl:647-648                         lighting mix-down
//   terrain_frag.glsl:1412-1434                       output_normal(), noise gates + LINEAR_BLENDING
//   srgb.glsl:4-10                                    to_srgb(), HAS_SRGB arm
//
// NOT ported, and each one is visible in the oracle frame: atmospheric scattering (USE_SCAT, on by
// default: "atmospheric scattering defaulted to 1"), shadows (USE_SHAD / CSM), specular, normal
// maps, spill lights, fog. Which of these the shipped build injects for an aircraft draw is not
// yet read out of the binary.
//
// LINEAR_BLENDING: `gl_FragColor = to_linear(tex_color)` into an sRGB framebuffer displays
// `tex_color` itself. The target here is a plain UNORM texture, so `tex_color` is written as is.

struct Uniforms {
    u_mvp_matrix: mat4x4<f32>,
    u_modelview_matrix: mat4x4<f32>,
    // gl_LightSource[0].position — eye space, w = 0 for the sun
    light_position: vec4<f32>,
    light_diffuse: vec4<f32>,
    light_ambient: vec4<f32>,
    // gl_FrontLightModelProduct.sceneColor
    scene_color: vec4<f32>,
    // u_alpha_cut.x / .y, terrain_frag.glsl:1419-1423
    u_alpha_cut: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var tex_base: texture_2d<f32>;
@group(0) @binding(2) var base_sampler: sampler;

struct VsIn {
    @location(0) a_vertex: vec3<f32>,
    @location(1) a_normal: vec3<f32>,
    @location(2) a_texcoord0: vec2<f32>,
    @location(3) a_color: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) v_normal_eye: vec3<f32>,
    @location(1) v_color: vec4<f32>,
    @location(2) v_texcoord_base: vec2<f32>,
};

// srgb.glsl:4-10; the exponent is the shipped 0.41666.
fn to_srgb(lin: vec3<f32>) -> vec3<f32> {
    return select(
        1.055 * pow(lin, vec3<f32>(0.41666)) - 0.055,
        12.92 * lin,
        lin <= vec3<f32>(0.0031308),
    );
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let position_object = vec4<f32>(in.a_vertex, 1.0);                 // terrain_vert.glsl:74
    out.clip_position = u.u_mvp_matrix * position_object;              // :75
    let m = u.u_modelview_matrix;
    let normal_object = normalize(in.a_normal);                        // :153
    out.v_normal_eye =
        normalize(mat3x3<f32>(m[0].xyz, m[1].xyz, m[2].xyz) * normal_object); // :155
    out.v_color = in.a_color;                                          // :180, instance_tint = 1
    out.v_texcoord_base = in.a_texcoord0;                              // :207
    return out;
}

@fragment
fn fs_main(v: VsOut) -> @location(0) vec4<f32> {
    var tex_color = textureSample(tex_base, base_sampler, v.v_texcoord_base);

    // terrain_frag.glsl:437-439,471
    let n_dot_l = max(0.0, dot(normalize(v.v_normal_eye), normalize(u.light_position.xyz)));
    let diffuse = u.light_diffuse.rgb * n_dot_l;
    let ambient = u.light_ambient.rgb;
    let light_color = clamp((ambient + diffuse) * v.v_color.rgb, vec3<f32>(0.0), vec3<f32>(2.0));

    // :648
    tex_color = vec4<f32>(tex_color.rgb * (to_srgb(light_color) + u.scene_color.rgb), tex_color.a);

    // :1419-1423
    if (tex_color.a > u.u_alpha_cut.y) {
        tex_color.a = 1.0;
    }
    if (tex_color.a <= u.u_alpha_cut.x) {
        discard;
    }
    return tex_color;
}
