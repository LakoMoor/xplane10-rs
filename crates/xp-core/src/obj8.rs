//! OBJ8 — X-Plane 10 3-D object files.
//!
//! Structure, measured from shipped content rather than from any written spec
//! (`…/Cessna 172SP/objects/fuse.obj` sha `34f1019b…`, 3 970 lines;
//! `…/KSEA_objects/ConCSlantRoof.obj` sha `ae3c8e8e…`, CRLF and tab-separated):
//!
//! ```text
//! A                                   <- magic
//! 800                                 <- version
//! OBJ
//! TEXTURE fuse.png                    <- also TEXTURE_LIT / _DRAPED / _NORMAL / _LOD
//! GLOBAL_specular 1.0
//! POINT_COUNTS 2850 0 0 10944         <- solid vertices, line vertices, particles, INDICES
//! VT x y z nx ny nz u v               <- 2850 solid vertices
//! VLINE x y z r g b                   <- line vertices (position + color), counted by field 2
//! IDX10 i0 i1 … i9                    <- the index stream: 10 indices per line
//! IDX i                               <-  … or 1 index per line
//! TRIS 0 10152                        <- draw descriptor over the index stream
//! ANIM_begin … ANIM_end               <- guards interleaved between descriptors
//! TRIS 10152 396
//! ```
//!
//! Two things about this format are easy to get wrong, and both were measured here:
//!
//! 1. **`POINT_COUNTS` field 4 and `TRIS` field 2 count indices, not triangles.** fuse.obj
//!    declares 10 944 of them and holds 10 944 of them, which is 3 648 triangles.
//! 2. **The index stream is one stream, written before the draw descriptors.** `TRIS` does not
//!    open a block that its indices follow; it names a range of the stream that already came.
//!    `VLINE` is a vertex directive, not a block header either.
//!
//! Directives the parser recognizes but does not yet model are preserved verbatim in
//! [`Obj8::raw_directives`] and reported, never dropped and never fatal.

use anyhow::{bail, Context, Result};

/// Near distance the original applies to an object that declares no LOD block.
///
/// Inferred, not read from source: X-Plane 10.51r2 reports "a first LOD near distance of
/// 2000.000000" for `ConCSlantRoof.obj`, which contains no `L32`/`L5`/`LIGHT_LOD` line.
/// Treating the absence as 0 would swallow a warning the original really prints.
/// See `docs/COMPATIBILITY.md` row 8.
pub const IMPLICIT_FIRST_LOD_NEAR: f32 = 2000.0;

/// What kind of `.obj` file this is. The install mixes X-Plane OBJ8 with plain Wavefront.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjKind {
    /// X-Plane object: `A`, `<version>`, `OBJ`.
    Obj8 { version: u32 },
    /// Wavefront geometry (`v`/`vt`/`vn`/`f`/`usemtl`), used by some cockpit packs.
    Wavefront,
    /// Neither; the content checker reports it instead of guessing.
    Unknown,
}

/// Classify by the first three non-empty lines.
pub fn classify(text: &str) -> ObjKind {
    let mut head = text.lines().map(|l| l.trim_end()).filter(|l| !l.is_empty());
    let a = head.next().unwrap_or("");
    let b = head.next().unwrap_or("");
    let c = head.next().unwrap_or("");
    if a == "A" && !b.is_empty() && b.chars().all(|ch| ch.is_ascii_digit()) && c == "OBJ" {
        return ObjKind::Obj8 {
            version: b.parse().unwrap_or(0),
        };
    }
    let first_word = a.split_whitespace().next().unwrap_or("");
    if matches!(
        first_word,
        "v" | "vt" | "vn" | "f" | "g" | "o" | "s" | "usemtl" | "mtllib"
    ) {
        return ObjKind::Wavefront;
    }
    ObjKind::Unknown
}

