//! GPU driver for the two ported passes: the g-buffer prepass and the HDR sun-lighting pass.
//!
//! [`PassChain`] owns the render targets, the two pipelines and their layouts, and the uniform
//! buffers. It does not know about scenes, cameras or textures: the caller supplies the
//! [`crate::gbuffer::TerrainTextures`] and the uniform blocks, begins the render passes, and calls
//! the draw hooks. Pass order is the caller's business — nothing in the shipped shader directory
//! states it (the survey's pass inventory is marked INFERRED for ordering).
//!
//! Everything is created eagerly. [`PassChain::with_size`] wraps creation in a validation error
//! scope, so a WGSL compile error or an unsupported attachment format comes back as an `Err`
//! instead of surfacing later as an uncaptured error — which, under wgpu's default error handler,
//! is a panic.

use std::borrow::Cow;

use anyhow::{anyhow, Context as _, Result};

use crate::gbuffer::{
    self, SunUniforms, TerrainTextures, TerrainUniforms, DEPTH_FORMAT, GBUFFER_FORMATS,
    SUN_UNIFORM_SIZE, TERRAIN_UNIFORM_SIZE,
};

use xp_core::scene;

/// The shared contract (`prefix.glsl:48-53`, `srgb.glsl`, `deferred_common.glsl:6-11,38-46`).
/// Compiled eagerly so its WGSL is validated too, even though no pipeline is built from it: it
/// declares no entry point and no `@group`/`@binding` on purpose.
const COMMON_WGSL: &str = include_str!("../shaders/common.wgsl");
/// Port of `terrain_vert.glsl` + `terrain_frag.glsl` at `OUTPUT_MODE == MODE_OUTPUT_GBUFFER`.
const TERRAIN_WGSL: &str = include_str!("../shaders/terrain_gbuffer.wgsl");
/// Port of `deferred_frag.glsl` at `DEFERRED_MODE == MODE_SUN`, plus `deferred_vert.glsl:47-49`.
const SUN_WGSL: &str = include_str!("../shaders/sun_light.wgsl");

/// Features the ported passes need.
///
/// None beyond wgpu's defaults. The `RGBA16Float` attachments of
/// [`gbuffer::GBUFFER_FORMATS`] (`terrain_frag.glsl:1527-1528`) are renderable and filterable in
/// core WebGPU on Vulkan/Metal/D3D12; on a GLES-only backend they additionally need
/// `wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES`, which a GLES caller should OR in
/// before [`PassChain::request_device`]. If the format is not renderable at all,
/// [`PassChain::with_size`] returns `Err` rather than drawing garbage.
pub const REQUIRED_FEATURES: wgpu::Features = wgpu::Features::empty();

/// Limits the ported passes need.
///
/// The one that bites is `max_color_attachment_bytes_per_sample`. The g-buffer's five attachments
/// cost 40 bytes per sample under wgpu's accounting — `Rgba8Unorm` counts as 8 bytes/sample
/// ("⚠️ `Rgba8Unorm`/`Rgba8Snorm`/`Bgra8Unorm`/`Bgra8Snorm` are deceptively 8 bytes per sample",
/// wgpu-types-30.0.1/src/limits.rs:228), and `Rgba16Float` counts as 8, so 2 RGBA8 + 3 RGBA16F is
/// 8+8+8+8+8 = 40. wgpu's own `Limits::default()` asks for only 32, so a device created with
/// `DeviceDescriptor::default()` cannot host the prepass: `PassChain::with_size` reports it as an
/// `Err` ("The total number of bytes per sample in color attachments 40 exceeds the limit 32").
/// Use this constant, or at least raise that one field, in your own descriptor.
pub const REQUIRED_LIMITS: wgpu::Limits = wgpu::Limits {
    max_color_attachment_bytes_per_sample: 64,
    ..wgpu::Limits::defaults()
};

/// Request a device that can run the ported passes, with [`REQUIRED_FEATURES`] and
/// [`REQUIRED_LIMITS`].
///
/// Convenience for callers that do not already have a device; `main.rs` may well want its own
/// descriptor (bigger limits, a memory hint), so this is a helper and not the only way in.
pub async fn request_device(
    adapter: &wgpu::Adapter,
    label: &str,
) -> Result<(wgpu::Device, wgpu::Queue)> {
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some(label),
            // BC is requested when the adapter has it and not otherwise: every `.dds` in the
            // install but two is block-compressed, yet the aircraft-only frame uses PNGs alone and
            // must still run on an adapter without the feature.
            required_features: REQUIRED_FEATURES
                | (adapter.features() & wgpu::Features::TEXTURE_COMPRESSION_BC),
            required_limits: REQUIRED_LIMITS,
            ..Default::default()
        })
        .await
        .with_context(|| format!("requesting a device for {label}"))?;
    Ok((device, queue))
}

