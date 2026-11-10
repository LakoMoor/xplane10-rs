// overlay.wgsl — screen-space textured quads for the 2-D cockpit panel and the interface.
//
// Vertices arrive in framebuffer pixels with the origin at the top-left corner and are mapped to
// clip space here; the texel is multiplied by a tint and blended over the 3-D frame with ordinary
// source-alpha blending. Nothing here is ported from a shipped shader: the original draws its 2-D
// panel through fixed-function GL (legacy.glsl stands in for it), and that state has not been read
// out of the binary yet.

struct Uniforms {
    // x, y: 2 / width, 2 / height; z: 1 to force alpha to 1
    scale: vec4<f32>,
    tint: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>) -> VsOut {
    var out: VsOut;
    out.clip_position = vec4<f32>(pos.x * u.scale.x - 1.0, 1.0 - pos.y * u.scale.y, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(v: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, v.uv);
    // scale.z = 1 makes the quad opaque whatever the texel's alpha: the final copy of the frame
    // into the window must not leave the window see-through where the frame's alpha is not 1.
    return vec4<f32>(t.rgb * u.tint.rgb, mix(t.a * u.tint.a, 1.0, u.scale.z));
}
