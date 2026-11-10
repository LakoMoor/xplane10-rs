//! Screen-space quads: the 2-D cockpit panel now, the interface later.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

const WGSL: &str = include_str!("../shaders/overlay.wgsl");

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    scale: [f32; 4],
    tint: [f32; 4],
}

/// One textured rectangle, in framebuffer pixels from the top-left corner.
pub struct Quad<'a> {
    pub texture: &'a wgpu::TextureView,
    /// Left, top, width, height. A texel maps to a pixel when width and height are the
    /// texture's own.
    pub rect: [f32; 4],
    pub tint: [f32; 4],
    /// The part of the texture to show, in texels: left, top, right, bottom. `None` is all of it.
    pub src: Option<[f32; 4]>,
}

pub struct Overlay {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Write alpha 1 for every fragment: for the copy of the finished frame into the window.
    pub opaque: bool,
}

impl Overlay {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("overlay.wgsl"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("overlay"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("overlay"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        static ATTRS: [wgpu::VertexAttribute; 2] = [
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 8,
                shader_location: 1,
            },
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("overlay"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 16,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &ATTRS,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        // Nearest: a panel drawn texel-for-pixel must not be softened by the sampler.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("overlay"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
            opaque: false,
        }
    }

    /// Draw `quads` over what `target` already holds, in order.
    pub fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        extent: (u32, u32),
        quads: &[Quad<'_>],
    ) {
        let mut prepared = Vec::with_capacity(quads.len());
        for q in quads {
            let [x, y, w, h] = q.rect;
            let size = q.texture.texture().size();
            let (tw, th) = (size.width as f32, size.height as f32);
            let [u0, v0, u1, v1] = match q.src {
                Some([l, t, r, b]) => [l / tw, t / th, r / tw, b / th],
                None => [0.0, 0.0, 1.0, 1.0],
            };
            let verts: [[f32; 4]; 6] = [
                [x, y, u0, v0],
                [x + w, y, u1, v0],
                [x, y + h, u0, v1],
                [x, y + h, u0, v1],
                [x + w, y, u1, v0],
                [x + w, y + h, u1, v1],
            ];
            let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("overlay quad"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let ubuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("overlay uniforms"),
                contents: bytemuck::bytes_of(&Uniforms {
                    scale: [
                        2.0 / extent.0 as f32,
                        2.0 / extent.1 as f32,
                        if self.opaque { 1.0 } else { 0.0 },
                        0.0,
                    ],
                    tint: q.tint,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("overlay quad"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: ubuf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(q.texture),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            prepared.push((vbuf, group));
        }
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            for (vbuf, group) in &prepared {
                pass.set_bind_group(0, group, &[]);
                pass.set_vertex_buffer(0, vbuf.slice(..));
                pass.draw(0..6, 0..1);
            }
        }
        queue.submit([enc.finish()]);
    }
}