/// The four attributes `terrain_gbuffer.wgsl` reads, i.e. `terrain_vert.glsl:21-33` for
/// `GEOM_MODE == MODE_GEOM_NORMAL`: position, normal, base ST, color.
///
/// The stride is 48, not the 32 that [`crate::mesh_gpu`] uploads: `a_color`
/// (`terrain_vert.glsl:23`, consumed at `:180` and then at `terrain_frag.glsl:1575`) has no slot
/// in the 3-attribute layout, and the port keeps the shipped attribute rather than inventing a
/// uniform source for it. A caller uploading `xp_core::scene` meshes must add a color stream (or
/// bind a constant-tint buffer) before the prepass will draw.
static TERRAIN_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 4] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 0,
        shader_location: 0, // a_vertex, terrain_vert.glsl:21
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 12,
        shader_location: 1, // a_normal, terrain_vert.glsl:22
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 24,
        shader_location: 2, // a_texcoord0, terrain_vert.glsl:33
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 32,
        shader_location: 3, // a_color, terrain_vert.glsl:23
    },
];

/// Stride of the terrain vertex stream: 12 + 12 + 8 + 16, with `a_color` last.
pub const TERRAIN_VERTEX_STRIDE: wgpu::BufferAddress = 48;

/// The one attribute `sun_light.wgsl` reads: `a_vertex` (`deferred_vert.glsl:8`), consumed by
/// `u_mvp_matrix * a_vertex` at `deferred_vert.glsl:48`.
static QUAD_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
    format: wgpu::VertexFormat::Float32x4,
    offset: 0,
    shader_location: 0,
}];

/// Two triangles covering NDC, for the full-screen sun pass. The shipped app supplied this as a
/// quad VBO (`deferred_vert.glsl:8`); the port owns it because the pass has no other vertex input.
/// W is 1.0 so `u_mvp_matrix * a_vertex` (`:48`) behaves as it did in GLSL with an identity ortho
/// matrix.
#[rustfmt::skip]
static QUAD_VERTICES: [[f32; 4]; 6] = [
    [-1.0, -1.0, 0.0, 1.0],
    [ 1.0, -1.0, 0.0, 1.0],
    [ 1.0,  1.0, 0.0, 1.0],
    [-1.0, -1.0, 0.0, 1.0],
    [ 1.0,  1.0, 0.0, 1.0],
    [-1.0,  1.0, 0.0, 1.0],
];

/// The g-buffer prepass + HDR sun-lighting driver.
// The window now draws the forward path; the deferred chain is only driven headless, so its
// resize path and the handles it keeps alive for it are unused for the moment.
#[allow(dead_code)]
pub struct PassChain {
    width: u32,
    height: u32,
    /// The five color attachments, in draw-buffer order (`GBUFFER_FORMATS`).
    gbuffer_textures: Vec<wgpu::Texture>,
    gbuffer_views: Vec<wgpu::TextureView>,
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    /// Sampler for the material textures (`tex_base`/`tex_comp`/`tex_comp2`/`tex_noise`/`tex_bump`).
    material_sampler: wgpu::Sampler,
    /// Sampler for the five `gbuffer_*` reads (`deferred_common.glsl:13-17`).
    gbuffer_sampler: wgpu::Sampler,
    terrain_bgl: wgpu::BindGroupLayout,
    sun_bgl: wgpu::BindGroupLayout,
    /// Held so the pipeline layouts outlive the pipelines built from them; nothing reads them.
    #[allow(dead_code)]
    terrain_layout: wgpu::PipelineLayout,
    #[allow(dead_code)]
    sun_layout: wgpu::PipelineLayout,
    terrain_pipeline: wgpu::RenderPipeline,
    terrain_line_pipeline: wgpu::RenderPipeline,
    sun_pipeline: wgpu::RenderPipeline,
    terrain_uniform: wgpu::Buffer,
    sun_uniform: wgpu::Buffer,
    /// Built once: the sun pass always reads the same five g-buffer views plus the sampler.
    sun_bind_group: wgpu::BindGroup,
    quad_buffer: wgpu::Buffer,
}

