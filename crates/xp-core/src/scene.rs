//! Scene assembly: shipped geometry → GPU-ready buffers.
//!
//! This module is window-free on purpose. It turns an already-parsed [`Obj8`] into an
//! interleaved vertex buffer, an index buffer and a draw list, so the counts can be golden-tested
//! without a GPU. Uploading them is the `xp` crate's job.
//!
//! ## What is measured and what is not
//!
//! The buffer layout follows facts measured from shipped content (see `docs/COMPATIBILITY.md`
//! rows 7, 16 and 32):
//!
//! * OBJ8 keeps **one** index stream, written before the draw descriptors, and `TRIS`/`LINES`
//!   name ranges of it. Ranges may appear in any order, so the descriptors are re-merged into
//!   file order by [`obj8::TriDescriptor::source_line`] before the buffers are built.
//! * Every index addresses the `VT` array: the largest index in the stream is below
//!   `POINT_COUNTS` field 1 in 3 080 of 3 080 shipped objects that have both a `POINT_COUNTS` and
//!   draw descriptors. `VLINE` vertices are therefore a separate array and are not emitted here.
//! * A draw descriptor inherits the `TEXTURE` that was active when it was written, and the
//!   material tint that was active — [`obj8::TriDescriptor::diffuse_rgb`]. Both are recorded on
//!   the descriptor.
//!
//! ## Why each draw gets its own vertex block
//!
//! `terrain_vert.glsl:23` declares `a_color` as a per-vertex attribute, `:180` passes it to the
//! fragment under `LIGHTING_MODE == MODE_LIGHTING_PIXEL`, and `terrain_frag.glsl:1575` multiplies
//! the albedo by it — `:1507` spells out the consequence, "diffuse_rgb TINT is passed into
//! v_color…thus we perma-tint the albedo on the way into the g-buffer". In GL the app could supply
//! that per-draw from fixed-function material state; WGSL has no constant vertex attribute, so the
//! only way to give two draws of the *same* vertex array different tints is to duplicate the
//! vertices those draws share. That is what [`build`] does: one vertex block per draw, each block
//! carrying that draw's tint.
//!
//! The duplication is real and bounded: a draw's block holds one vertex per *distinct* index in
//! its declared range, so the object's total vertex count is the sum of distinct indices over all
//! draws, never more.
//!
//! The **axis convention is not yet pinned by the oracle.** The shipped binary's own Cycle Dump
//! legend states `x location positive aft`, `y location positive right`, `z location positive up`,
//! and that is the only direct evidence about OBJ8's local space this session has. No conversion
//! is applied here: positions pass through unchanged, and the camera transform in `xp` is where a
//! mapping decision belongs once it can be checked against a rendered frame.

use std::collections::HashMap;

use anyhow::{Context as _, Result};

use crate::obj8::{self, LineDescriptor, Obj8, TextureKind, TriDescriptor, Vertex};

/// Bytes per vertex in [`Mesh::vertex_bytes`]: 3 position + 3 normal + 2 uv + 4 color = 12 `f32`
/// = 48 bytes.
///
/// The fourth attribute is not spare: `terrain_vert.glsl:23` declares `a_color` and
/// `terrain_frag.glsl:1575` multiplies the albedo by it. A 32-byte position/normal/uv vertex has
/// nowhere to put the OBJ8 material tint, which is why this stride is 48 and not 32.
pub const VERTEX_STRIDE: usize = 12 * std::mem::size_of::<f32>();

/// The tint a draw receives when no `ATTR_diffuse_rgb` was active for it.
///
/// INFERRED, and recorded as an inference rather than slipped through: nothing in the install
/// states the app's default material. What is measured is that the default has to be the common
/// case — only 26 of 3 197 OBJ8 objects write `ATTR_diffuse_rgb` or `ATTR_diffuse` at all, so
/// almost every draw in the shipped tree inherits it — and that `ATTR_diffuse_rgb` behaves as a
/// multiplier on the texture rather than as a replacement for it:
/// `Resources/default scenery/airport scenery/Common_Elements/Miscellaneous/One_Traffic_Cone.obj`
/// supplies the cone's orange entirely from `ATTR_diffuse_rgb 0.925 0.364 0.000` at line 88 over a
/// shared atlas (`../../textures/Common_Airport_Elements.png`), and
/// `Aircraft/Space Ships/Space Shuttle/Objects/Cockpit_TextLight.obj` restores white by writing
/// `ATTR_diffuse_rgb 1.000 1.000 1.000` at line 7946 rather than by resetting. A default below 1.0
/// would dim 99.2 % of shipped content by that same factor.
///
/// Alpha is 1.0 because no OBJ8 directive sets a material alpha: every tint line in the corpus
/// carries exactly three fields.
pub const DEFAULT_DIFFUSE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// How a draw's index range is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topology {
    /// `TRIS`: every three indices are one triangle.
    Triangles,
    /// `LINES`: consecutive index pairs are one line segment.
    LineSegments,
}

