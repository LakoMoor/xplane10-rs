//! The forward pass: `terrain_forward.wgsl`, one draw per OBJ8 descriptor straight into the
//! frame, with a depth buffer. This is the path the original takes with HDR off, which is its
//! default; see the shader header for what the variant covers.

use anyhow::Result;
use bytemuck::{Pod, Zeroable};

use crate::mesh_gpu::GpuMesh;
use xp_core::scene;

const WGSL: &str = include_str!("../shaders/terrain_forward.wgsl");
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// `Uniforms` of `terrain_forward.wgsl`, field for field.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ForwardUniforms {
    pub u_mvp_matrix: [[f32; 4]; 4],
    pub u_modelview_matrix: [[f32; 4]; 4],
    pub light_position: [f32; 4],
    pub light_diffuse: [f32; 4],
    pub light_ambient: [f32; 4],
    pub scene_color: [f32; 4],
    pub u_alpha_cut: [f32; 4],
}

const _: [(); 208] = [(); std::mem::size_of::<ForwardUniforms>()];

pub struct Forward {
    layout: wgpu::BindGroupLayout,
    triangles: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
    depth: wgpu::TextureView,
}

impl Forward {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, extent: (u32, u32)) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain_forward.wgsl"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("forward"),
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
            label: Some("forward"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let build = |topology, label| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(GpuMesh::vertex_layout())],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        // ASSUMED: ordinary source-alpha blending. Blend state is app-side GL
                        // state and has not been read out of the binary.
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                // Cull mode is app-side GL state too; none is invented.
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
        };
        let triangles = build(wgpu::PrimitiveTopology::TriangleList, "forward triangles");
        let lines = build(wgpu::PrimitiveTopology::LineList, "forward lines");
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("forward uniforms"),
            size: std::mem::size_of::<ForwardUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("forward material"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let depth = depth_view(device, extent);
        Self {
            layout,
            triangles,
            lines,
            uniform,
            sampler,
            depth,
        }
    }

    pub fn bind_group(&self, device: &wgpu::Device, base: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("forward material"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(base),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Draw every object into `target`. `groups[i][slot]` is object `i`'s bind group for texture
    /// slot `slot`; `fallback[i]` serves a draw that names no slot.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        clear: wgpu::Color,
        uniforms: &ForwardUniforms,
        meshes: &[&GpuMesh],
        groups: &[Vec<wgpu::BindGroup>],
        fallback: &[wgpu::BindGroup],
    ) -> Result<()> {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(uniforms));
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("forward"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
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
            for (i, mesh) in meshes.iter().enumerate() {
                mesh.draw(&mut pass, &mut |pass, d: &scene::DrawCall| {
                    pass.set_pipeline(match d.topology {
                        scene::Topology::Triangles => &self.triangles,
                        scene::Topology::LineSegments => &self.lines,
                    });
                    let group = d.texture.map_or(&fallback[i], |t| &groups[i][t]);
                    pass.set_bind_group(0, group, &[]);
                })?;
            }
        }
        queue.submit([enc.finish()]);
        Ok(())
    }
}

/// The depth buffer for a frame of `extent`.
fn depth_view(device: &wgpu::Device, extent: (u32, u32)) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("forward depth"),
            size: wgpu::Extent3d {
                width: extent.0.max(1),
                height: extent.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

impl Forward {
    /// Follow a new frame size: only the depth buffer depends on it, so pipelines and the bind
    /// groups made from `layout` stay valid.
    pub fn resize(&mut self, device: &wgpu::Device, extent: (u32, u32)) {
        self.depth = depth_view(device, extent);
    }
}