impl PassChain {
    /// Create the chain with g-buffer targets of `width` x `height`.
    ///
    /// All three shader modules, both bind group layouts, both pipeline layouts, both pipelines and
    /// all targets are created inside one validation error scope, so a WGSL error in any source —
    /// or a format the adapter cannot render to — returns `Err` from here.
    pub fn with_size(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let width = width.max(1);
        let height = height.max(1);

        check(device, "creating the g-buffer pass chain", || {
            // Shader modules: naga parses and validates WGSL here, so a syntax or type error
            // lands in the scope rather than panicking.
            let _common = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("common.wgsl (prefix.glsl:48-53, srgb.glsl, deferred_common.glsl)"),
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(COMMON_WGSL)),
            });
            let terrain = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("terrain_gbuffer.wgsl"),
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(TERRAIN_WGSL)),
            });
            let sun = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("sun_light.wgsl"),
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SUN_WGSL)),
            });

            let terrain_uniform = uniform_buffer(device, "terrain uniforms", TERRAIN_UNIFORM_SIZE);
            let sun_uniform = uniform_buffer(device, "sun uniforms", SUN_UNIFORM_SIZE);
            let quad_buffer = buffer_with(
                device,
                "full-screen quad",
                bytemuck::cast_slice(&QUAD_VERTICES),
            )?;

            let material_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("material textures"),
                // INFERRED: no GLSL file states a wrap mode or filter. Terrain textures tile
                // (tex_params scales ST, terrain_frag.glsl:757-766), so Repeat + linear.
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            });
            let gbuffer_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("gbuffer textures"),
                // Screen-space reads of the g-buffer (`deferred_frag.glsl:534`) stay inside 0..1,
                // so clamp rather than tile.
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            });

            let terrain_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("terrain gbuffer"),
                entries: &[
                    // TerrainUniforms: terrain_common.glsl:72,73,77,78,79,92,98,107,69 + SunLight.
                    // VERTEX_FRAGMENT because the vertex stage reads the matrices (:72,73) and the
                    // fragment stage reads the rest.
                    uniform_entry(0, TERRAIN_UNIFORM_SIZE),
                    texture_entry(gbuffer::BIND_TERRAIN_BASE),
                    texture_entry(gbuffer::BIND_TERRAIN_COMP),
                    texture_entry(gbuffer::BIND_TERRAIN_COMP2),
                    texture_entry(gbuffer::BIND_TERRAIN_NOISE),
                    texture_entry(gbuffer::BIND_TERRAIN_BUMP),
                    wgpu::BindGroupLayoutEntry {
                        binding: 6,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
            let sun_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("sun light"),
                entries: &[
                    // SunUniforms: deferred_common.glsl:6-11 + deferred_frag.glsl:21,26 +
                    // SunLight + deferred_vert.glsl:5. VERTEX_FRAGMENT: the vertex stage reads
                    // u_mvp_matrix (`deferred_vert.glsl:48`), the fragment stage reads the rest.
                    uniform_entry(0, SUN_UNIFORM_SIZE),
                    texture_entry(gbuffer::BIND_GBUFFER_POS),
                    texture_entry(gbuffer::BIND_GBUFFER_TEX),
                    texture_entry(gbuffer::BIND_GBUFFER_NRM),
                    texture_entry(gbuffer::BIND_GBUFFER_LIT),
                    texture_entry(gbuffer::BIND_GBUFFER_MAT),
                    wgpu::BindGroupLayoutEntry {
                        binding: 6,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

            let terrain_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("terrain gbuffer"),
                bind_group_layouts: &[Some(&terrain_bgl)],
                immediate_size: 0,
            });
            let sun_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("sun light"),
                bind_group_layouts: &[Some(&sun_bgl)],
                immediate_size: 0,
            });

            let (gbuffer_textures, gbuffer_views) = make_gbuffer(device, width, height);
            let (depth_texture, depth_view) = make_depth(device, width, height);

            let terrain_pipeline = build_terrain_pipeline(
                device,
                &terrain_layout,
                &terrain,
                wgpu::PrimitiveTopology::TriangleList,
                "terrain gbuffer",
            );
            // The line-list twin of the same program. 62 shipped objects write at least one
            // `LINES` descriptor; none of the Cessna 172SP or KSEA objects do, but the port draws
            // them rather than dropping them.
            let terrain_line_pipeline = build_terrain_pipeline(
                device,
                &terrain_layout,
                &terrain,
                wgpu::PrimitiveTopology::LineList,
                "terrain gbuffer lines",
            );

            let sun_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sun light"),
                layout: Some(&sun_layout),
                vertex: wgpu::VertexState {
                    module: &sun,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: 16,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &QUAD_VERTEX_ATTRIBUTES,
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &sun,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    // INFERRED: the sun pass is the first writer of the HDR scene target
                    // (`gl_FragColor = vec4(temp,1.0)`, `deferred_frag.glsl:770`), so it replaces
                    // rather than blends. The per-light additive pass (`:821`) is the one that
                    // accumulates, and it is deferred.
                    targets: &[Some(wgpu::ColorTargetState {
                        format: surface_format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                // The MODE_SUN pass recovers eye-Z from attachment 4 G (`deferred_frag.glsl:536`,
                // `:542`) and never reads gl_FragCoord.z or writes gl_FragDepth, so it needs no
                // depth attachment — begin its render pass without one.
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

            let sun_bind_group = build_sun_bind_group(
                device,
                &sun_bgl,
                &sun_uniform,
                &gbuffer_views,
                &gbuffer_sampler,
                "sun light gbuffer",
            );

            Ok(Self {
                width,
                height,
                gbuffer_textures,
                gbuffer_views,
                depth_texture,
                depth_view,
                material_sampler,
                gbuffer_sampler,
                terrain_bgl,
                sun_bgl,
                terrain_layout,
                sun_layout,
                terrain_pipeline,
                terrain_line_pipeline,
                sun_pipeline,
                terrain_uniform,
                sun_uniform,
                sun_bind_group,
                quad_buffer,
            })
        })?
    }

    /// Current g-buffer extent.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Recreate the g-buffer and depth targets at a new size and rebuild the sun bind group to
    /// match. A zero extent is ignored, as `main.rs` already ignores a minimized window.
    #[allow(dead_code)]
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 || (width == self.width && height == self.height) {
            return Ok(());
        }
        self.width = width;
        self.height = height;
        let built = check(device, "resizing the g-buffer", || {
            let (textures, views) = make_gbuffer(device, width, height);
            let (depth_texture, depth_view) = make_depth(device, width, height);
            let bind_group = build_sun_bind_group(
                device,
                &self.sun_bgl,
                &self.sun_uniform,
                &views,
                &self.gbuffer_sampler,
                "sun light gbuffer (resized)",
            );
            (textures, views, depth_texture, depth_view, bind_group)
        })?;
        let (textures, views, depth_texture, depth_view, bind_group) = built;
        self.gbuffer_textures = textures;
        self.gbuffer_views = views;
        self.depth_texture = depth_texture;
        self.depth_view = depth_view;
        self.sun_bind_group = bind_group;
        Ok(())
    }

    /// The five color attachment views, in draw-buffer order.
    pub fn gbuffer_color_views(&self) -> &[wgpu::TextureView] {
        &self.gbuffer_views
    }

    /// The depth attachment view the prepass renders against.
    pub fn depth_view(&self) -> &wgpu::TextureView {
        &self.depth_view
    }

    /// The g-buffer prepass pipeline for one topology. `TRIS` draws go to the triangle-list
    /// variant and `LINES` draws to the line-list variant; both run the same program against the
    /// same attachments with the same depth state.
    pub fn terrain_pipeline_for(&self, topology: scene::Topology) -> &wgpu::RenderPipeline {
        match topology {
            scene::Topology::Triangles => &self.terrain_pipeline,
            scene::Topology::LineSegments => &self.terrain_line_pipeline,
        }
    }

    /// The vertex-buffer layout the terrain pipeline expects (48-byte stride, 4 attributes).
    pub fn terrain_vertex_layout(&self) -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: TERRAIN_VERTEX_STRIDE,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &TERRAIN_VERTEX_ATTRIBUTES,
        }
    }

    /// Bind group for one terrain draw: the chain's uniform buffer, the caller's five material
    /// textures and the chain's material sampler. Bindings are `terrain_common.glsl:152-155,165`
    /// in the order `terrain_gbuffer.wgsl` declares them.
    pub fn terrain_bind_group(
        &self,
        device: &wgpu::Device,
        textures: &TerrainTextures<'_>,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain gbuffer draw"),
            layout: &self.terrain_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.terrain_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: gbuffer::BIND_TERRAIN_BASE,
                    resource: wgpu::BindingResource::TextureView(textures.base),
                },
                wgpu::BindGroupEntry {
                    binding: gbuffer::BIND_TERRAIN_COMP,
                    resource: wgpu::BindingResource::TextureView(textures.comp),
                },
                wgpu::BindGroupEntry {
                    binding: gbuffer::BIND_TERRAIN_COMP2,
                    resource: wgpu::BindingResource::TextureView(textures.comp2),
                },
                wgpu::BindGroupEntry {
                    binding: gbuffer::BIND_TERRAIN_NOISE,
                    resource: wgpu::BindingResource::TextureView(textures.noise),
                },
                wgpu::BindGroupEntry {
                    binding: gbuffer::BIND_TERRAIN_BUMP,
                    resource: wgpu::BindingResource::TextureView(textures.bump),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&self.material_sampler),
                },
            ],
        })
    }

    /// Issue one g-buffer prepass draw into a pass begun over the five color attachments plus
    /// [`PassChain::depth_view`]. The caller sets the vertex/index buffers and does the drawing;
    /// this sets the pipeline for the draw's topology, the uniforms and the bind group.
    ///
    /// `topology` picks between the triangle-list and line-list variants of the same program,
    /// because OBJ8 `TRIS` and `LINES` descriptors share one index stream and one material and
    /// differ only in how that stream is read.
    ///
    /// Takes `&self`, not `&mut self`, on purpose: the caller's render pass already borrows the
    /// attachment views out of this chain, so a mutable borrow here would deadlock the caller.
    pub fn draw_prepass(
        &self,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'_>,
        bind_group: &wgpu::BindGroup,
        uniforms: &TerrainUniforms,
        topology: scene::Topology,
    ) {
        queue.write_buffer(&self.terrain_uniform, 0, bytemuck::bytes_of(uniforms));
        pass.set_pipeline(self.terrain_pipeline_for(topology));
        pass.set_bind_group(0, bind_group, &[]);
    }

    /// Issue the full-screen sun-lighting draw into a pass begun over the HDR scene target. That
    /// pass needs no depth attachment (see the note in [`PassChain::with_size`]).
    pub fn draw_sun_light(
        &self,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'_>,
        uniforms: &SunUniforms,
    ) {
        queue.write_buffer(&self.sun_uniform, 0, bytemuck::bytes_of(uniforms));
        pass.set_pipeline(&self.sun_pipeline);
        pass.set_bind_group(0, &self.sun_bind_group, &[]);
        pass.set_vertex_buffer(0, self.quad_buffer.slice(..));
        pass.draw(0..6, 0..1);
    }
}