/// One `VT` line: position, normal, texture coordinate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// One `VLINE` line: position and color of a line-geometry vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineVertex {
    pub pos: [f32; 3],
    pub color: [f32; 3],
}

/// `POINT_COUNTS solid_vertices line_vertices particles index_count`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointCounts {
    pub solid_vertices: u32,
    pub line_vertices: u32,
    pub particles: u32,
    /// Indices, not triangles.
    pub indices: u32,
}

/// A texture binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureBinding {
    pub kind: TextureKind,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureKind {
    /// `TEXTURE` — the diffuse/lit map.
    Diffuse,
    /// `TEXTURE_LIT`
    Lit,
    /// `TEXTURE_DRAPED`
    Draped,
    /// `TEXTURE_NORMAL` — bump/normal map.
    Normal,
    /// `TEXTURE_LOD`
    Lod,
}

/// `TRIS first_index index_count`: a range of the object's index stream, drawn as triangles,
/// together with the state directives that wrapped it.
#[derive(Debug, Clone, PartialEq)]
pub struct TriDescriptor {
    pub first_index: u32,
    pub index_count: u32,
    /// Directives seen since the previous descriptor (ANIM_*, ATTR_*, …), in file order.
    pub guards: Vec<String>,
    /// The `TEXTURE` binding this range is drawn with, as a slot in [`Obj8::textures`].
    pub texture: Option<usize>,
    /// The material diffuse tint this range is drawn with, from `ATTR_diffuse_rgb` /
    /// `ATTR_diffuse`, or `None` when the descriptor inherits the app's default material.
    ///
    /// `terrain_vert.glsl:167-169` names the source outright: "a_color contains the
    /// \"attr_diffuse\" diffuse material tint, which is applied to both ambient and diffuse
    /// channels in x-plane … a_color is our gl_FrontMaterial.diffuse". RGB only — every tint
    /// line in the shipped corpus carries exactly three fields, so alpha is not settable this
    /// way and stays 1.0.
    pub diffuse_rgb: Option<[f32; 3]>,
    /// Line of the `TRIS` directive in the source file; see [`LineDescriptor::source_line`].
    pub source_line: usize,
}

/// `LINES first_index index_count`: a range of the same index stream, drawn as line strips.
#[derive(Debug, Clone, PartialEq)]
pub struct LineDescriptor {
    pub first_index: u32,
    pub index_count: u32,
    /// Directives seen since the previous descriptor, in file order.
    pub guards: Vec<String>,
    /// The `TEXTURE` binding this range is drawn with, as a slot in [`Obj8::textures`].
    pub texture: Option<usize>,
    /// Material tint for this range, as in [`TriDescriptor::diffuse_rgb`].
    pub diffuse_rgb: Option<[f32; 3]>,
    /// Line of the `LINES` directive in the source file.
    ///
    /// `TRIS` and `LINES` share one index stream and may be written in any order, and both
    /// interleave with the `ANIM_*` blocks that guard them, so a renderer that wants the file's
    /// own draw order needs the position each descriptor was written at — the two descriptor
    /// lists on their own lose it.
    pub source_line: usize,
}
/// An LOD selection block: `L32 near far`, `L5 near far`, `LIGHT_LOD near far`.
#[derive(Debug, Clone, PartialEq)]
pub struct LodBlock {
    pub kind: LodKind,
    pub near: f32,
    pub far: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LodKind {
    Lod32,
    Lod5,
    LightLod,
}

/// A directive the parser keeps but does not yet interpret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDirective {
    pub line: usize,
    pub text: String,
}

/// How an object is being used. The original validates differently in each role.
///
/// Measured, not assumed: in the scripted 10.51r2 run, Log.txt warned about
/// `Custom Scenery/KSEA Demo Area/KSEA_objects/ConCSlantRoof.obj` (a scenery object reached
/// through `library.txt`) but said nothing about `Cessna 172SP/objects/fuse.obj`, which has
/// the same absence of `L32`/`L5`/`LIGHT_LOD`. The first-LOD rule therefore belongs to the
/// scenery loader, not to object parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectRole {
    /// Loaded as a part of an aircraft (`Aircraft/**`).
    Aircraft,
    /// Loaded as scenery through a pack's `library.txt` (`Custom Scenery/**`,
    /// `Resources/default scenery/**`).
    Scenery,
}