/// One draw call: a range of [`Mesh::indices`] with a material.
///
/// A draw carries two index offsets. [`DrawCall::declared_index_offset`] is the `first_index` the
/// source file wrote, and [`DrawCall::index_offset`] is where this draw's indices actually live in
/// [`Mesh::indices`] after the per-draw re-basing. Keeping both is what makes a draw-count
/// mismatch against the original diagnosable: the declared pair still says what the file asked
/// for, and the upload pair says what the port is doing about it.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawCall {
    /// `first_index` as the source file declares it — an offset into the object's index stream.
    pub declared_index_offset: usize,
    /// `index_count` as the source file declares it. Re-basing does not change it: exactly one
    /// upload index is emitted per declared index.
    pub index_count: usize,
    /// Offset into [`Mesh::indices`] to start drawing at.
    pub index_offset: usize,
    /// First vertex of this draw's block, as a vertex index into [`Mesh::vertex_bytes`].
    pub vertex_offset: usize,
    /// Vertices in this draw's block — one per distinct index in its declared range.
    pub vertex_count: usize,
    pub topology: Topology,
    /// Index into [`Mesh::textures`], or `None` for an object that declares no `TEXTURE`.
    pub texture: Option<usize>,
    /// The tint this draw's vertices carry.
    pub color: [f32; 4],
    /// Whether [`DrawCall::color`] came from an `ATTR_diffuse_rgb`/`ATTR_diffuse` in the file, or
    /// is [`DEFAULT_DIFFUSE`]. A file can write the default value explicitly; this says which
    /// happened, so a golden can tell an authored 1.0 from an inherited one.
    pub tinted: bool,
    /// Line of the `TRIS`/`LINES` directive this draw came from.
    pub source_line: usize,
    /// `ANIM_*`/`ATTR_*`/`LIGHT_*` directives that wrapped this descriptor in the source file.
    /// Kept so a later milestone can split the list by animation group without re-parsing.
    pub guards: Vec<String>,
}

impl DrawCall {
    /// Triangle count for a triangle draw, 0 for a line draw.
    pub fn triangles(&self) -> usize {
        match self.topology {
            Topology::Triangles => self.index_count / 3,
            Topology::LineSegments => 0,
        }
    }

    /// The index range to draw, as offsets into [`Mesh::indices`].
    pub fn upload_range(&self) -> std::ops::Range<u32> {
        let start = self.index_offset as u32;
        start..start.saturating_add(self.index_count as u32)
    }
}

/// A texture a draw can bind, with the path exactly as the object names it.
///
/// The path is relative to the `.obj` file's own directory, which is how the original resolves
/// them; resolving it is the caller's job (`xp_core::content` measures the rule).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureSlot {
    pub kind: TextureKind,
    pub path: String,
}

/// Geometry ready to upload.
#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    /// Interleaved position / normal / uv / color, `VERTEX_STRIDE` bytes per vertex.
    pub vertex_bytes: Vec<u8>,
    pub indices: Vec<u32>,
    pub draws: Vec<DrawCall>,
    pub textures: Vec<TextureSlot>,
}

impl Mesh {
    pub fn vertex_count(&self) -> usize {
        self.vertex_bytes.len() / VERTEX_STRIDE
    }

    /// Total triangles across every triangle draw. This is the number a rendered frame must
    /// reproduce, so it is what the draw-count reconciliation compares.
    pub fn triangle_count(&self) -> usize {
        self.draws.iter().map(DrawCall::triangles).sum()
    }
}

/// A draw descriptor before its vertex block exists, in a shape both descriptor kinds map to.
struct Declared<'a> {
    source_line: usize,
    first_index: u32,
    index_count: u32,
    topology: Topology,
    texture: Option<usize>,
    diffuse_rgb: Option<[f32; 3]>,
    guards: &'a Vec<String>,
}