/// A fragment target that blends premultiplied source over the destination:
/// `(1 * src) + (1 - srcA) * dst`, the "1,1-SA" buildup `terrain_frag.glsl:1509-1512` asks for.
fn premultiplied_target(format: wgpu::TextureFormat) -> wgpu::ColorTargetState {
    wgpu::ColorTargetState {
        format,
        blend: Some(wgpu::BlendState {
            color: wgpu::BlendComponent::OVER,
            alpha: wgpu::BlendComponent::OVER,
        }),
        write_mask: wgpu::ColorWrites::ALL,
    }
}

/// The g-buffer prepass pipeline: the five `gl_FragData` slots (`terrain_frag.glsl:41-45`,
/// written at `:1575-1579`), the premultiplied "1,1-SA" buildup (`:1509-1512`), the hard-cut
/// fifth attachment (`:1572-1573`) and the inferred depth state ([`gbuffer::DEPTH_FORMAT`]).
///
/// Built twice, once per primitive topology. OBJ8 `TRIS` and `LINES` descriptors share one index
/// stream, one program and one material, and only the way the index stream is read differs, so
/// the port varies the topology instead of dropping the line draws.
fn build_terrain_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    topology: wgpu::PrimitiveTopology,
    label: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: TERRAIN_VERTEX_STRIDE,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &TERRAIN_VERTEX_ATTRIBUTES,
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[
                Some(premultiplied_target(GBUFFER_FORMATS[0])),
                Some(premultiplied_target(GBUFFER_FORMATS[1])),
                Some(premultiplied_target(GBUFFER_FORMATS[2])),
                Some(premultiplied_target(GBUFFER_FORMATS[3])),
                Some(wgpu::ColorTargetState {
                    format: GBUFFER_FORMATS[4],
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                }),
            ],
        }),
        // INFERRED: cull mode is app-side GL state and the shader directory never mentions it, so
        // the port does not invent a cull rule.
        primitive: wgpu::PrimitiveState {
            topology,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// A vertex+fragment uniform binding of `size` bytes.
fn uniform_entry(binding: u32, size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size),
        },
        count: None,
    }
}