/// A validation problem, worded like the original's own Log.txt diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

/// A parsed OBJ8 file.
#[derive(Debug, Clone, PartialEq)]
pub struct Obj8 {
    pub version: u32,
    pub textures: Vec<TextureBinding>,
    /// `GLOBAL_specular`, when present.
    pub global_specular: Option<f32>,
    pub point_counts: Option<PointCounts>,
    pub vertices: Vec<Vertex>,
    pub line_vertices: Vec<LineVertex>,
    /// The whole index stream, in file order, across every descriptor.
    pub indices: Vec<u32>,
    pub tri_descriptors: Vec<TriDescriptor>,
    /// `LINES` draw descriptors, in file order. They share the index stream with TRIS.
    pub line_descriptors: Vec<LineDescriptor>,
    pub lod_blocks: Vec<LodBlock>,
    /// ATTR_*, ANIM_*, IF/ELSE/ENDIF, LIGHT_*, SMOOTH*, EXPORT, DRW_, WED_, TILTED,
    /// SLOPE_LIMIT, … kept for later milestones.
    pub raw_directives: Vec<RawDirective>,
    /// `#`-prefixed WorldEditor metadata lines, kept verbatim (see [`Self::parse`]).
    pub metadata_lines: Vec<RawDirective>,
}

impl Obj8 {
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Obj8> {
        let path = path.as_ref();
        let text = crate::read_text_lossy(path)
            .with_context(|| format!("reading OBJ {}", path.display()))?;
        Obj8::parse(&text).with_context(|| format!("parsing OBJ {}", path.display()))
    }