/// Flatten one parsed object into buffers.
///
/// Draws are emitted in the order the file wrote them, which the two descriptor lists on their own
/// do not preserve: `Resources/default scenery/sim objects/landscape/radar.obj` writes
/// `TRIS 0 798`, then `LINES 798 2`, then `TRIS 800 1530`.
pub fn build(obj: &Obj8) -> Result<Mesh> {
    let textures: Vec<TextureSlot> = obj
        .textures
        .iter()
        .map(|t| TextureSlot {
            kind: t.kind,
            path: t.path.clone(),
        })
        .collect();

    let mut declared: Vec<Declared<'_>> =
        Vec::with_capacity(obj.tri_descriptors.len() + obj.line_descriptors.len());
    for d in &obj.tri_descriptors {
        declared.push(declared_of_tri(d));
    }
    for d in &obj.line_descriptors {
        declared.push(declared_of_line(d));
    }
    // Stable sort: two descriptors cannot share a line, but if the file ever repeats one the
    // original's own order is the tie-break we want to keep.
    declared.sort_by_key(|d| d.source_line);

    let mut vertex_bytes: Vec<u8> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut draws: Vec<DrawCall> = Vec::with_capacity(declared.len());
    // Reused across draws to avoid a per-draw allocation; cleared before every block.
    let mut remap: HashMap<u32, u32> = HashMap::new();

    for d in declared {
        let color = match d.diffuse_rgb {
            Some(rgb) => [rgb[0], rgb[1], rgb[2], 1.0],
            None => DEFAULT_DIFFUSE,
        };
        let what = format!(
            "draw descriptor at line {} ({} {})",
            d.source_line, d.first_index, d.index_count
        );

        let start = d.first_index as usize;
        let end = start
            .checked_add(d.index_count as usize)
            .with_context(|| format!("{what}: range overflows usize"))?;
        let stream = obj.indices.get(start..end).with_context(|| {
            format!(
                "{what}: wants indices {start}..{end} but the object's index stream holds only {}",
                obj.indices.len()
            )
        })?;

        let vertex_base = u32::try_from(vertex_bytes.len() / VERTEX_STRIDE)
            .with_context(|| format!("{what}: vertex count exceeds u32"))?;
        let index_base = indices.len();

        remap.clear();
        let mut emitted: u32 = 0;
        for &i in stream {
            let slot = match remap.get(&i) {
                Some(&slot) => slot,
                None => {
                    let vertex = obj.vertices.get(i as usize).with_context(|| {
                        format!(
                            "{what}: index {i} is outside the {} vertices this object declares",
                            obj.vertices.len()
                        )
                    })?;
                    push_vertex(&mut vertex_bytes, vertex, color);
                    let slot = emitted;
                    emitted += 1;
                    remap.insert(i, slot);
                    slot
                }
            };
            indices.push(
                vertex_base
                    .checked_add(slot)
                    .with_context(|| format!("{what}: re-based index overflows u32"))?,
            );
        }

        draws.push(DrawCall {
            declared_index_offset: d.first_index as usize,
            index_count: d.index_count as usize,
            index_offset: index_base,
            vertex_offset: vertex_base as usize,
            vertex_count: emitted as usize,
            topology: d.topology,
            texture: d.texture,
            color,
            tinted: d.diffuse_rgb.is_some(),
            source_line: d.source_line,
            guards: d.guards.clone(),
        });
    }

    Ok(Mesh {
        vertex_bytes,
        indices,
        draws,
        textures,
    })
}

fn declared_of_tri(d: &TriDescriptor) -> Declared<'_> {
    Declared {
        source_line: d.source_line,
        first_index: d.first_index,
        index_count: d.index_count,
        topology: Topology::Triangles,
        texture: d.texture,
        diffuse_rgb: d.diffuse_rgb,
        guards: &d.guards,
    }
}

fn declared_of_line(d: &LineDescriptor) -> Declared<'_> {
    Declared {
        source_line: d.source_line,
        first_index: d.first_index,
        index_count: d.index_count,
        topology: Topology::LineSegments,
        texture: d.texture,
        diffuse_rgb: d.diffuse_rgb,
        guards: &d.guards,
    }
}

/// Append one vertex: position, normal, uv, then the draw's tint.
fn push_vertex(bytes: &mut Vec<u8>, v: &Vertex, color: [f32; 4]) {
    for f in v.pos.into_iter().chain(v.normal).chain(v.uv).chain(color) {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
}

/// Load one `.obj` from disk and build it.
pub fn load(path: impl AsRef<std::path::Path>) -> Result<(Obj8, Mesh)> {
    let path = path.as_ref();
    let obj = obj8::Obj8::load(path)?;
    let mesh = build(&obj).with_context(|| format!("assembling {}", path.display()))?;
    Ok((obj, mesh))
}