/// A fragment-only 2D texture binding, filterable because every fetch in the ported code is a
/// `texture2D`/`textureSample` with no explicit LOD (`deferred_frag.glsl:536-540`,
/// `terrain_frag.glsl:365,684,757-766`).
fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// The sun pass bind group: uniform block, the five g-buffer views in the mapping
/// [`gbuffer::GBUFFER_SAMPLERS`] states, and the screen-space sampler.
fn build_sun_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    views: &[wgpu::TextureView],
    sampler: &wgpu::Sampler,
    label: &str,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            // gbuffer_tex <- attachment 0, gbuffer_mat <- 1, gbuffer_nrm <- 2, gbuffer_lit <- 3,
            // gbuffer_pos <- 4 (gbuffer::GBUFFER_SAMPLERS; the reads are deferred_frag.glsl:536-540).
            wgpu::BindGroupEntry {
                binding: gbuffer::BIND_GBUFFER_TEX,
                resource: wgpu::BindingResource::TextureView(&views[gbuffer::ATTACH_ALBEDO]),
            },
            wgpu::BindGroupEntry {
                binding: gbuffer::BIND_GBUFFER_MAT,
                resource: wgpu::BindingResource::TextureView(&views[gbuffer::ATTACH_MAT]),
            },
            wgpu::BindGroupEntry {
                binding: gbuffer::BIND_GBUFFER_NRM,
                resource: wgpu::BindingResource::TextureView(&views[gbuffer::ATTACH_NORMAL]),
            },
            wgpu::BindGroupEntry {
                binding: gbuffer::BIND_GBUFFER_LIT,
                resource: wgpu::BindingResource::TextureView(&views[gbuffer::ATTACH_EMISSIVE]),
            },
            wgpu::BindGroupEntry {
                binding: gbuffer::BIND_GBUFFER_POS,
                resource: wgpu::BindingResource::TextureView(&views[gbuffer::ATTACH_POSITION]),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// Allocate the five color attachments of the g-buffer.
fn make_gbuffer(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (Vec<wgpu::Texture>, Vec<wgpu::TextureView>) {
    let mut textures = Vec::with_capacity(GBUFFER_FORMATS.len());
    let mut views = Vec::with_capacity(GBUFFER_FORMATS.len());
    for (index, format) in GBUFFER_FORMATS.iter().copied().enumerate() {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("gbuffer attachment {index}")),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            // COPY_SRC is not needed by the passes themselves; it lets the headless path read a
            // g-buffer attachment back to localise a black frame to the pass that produced it.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        textures.push(texture);
        views.push(view);
    }
    (textures, views)
}

/// Allocate the depth attachment (see [`gbuffer::DEPTH_FORMAT`] for why it exists at all).
fn make_depth(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gbuffer depth"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        // RENDER_ATTACHMENT only: the shaders recover eye-Z from attachment 4 G
        // (`deferred_frag.glsl:536`), so nothing samples this buffer.
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// An empty uniform buffer big enough for `size` bytes.
fn uniform_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// A vertex buffer initialised with `bytes`.
fn buffer_with(device: &wgpu::Device, label: &str, bytes: &[u8]) -> Result<wgpu::Buffer> {
    let size = u64::try_from(bytes.len()).with_context(|| {
        format!(
            "{label}: {} bytes does not fit a BufferAddress",
            bytes.len()
        )
    })?;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: true,
    });
    buffer
        .slice(..)
        .get_mapped_range_mut()
        .map_err(|e| anyhow!("{label}: mapping failed: {e}"))?
        .copy_from_slice(bytes);
    buffer.unmap();
    Ok(buffer)
}

/// Run `f` with a validation error scope pushed, turning any captured error into an `Err`.
///
/// wgpu reports a WGSL compile failure as an uncaptured `wgpu::Error::Validation`, which its
/// default error handler turns into a panic. Scoping the eager creation here means
/// [`PassChain::with_size`] can fail cleanly instead.
fn check<T>(device: &wgpu::Device, what: &str, f: impl FnOnce() -> T) -> Result<T> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let value = f();
    // On the wgpu-core backend the pop future is already ready; pollster keeps `new` synchronous.
    match pollster::block_on(scope.pop()) {
        Some(e) => Err(anyhow!("{what}: {e}")),
        None => Ok(value),
    }
}