    /// Parse OBJ8 text. `\n`, `\r\n` and bare `\r` are accepted (several shipped packs are
    /// CRLF) and fields may be separated by tabs as well as spaces.
    pub fn parse(text: &str) -> Result<Obj8> {
        let ObjKind::Obj8 { version } = classify(text) else {
            bail!(
                "not an X-Plane OBJ8 file (first lines are {:?})",
                head_lines(text)
            );
        };

        let mut out = Obj8 {
            version,
            textures: Vec::new(),
            global_specular: None,
            point_counts: None,
            vertices: Vec::new(),
            line_vertices: Vec::new(),
            indices: Vec::new(),
            tri_descriptors: Vec::new(),
            line_descriptors: Vec::new(),
            lod_blocks: Vec::new(),
            raw_directives: Vec::new(),
            metadata_lines: Vec::new(),
        };

        // State directives seen since the previous TRIS descriptor, attached to the next one.
        let mut guards: Vec<String> = Vec::new();
        // Which `TEXTURE` binding the next draw descriptor inherits. Only the plain `TEXTURE`
        // directive is the active diffuse map; `TEXTURE_LIT`/`_NORMAL`/`_DRAPED`/`_LOD` are
        // additional slots on the same geometry, not a switch of the diffuse map.
        let mut active_diffuse: Option<usize> = None;
        // Which material tint the next draw descriptor inherits, set by `ATTR_diffuse_rgb` /
        // `ATTR_diffuse` and cleared by `ATTR_reset`.
        //
        // The directive is a preamble to the descriptor that *follows* it, not the one before:
        // `Aircraft/General Aviation/Baron B58/objects/B58_fuselage.obj` writes `TRIS 0 4608` at
        // line 36811, `ATTR_diffuse_rgb 0.950 0.950 0.950` at 36812, and `TRIS 4608 47613` at
        // 36814 — so 0.950 belongs to the second descriptor. The value then persists until the
        // next tint or an `ATTR_reset`.
        let mut active_tint: Option<[f32; 3]> = None;

        for (n, raw) in text.lines().enumerate() {
            let line_no = n + 1;
            let line = raw.trim_end();
            if line.is_empty() || line.starts_with('*') {
                continue;
            }
            // OBJ8's own comment character is '*', but WorldEditor writes '#' metadata lines
            // (`####_group`, `####_alpha`, `# Image: …`, `#ATTR_LOD`). They are kept rather
            // than dropped so the content checker can report them, and they are not treated
            // as directives or as guards on the next TRIS descriptor.
            if line.starts_with('#') {
                out.metadata_lines.push(RawDirective {
                    line: line_no,
                    text: line.to_string(),
                });
                continue;
            }
            let mut it = line.split_whitespace();
            let tok = it.next().unwrap_or("");
            let rest: Vec<&str> = it.collect();

            match tok {
                "A" | "OBJ" => {}
                v if !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()) => {}
                "TEXTURE" => active_diffuse = Some(out.push_texture(TextureKind::Diffuse, &rest)),
                // The other texture directives name additional slots on the same geometry; they
                // do not change which diffuse map the next draw descriptor uses.
                "TEXTURE_LIT" => {
                    let _ = out.push_texture(TextureKind::Lit, &rest);
                }
                "TEXTURE_DRAPED" => {
                    let _ = out.push_texture(TextureKind::Draped, &rest);
                }
                "TEXTURE_NORMAL" => {
                    let _ = out.push_texture(TextureKind::Normal, &rest);
                }
                "TEXTURE_LOD" => {
                    let _ = out.push_texture(TextureKind::Lod, &rest);
                }
                // The material diffuse tint, and its reset. Both spellings take three floats —
                // all 72 `ATTR_diffuse_rgb` lines and all 11 `ATTR_diffuse` lines in the shipped
                // corpus carry exactly three fields — and both sit in the same structural place,
                // immediately above a `TRIS` and usually beside `ATTR_shiny_rat`, in files that
                // use one spelling and never the other: `ATTR_diffuse` occurs only in
                // `Aircraft/Heavy Metal/B747-100 NASA/objects/747Cockpit_{Buttons,Main,Panels}.obj`,
                // `Aircraft/Heavy Metal/B747-400 United/objects/{747Cockpit_*,747_HStab}.obj` and
                // `Aircraft/X-Planes/X-15/X-15_Cockpit.obj`, none of which use `ATTR_diffuse_rgb`.
                // The port therefore treats them as one state variable.
                //
                // The value is not clamped: `ATTR_diffuse_rgb 2.5 2.5 2.5` is shipped content, and
                // `ATTR_diffuse 0.5882 0.5882 0.5882` (= 150/255) shows the other spelling carrying
                // 8-bit-quantized values. The line is still recorded as a raw directive and still
                // lands in `guards`, so the directive histogram stays unchanged.
                "ATTR_diffuse_rgb" | "ATTR_diffuse" => {
                    out.raw_directives.push(RawDirective {
                        line: line_no,
                        text: line.to_string(),
                    });
                    guards.push(tok.to_string());
                    if rest.len() != 3 {
                        bail!("line {line_no}: {tok} wants <r g b>, got {}", rest.len());
                    }
                    active_tint = Some([
                        parse_f32(Some(rest[0]), tok, line_no)?,
                        parse_f32(Some(rest[1]), tok, line_no)?,
                        parse_f32(Some(rest[2]), tok, line_no)?,
                    ]);
                }
                // `ATTR_reset` restores the app's default material state, tint included:
                // `Resources/default scenery/airport scenery/1000_Landmarks/Eiffel_Tower.obj`
                // sets 0.800 at line 22047, draws four descriptors with it, resets at 22060, and
                // draws `TRIS 25788 3672` at 22062 untinted. The other state `ATTR_reset` restores
                // (culling, blending, `ATTR_shiny_rat`) is not modeled yet, so only the tint is
                // cleared here.
                "ATTR_reset" => {
                    out.raw_directives.push(RawDirective {
                        line: line_no,
                        text: line.to_string(),
                    });
                    guards.push(tok.to_string());
                    active_tint = None;
                }
                "GLOBAL_specular" => {
                    out.global_specular = Some(parse_f32(rest.first().copied(), tok, line_no)?);
                }
                "POINT_COUNTS" => {
                    if rest.len() != 4 {
                        bail!(
                            "line {line_no}: POINT_COUNTS wants 4 fields, got {}",
                            rest.len()
                        );
                    }
                    out.point_counts = Some(PointCounts {
                        solid_vertices: parse_u32(rest[0], tok, line_no)?,
                        line_vertices: parse_u32(rest[1], tok, line_no)?,
                        particles: parse_u32(rest[2], tok, line_no)?,
                        indices: parse_u32(rest[3], tok, line_no)?,
                    });
                }
                "VT" => {
                    if rest.len() != 8 {
                        bail!("line {line_no}: VT wants 8 fields, got {}", rest.len());
                    }
                    let mut f = [0f32; 8];
                    for (i, s) in rest.iter().enumerate() {
                        f[i] = parse_f32(Some(s), tok, line_no)?;
                    }
                    out.vertices.push(Vertex {
                        pos: [f[0], f[1], f[2]],
                        normal: [f[3], f[4], f[5]],
                        uv: [f[6], f[7]],
                    });
                }
                "VLINE" => {
                    if rest.len() != 6 {
                        bail!(
                            "line {line_no}: VLINE wants <x y z r g b>, got {}",
                            rest.len()
                        );
                    }
                    let mut f = [0f32; 6];
                    for (i, s) in rest.iter().enumerate() {
                        f[i] = parse_f32(Some(s), tok, line_no)?;
                    }
                    out.line_vertices.push(LineVertex {
                        pos: [f[0], f[1], f[2]],
                        color: [f[3], f[4], f[5]],
                    });
                }
                "TRIS" => {
                    if rest.len() != 2 {
                        bail!(
                            "line {line_no}: TRIS wants <first_index> <index_count>, got {line:?}"
                        );
                    }
                    out.tri_descriptors.push(TriDescriptor {
                        first_index: parse_u32(rest[0], tok, line_no)?,
                        index_count: parse_u32(rest[1], tok, line_no)?,
                        texture: active_diffuse,
                        diffuse_rgb: active_tint,
                        source_line: line_no,
                        guards: std::mem::take(&mut guards),
                    });
                }
                // `LINES first_index index_count` consumes positions in the same index stream
                // TRIS does: radar.obj declares `1356 2 0 2330`, has `LINES 798 2` between
                // `TRIS 0 798` and `TRIS 800 1530`, and holds 2 330 indices — 798 + 2 + 1530.
                // Measured across the corpus: POINT_COUNTS field 2 equals the VLINE count in
                // 3 165 of 3 165 files that carry a POINT_COUNTS, so it counts line *vertices*,
                // not the indices LINES consumes (balloon3.obj declares 48 line vertices while
                // its LINES descriptors cover 60 indices). Every index in the stream, including
                // those inside a LINES range, is below POINT_COUNTS field 1 — 3 080 of 3 080
                // files — so the stream addresses the VT array only and VLINE vertices are a
                // separate array.
                "LINES" => {
                    if rest.len() != 2 {
                        bail!(
                            "line {line_no}: LINES wants <first_index> <index_count>, got {line:?}"
                        );
                    }
                    out.line_descriptors.push(LineDescriptor {
                        first_index: parse_u32(rest[0], tok, line_no)?,
                        index_count: parse_u32(rest[1], tok, line_no)?,
                        texture: active_diffuse,
                        diffuse_rgb: active_tint,
                        source_line: line_no,
                        guards: std::mem::take(&mut guards),
                    });
                }
                "IDX10" | "IDX" => {
                    let want = usize::from(tok == "IDX10") * 9 + 1;
                    if rest.len() != want {
                        bail!(
                            "line {line_no}: {tok} wants {want} indices, got {}",
                            rest.len()
                        );
                    }
                    for s in &rest {
                        let i = parse_u32(s, tok, line_no)?;
                        out.indices.push(i);
                    }
                }
                "L32" => out.push_lod(LodKind::Lod32, &rest, line_no)?,
                "L5" => out.push_lod(LodKind::Lod5, &rest, line_no)?,
                "LIGHT_LOD" => out.push_lod(LodKind::LightLod, &rest, line_no)?,
                _ => {
                    out.raw_directives.push(RawDirective {
                        line: line_no,
                        text: line.to_string(),
                    });
                    guards.push(tok.to_string());
                }
            }
        }

