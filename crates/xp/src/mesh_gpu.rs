//! GPU-side mesh upload: `xp_core::scene::Mesh` → wgpu buffers.
//!
//! The core crate builds the buffers (see `xp_core::scene`); this module only moves them to the
//! device. Keeping the split means the counts stay golden-testable without a GPU, so a draw-count
//! mismatch can be attributed to assembly or to upload rather than to both at once.
//!
//! Vertex layout is the core's: 3 position + 3 normal + 2 uv + 4 color `f32`, little-endian,
//! 48-byte stride. The color stream is the OBJ8 material tint (`ATTR_diffuse_rgb`), which
//! [`xp_core::scene`] bakes per draw because WGSL has no constant vertex attribute.
//! Indices stay `u32`: OBJ8 index streams are 32-bit in shipped content — `POINT_COUNTS` field 4
//! reaches 102 360 in `HarrierWings.obj` — so a `u16` index buffer could not represent it.

use anyhow::{Context, Result};

use xp_core::scene::{self, Mesh};

/// One uploaded object plus the draw list that indexes it.
#[derive(Debug)]
pub struct GpuMesh {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub vertex_count: usize,
    pub index_count: usize,
    /// Copied from the core mesh so the draw loop never reaches back into parsed content.
    pub draws: Vec<scene::DrawCall>,
    /// Texture paths as the object named them; resolving them to a real file is the caller's job.
    pub textures: Vec<scene::TextureSlot>,
    /// Source path, for buffer labels and for saying which object a draw belongs to.
    pub source: String,
}

/// The four attributes `terrain_gbuffer.wgsl` reads, matching [`xp_core::scene`]'s vertex
/// encoding: position, normal, base ST, and the material tint `a_color`
/// (`terrain_vert.glsl:21-23,33`).
///
/// Declared as a `'static` array because `wgpu::VertexBufferLayout` borrows its attribute slice.
/// The fourth attribute is the reason [`xp_core::scene::VERTEX_STRIDE`] is 48 and not 32: the
/// OBJ8 material tint has nowhere else to go, and `terrain_frag.glsl:1575` multiplies the albedo
/// by it.
static VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 4] = [
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

impl GpuMesh {
    /// Upload one assembled mesh.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mesh: &Mesh,
        source: &str,
    ) -> Result<Self> {
        let vertex_buffer = write_buffer(device, queue, &mesh.vertex_bytes, "vertices", source)?;
        let index_bytes: Vec<u8> = mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        let index_buffer = write_buffer(device, queue, &index_bytes, "indices", source)?;

        Ok(Self {
            vertex_buffer,
            index_buffer,
            vertex_count: mesh.vertex_count(),
            index_count: mesh.indices.len(),
            draws: mesh.draws.clone(),
            textures: mesh.textures.clone(),
            source: source.to_string(),
        })
    }

    /// The vertex-buffer layout matching [`xp_core::scene`]'s vertex encoding.
    pub fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: scene::VERTEX_STRIDE as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX_ATTRIBUTES,
        }
    }
    /// Issue every draw of this object into `pass`.
    ///
    /// `prepare` is called with the draw about to be issued, before its indices are submitted, so
    /// the caller can set the pipeline and bind the material for that draw: it is the only place
    /// where the draw's texture slot, its topology and the pipeline that matches them are all
    /// visible at once. The slot may name a file the install does not contain — `xp_core::content`
    /// measures 277 such references across the shipped tree — and the draw's topology decides
    /// whether it goes to the triangle-list or the line-list variant of the prepass program.
    ///
    /// The buffers are set once for the object; only the pipeline and bind group change per draw.
    pub fn draw<F>(&self, pass: &mut wgpu::RenderPass<'_>, prepare: &mut F) -> Result<()>
    where
        for<'p, 'e, 'd> F: FnMut(&'p mut wgpu::RenderPass<'e>, &'d scene::DrawCall),
    {
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        for d in &self.draws {
            prepare(pass, d);
            let base = u32::try_from(d.index_offset)
                .with_context(|| format!("{}: draw offset {}", self.source, d.index_offset))?;
            let end = base
                .checked_add(
                    u32::try_from(d.index_count).with_context(|| {
                        format!("{}: draw count {}", self.source, d.index_count)
                    })?,
                )
                .with_context(|| format!("{}: draw range overflows u32", self.source))?;
            pass.draw_indexed(base..end, 0, 0..1);
        }
        Ok(())
    }
}

/// Create a buffer and write `bytes` into it.
///
/// `wgpu::Queue::write_buffer` requires the buffer to be at least as large as the data and the
/// data length to be a multiple of 4. OBJ8 vertex buffers (32-byte vertices) and index streams
/// (4-byte indices) satisfy that by construction, but the size is computed from the data rather
/// than assumed, and a short tail is padded instead of panicking inside wgpu.
fn write_buffer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bytes: &[u8],
    what: &str,
    source: &str,
) -> Result<wgpu::Buffer> {
    let len = u64::try_from(bytes.len())
        .with_context(|| format!("{what} of {source} does not fit a BufferAddress"))?;
    let padded = (len + 3) & !3;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&format!("{what} of {source}")),
        size: padded,
        usage: wgpu::BufferUsages::VERTEX
            | wgpu::BufferUsages::INDEX
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, bytes);
    Ok(buffer)
}