        Ok(out)
    }

    /// Record a texture binding and return its slot index.
    ///
    /// The slot is what [`TriDescriptor::texture`] stores: OBJ8 switches the active diffuse map
    /// with each `TEXTURE` line, and a draw descriptor inherits whichever one was active when it
    /// was written.
    ///
    /// An empty path is legal shipped content: placeholder objects such as
    /// `Resources/default scenery/airport scenery/Common_Elements/Lighting/Down_Light_Only.obj`
    /// write `TEXTURE` with no argument alongside `POINT_COUNTS 0 0 0 0` and an `EXPORT`
    /// redirect, so rejecting it would fail 82 real files.
    fn push_texture(&mut self, kind: TextureKind, rest: &[&str]) -> usize {
        let slot = self.textures.len();
        self.textures.push(TextureBinding {
            kind,
            path: rest.first().unwrap_or(&"").to_string(),
        });
        slot
    }

    fn push_lod(&mut self, kind: LodKind, rest: &[&str], line_no: usize) -> Result<()> {
        if rest.len() != 2 {
            bail!(
                "line {line_no}: LOD block wants <near> <far>, got {}",
                rest.len()
            );
        }
        self.lod_blocks.push(LodBlock {
            kind,
            near: parse_f32(rest.first().copied(), "LOD", line_no)?,
            far: parse_f32(rest.get(1).copied(), "LOD", line_no)?,
        });
        Ok(())
    }

    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    /// Indices declared by `POINT_COUNTS`, when the directive is present.
    pub fn index_count_declared(&self) -> Option<u32> {
        self.point_counts.map(|p| p.indices)
    }

    pub fn index_count_actual(&self) -> usize {
        self.indices.len()
    }

    /// Triangles actually described by the index stream.
    pub fn triangles_actual(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn lod_blocks(&self) -> &[LodBlock] {
        &self.lod_blocks
    }

    pub fn textures(&self) -> &[TextureBinding] {
        &self.textures
    }

    /// Total vertices in the index space: solid, then line, then particles.
    fn index_space(&self) -> usize {
        let particles = self.point_counts.map_or(0, |p| p.particles as usize);
        self.vertices.len() + self.line_vertices.len() + particles
    }

    /// Validate the object the way the original does for this role, reusing the original's
    /// wording where the original has wording (see `docs/ORACLE.md` §2).
    pub fn diagnostics(&self, name: &str, role: ObjectRole) -> Vec<Diagnostic> {
        let mut out = Vec::new();

        if let Some(pc) = self.point_counts {
            if pc.solid_vertices as usize != self.vertices.len() {
                out.push(Diagnostic {
                    severity: Severity::Error,
                    message: format!(
                        "The object {name} declares {} solid vertices in POINT_COUNTS but defines {}",
                        pc.solid_vertices,
                        self.vertices.len()
                    ),
                });
            }
            if pc.line_vertices as usize != self.line_vertices.len() {
                out.push(Diagnostic {
                    severity: Severity::Error,
                    message: format!(
                        "The object {name} declares {} line vertices in POINT_COUNTS but defines {}",
                        pc.line_vertices,
                        self.line_vertices.len()
                    ),
                });
            }
            if pc.indices as usize != self.indices.len() {
                out.push(Diagnostic {
                    severity: Severity::Error,
                    message: format!(
                        "The object {name} declares {} indices in POINT_COUNTS but defines {}",
                        pc.indices,
                        self.indices.len()
                    ),
                });
            }
        }

        // `TRIS` and `LINES` both name ranges of the one index stream, and they may appear in
        // any order. Measured on the shipped install: of 3 080 v800 objects carrying both
        // POINT_COUNTS and draw descriptors, 3 067 have ranges that tile [0, POINT_COUNTS[4])
        // exactly. The other 13 overlap or leave a gap — `1000 autogen/global_objects/objects/
        // RooftopHelipad2A.obj` declares 32 823 indices, holds 32 823, and its descriptors
        // cover 45 837, so some index ranges are drawn by more than one material. The original
        // draws each descriptor as given, so this is legal content rather than a broken file:
        // it is a warning. A stream whose length disagrees with POINT_COUNTS is a real break
        // and is reported as an error above.
        let mut ranges: Vec<(u32, u32)> = self
            .tri_descriptors
            .iter()
            .map(|d| (d.first_index, d.index_count))
            .chain(
                self.line_descriptors
                    .iter()
                    .map(|d| (d.first_index, d.index_count)),
            )
            .collect();
        ranges.sort_unstable();
        let total = self.indices.len() as u64;
        let mut cur: u64 = 0;
        let mut overlap = false;
        let mut gap = false;
        for &(start, count) in &ranges {
            let start = u64::from(start);
            if start < cur {
                overlap = true;
            } else if start > cur {
                gap = true;
                cur = start;
            }
            cur += u64::from(count);
        }
        gap |= cur != total;
        if overlap || gap {
            let why = if overlap && gap {
                "overlap and leave gaps"
            } else if overlap {
                "overlap"
            } else {
                "leave gaps"
            };
            out.push(Diagnostic {
                severity: Severity::Warning,
                message: format!(
                    "The object {name} draw descriptors {why} in its {total}-index stream"
                ),
            });
        }

        let space = self.index_space();
        if let Some(&bad) = self.indices.iter().find(|&&i| i as usize >= space) {
            out.push(Diagnostic {
                severity: Severity::Error,
                message: format!(
                    "The object {name} references index {bad} but the index space holds {space} entries"
                ),
            });
        }

        // The original's own rule, observed in Log.txt for the scenery object
        // ConCSlantRoof.obj:
        //   W/OBJ: The object … has a first LOD near distance of 2000.000000 but it should start at 0!
        // It is not reported for aircraft objects, so it is scoped to the scenery role.
        let first_near = self
            .lod_blocks
            .first()
            .map(|l| l.near)
            .unwrap_or(IMPLICIT_FIRST_LOD_NEAR);
        if role == ObjectRole::Scenery && first_near != 0.0 {
            out.push(Diagnostic {
                severity: Severity::Warning,
                message: format!(
                    "The object {name} has a first LOD near distance of {first_near:.6} but it should start at 0!",
                ),
            });
        }

        out
    }
}

fn head_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.is_empty())
        .take(3)
        .collect()
}

fn parse_f32(s: Option<&str>, what: &str, line: usize) -> Result<f32> {
    let s = s.with_context(|| format!("line {line}: {what} is missing a value"))?;
    s.parse::<f32>()
        .with_context(|| format!("line {line}: {what} has a non-numeric value {s:?}"))
}

fn parse_u32(s: &str, what: &str, line: usize) -> Result<u32> {
    s.parse::<u32>()
        .with_context(|| format!("line {line}: {what} has a bad index/count {s:?}"))
}
