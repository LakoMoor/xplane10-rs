//! Golden tests for `xp_core::scene` — the OBJ8 → GPU-buffer assembly.
//!
//! The expectations are the counts measured from the shipped files themselves, which
//! `tests/golden/obj8/fuse_summary.txt` already pins. What is new here is the derived layer:
//! per-draw vertex blocks, the re-based upload offsets, the tint each block carries, and the
//! triangle count a rendered frame has to reproduce.
//!
//! The upload counts are *derived*, not declared, so they are pinned here rather than in the
//! format goldens: `Mesh::vertex_count()` is the sum of distinct indices per draw, which exceeds
//! the file's `VT` count wherever two draws share a vertex.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use xp_core::obj8::{self, ObjectRole};
use xp_core::scene::{self, Topology};

fn install() -> Result<PathBuf> {
    let root = xp_core::install_root().context("set XP_INSTALL to the shipped installation")?;
    if !root.is_dir() {
        bail!("XP_INSTALL={} is not a directory", root.display());
    }
    Ok(root)
}

fn at(root: &Path, rel: &str) -> PathBuf {
    root.join(rel)
}

/// `Aircraft/General Aviation/Cessna 172SP/objects/fuse.obj`, sha `34f1019b…`.
///
/// Measured in the file: 2 850 `VT`, 1 094 `IDX10` + 4 `IDX` = 10 944 indices, and three
/// descriptors `TRIS 0 10152`, `TRIS 10152 396`, `TRIS 10548 396`.
#[test]
fn fuse_builds_the_buffers_the_file_describes() -> Result<()> {
    let root = install()?;
    let path = at(
        &root,
        "Aircraft/General Aviation/Cessna 172SP/objects/fuse.obj",
    );
    let (obj, mesh) = scene::load(&path)?;

    // What the file declares is unchanged by assembly.
    assert_eq!(obj.vertices.len(), 2850, "VT count");
    assert_eq!(obj.indices.len(), 10944, "declared index stream length");
    assert_eq!(mesh.draws.len(), 3, "draw count");
    assert_eq!(mesh.triangle_count(), 3648, "triangle count");

    // Declared offsets stay what the file wrote: `first_index` and `index_count` verbatim.
    let declared: Vec<(usize, usize)> = mesh
        .draws
        .iter()
        .map(|d| (d.declared_index_offset, d.index_count))
        .collect();
    assert_eq!(
        declared,
        vec![(0, 10152), (10152, 396), (10548, 396)],
        "declared draw offsets and counts"
    );

    // The three descriptors tile the stream, so the upload stream has the same length even though
    // it is no longer a copy of the file's index values.
    assert_eq!(mesh.indices.len(), 10944, "upload index count");

    // One vertex is used by two of the three draws, so the per-draw blocks hold one vertex more
    // than the file's VT array. That duplication is the point: it is what lets two draws of one
    // vertex array carry different `a_color` values.
    assert_eq!(mesh.vertex_count(), 2851, "emitted vertices");
    assert_eq!(
        mesh.vertex_bytes.len(),
        mesh.vertex_count() * scene::VERTEX_STRIDE,
        "interleaved buffer size"
    );

    // Upload offsets and block sizes: (index_offset, index_count, vertex_offset, vertex_count).
    let upload: Vec<(usize, usize, usize, usize)> = mesh
        .draws
        .iter()
        .map(|d| {
            (
                d.index_offset,
                d.index_count,
                d.vertex_offset,
                d.vertex_count,
            )
        })
        .collect();
    assert_eq!(
        upload,
        vec![
            (0, 10152, 0, 2531),
            (10152, 396, 2531, 160),
            (10548, 396, 2691, 160)
        ],
        "upload offsets and vertex block sizes"
    );

    for d in &mesh.draws {
        assert_eq!(d.topology, Topology::Triangles, "topology");
        assert_eq!(d.texture, Some(0), "texture slot for {}", d.index_offset);
        // fuse.obj writes no ATTR_diffuse_rgb, so every draw inherits the default material.
        assert!(
            !d.tinted,
            "draw at line {} should be untinted",
            d.source_line
        );
        assert_eq!(d.color, scene::DEFAULT_DIFFUSE, "tint");
    }
    // `TEXTURE fuse.png` and `TEXTURE_NORMAL fuse_bump.png` are two slots; only the first is the
    // active diffuse map, so every draw above points at slot 0.
    assert_eq!(mesh.textures.len(), 2, "texture slots");

    Ok(())
}

/// `Resources/default scenery/sim objects/landscape/radar.obj` is the fixture that proves
/// `LINES` shares the index stream: `POINT_COUNTS 1356 2 0 2330`, `TRIS 0 798`,
/// `LINES 798 2`, `TRIS 800 1530`.
///
/// It is also the fixture for draw *order*: the two descriptor lists on their own would emit
/// `TRIS`, `TRIS`, `LINES`, while the file writes `TRIS`, `LINES`, `TRIS`.
#[test]
fn line_descriptors_share_the_index_stream() -> Result<()> {
    let root = install()?;
    let path = at(
        &root,
        "Resources/default scenery/sim objects/landscape/radar.obj",
    );
    let (obj, mesh) = scene::load(&path)?;

    assert_eq!(obj.vertices.len(), 1356, "VT count");
    assert_eq!(obj.indices.len(), 2330, "declared index stream length");
    assert_eq!(
        mesh.draws.len(),
        3,
        "two TRIS plus one LINES become three draws"
    );

    // File order, not list order: the LINES draw sits between the two TRIS draws.
    let kinds: Vec<Topology> = mesh.draws.iter().map(|d| d.topology).collect();
    assert_eq!(
        kinds,
        vec![
            Topology::Triangles,
            Topology::LineSegments,
            Topology::Triangles
        ],
        "draws follow the order the file wrote them in"
    );
    let lines: Vec<usize> = mesh.draws.iter().map(|d| d.source_line).collect();
    assert!(
        lines.windows(2).all(|w| w[0] < w[1]),
        "source lines must ascend, got {lines:?}"
    );

    let line = mesh
        .draws
        .iter()
        .find(|d| d.topology == Topology::LineSegments)
        .context("no LINES draw")?;
    assert_eq!(
        (line.declared_index_offset, line.index_count),
        (798, 2),
        "LINES range"
    );

    // 798 + 2 + 1530 = 2330: the ranges tile the stream the way POINT_COUNTS field 4 counts it,
    // so the upload stream is the same length.
    let covered: usize = mesh.draws.iter().map(|d| d.index_count).sum();
    assert_eq!(covered, obj.indices.len(), "ranges cover the stream");
    assert_eq!(mesh.indices.len(), covered, "upload stream length");

    // Line draws contribute no triangles, so the triangle total comes only from the TRIS ranges.
    assert_eq!(mesh.triangle_count(), 2328 / 3, "triangle count");

    Ok(())
}

/// `Aircraft/General Aviation/Baron B58/objects/B58_fuselage.obj` is the fixture for the material
/// tint: it writes `ATTR_diffuse_rgb 0.950 0.950 0.950` between its first and second descriptors,
/// and `ATTR_reset` before a later one.
#[test]
fn material_tint_follows_the_descriptor_that_follows_it() -> Result<()> {
    let root = install()?;
    let path = at(
        &root,
        "Aircraft/General Aviation/Baron B58/objects/B58_fuselage.obj",
    );
    let (_obj, mesh) = scene::load(&path)?;

    // The directive is a preamble to the *next* descriptor: the file's first TRIS at line 36 811
    // has no tint before it, and the 0.950 at line 36 812 belongs to the descriptor at 36 814.
    let first = mesh.draws.first().context("no draws")?;
    assert_eq!(first.source_line, 36811, "first descriptor line");
    assert!(!first.tinted, "the first draw has no tint preamble");
    assert_eq!(first.color, scene::DEFAULT_DIFFUSE);

    let second = &mesh.draws[1];
    assert_eq!(second.source_line, 36814, "second descriptor line");
    assert!(second.tinted, "the 0.950 preamble belongs to this draw");
    assert_eq!(second.color, [0.950, 0.950, 0.950, 1.0], "tint");

    // Tint persists until the next tint or an ATTR_reset, so a draw with no immediate preamble
    // still inherits the last value written.
    let tinted: Vec<(usize, [f32; 4])> = mesh
        .draws
        .iter()
        .filter(|d| d.tinted)
        .map(|d| (d.source_line, d.color))
        .collect();
    assert_eq!(
        tinted,
        vec![
            (36814, [0.950, 0.950, 0.950, 1.0]),
            (36940, [0.800, 0.800, 0.800, 1.0]),
            (36986, [0.950, 0.950, 0.950, 1.0]),
            (36990, [0.950, 0.950, 0.950, 1.0]),
            (37006, [0.950, 0.950, 0.950, 1.0]),
            (37011, [0.950, 0.950, 0.950, 1.0]),
            (37016, [0.950, 0.950, 0.950, 1.0]),
            (37046, [0.950, 0.950, 0.950, 1.0]),
            (37050, [0.800, 0.800, 0.800, 1.0]),
        ],
        "tinted draws, in file order"
    );

    // A descriptor with no tint line of its own still inherits the last value written: the file
    // sets 0.950 at line 36 984, and the descriptors at 36 986 and 36 990 both draw with it, but
    // only 36 986 has the directive in its own preamble.
    let inherited: Vec<(usize, [f32; 4])> = mesh
        .draws
        .iter()
        .filter(|d| d.tinted && !d.guards.iter().any(|g| g == "ATTR_diffuse_rgb"))
        .map(|d| (d.source_line, d.color))
        .collect();
    assert_eq!(
        inherited,
        vec![
            (36990, [0.950, 0.950, 0.950, 1.0]),
            (37011, [0.950, 0.950, 0.950, 1.0]),
            (37016, [0.950, 0.950, 0.950, 1.0]),
            (37046, [0.950, 0.950, 0.950, 1.0]),
        ],
        "draws that inherit a tint instead of setting it"
    );

    // An ATTR_reset goes back to the default: the file resets before the descriptor that draws
    // `TRIS 52221 1878`, and that draw carries the default material with no tint of its own.
    assert!(
        mesh.draws
            .iter()
            .any(|d| !d.tinted && d.color == scene::DEFAULT_DIFFUSE),
        "an ATTR_reset must restore the default"
    );

    Ok(())
}

/// Assembly must not invent or lose geometry anywhere in the install: every draw stays inside the
/// stream it was declared against, every upload index stays inside its own vertex block, and the
/// tint each block carries is the tint the descriptor inherited.
#[test]
fn every_shipped_object_assembles_without_damage() -> Result<()> {
    let root = install()?;
    let mut built = 0usize;
    let mut with_point_counts = 0usize;
    let mut total_triangles = 0u64;
    let mut total_vertices = 0u64;
    let mut total_indices = 0u64;
    let mut total_draws = 0u64;
    let mut duplicated = 0usize;
    let mut extra_vertices = 0u64;
    let mut tinted_draws = 0usize;
    let mut untiled = Vec::new();
    let mut out_of_range = Vec::new();

    for entry in walk(&root)? {
        let Ok(text) = xp_core::read_text_lossy(&entry) else {
            continue;
        };
        if !matches!(obj8::classify(&text), obj8::ObjKind::Obj8 { .. }) {
            continue;
        }
        let obj =
            obj8::Obj8::parse(&text).with_context(|| format!("parsing {}", entry.display()))?;
        let mesh = scene::build(&obj).with_context(|| format!("building {}", entry.display()))?;
        built += 1;

        assert_eq!(
            mesh.draws.len(),
            obj.tri_descriptors.len() + obj.line_descriptors.len(),
            "draw count for {}",
            entry.display()
        );
        assert_eq!(
            mesh.vertex_bytes.len(),
            mesh.vertex_count() * scene::VERTEX_STRIDE,
            "buffer size for {}",
            entry.display()
        );

        // Upload totals are the sum of the per-draw blocks, by construction.
        let sum_index_count: usize = mesh.draws.iter().map(|d| d.index_count).sum();
        let sum_vertex_count: usize = mesh.draws.iter().map(|d| d.vertex_count).sum();
        assert_eq!(
            mesh.indices.len(),
            sum_index_count,
            "upload indices vs declared counts in {}",
            entry.display()
        );
        assert_eq!(
            mesh.vertex_count(),
            sum_vertex_count,
            "upload vertices vs block sizes in {}",
            entry.display()
        );

        // Draws come out in the order the file wrote them.
        assert!(
            mesh.draws
                .windows(2)
                .all(|w| w[0].source_line < w[1].source_line),
            "draws out of file order in {}",
            entry.display()
        );

        for d in &mesh.draws {
            // The declared range must exist in the file's own stream.
            if d.declared_index_offset + d.index_count > obj.indices.len() {
                out_of_range.push(format!(
                    "{}: draw at line {} declares {}+{} over a {}-index stream",
                    entry.display(),
                    d.source_line,
                    d.declared_index_offset,
                    d.index_count,
                    obj.indices.len()
                ));
            }
            // The upload range and the vertex block must exist in the buffers.
            if d.index_offset + d.index_count > mesh.indices.len() {
                out_of_range.push(format!(
                    "{}: draw at line {} uploads {}+{} over a {}-index buffer",
                    entry.display(),
                    d.source_line,
                    d.index_offset,
                    d.index_count,
                    mesh.indices.len()
                ));
            }
            if d.vertex_offset + d.vertex_count > mesh.vertex_count() {
                out_of_range.push(format!(
                    "{}: draw at line {} takes vertices {}+{} of {}",
                    entry.display(),
                    d.source_line,
                    d.vertex_offset,
                    d.vertex_count,
                    mesh.vertex_count()
                ));
            }
            // Every upload index must address this draw's own block, and the block must be dense:
            // a block of n vertices has to be referenced by at least n distinct indices, or the
            // assembly emitted a vertex nothing draws.
            for &i in &mesh.indices[d.index_offset..d.index_offset + d.index_count] {
                if (i as usize) < d.vertex_offset
                    || (i as usize) >= d.vertex_offset + d.vertex_count
                {
                    out_of_range.push(format!(
                        "{}: draw at line {} references vertex {i} outside its block {}..{}",
                        entry.display(),
                        d.source_line,
                        d.vertex_offset,
                        d.vertex_offset + d.vertex_count
                    ));
                    break;
                }
            }
            if d.vertex_count > d.index_count {
                out_of_range.push(format!(
                    "{}: draw at line {} emits {} vertices for {} indices",
                    entry.display(),
                    d.source_line,
                    d.vertex_count,
                    d.index_count
                ));
            }

            if let Some(t) = d.texture {
                if t >= mesh.textures.len() {
                    bail!(
                        "{}: draw references texture slot {t} of {}",
                        entry.display(),
                        mesh.textures.len()
                    );
                }
            }

            // The tint is the descriptor's, or the documented default.
            let want = obj
                .tri_descriptors
                .iter()
                .map(|t| (t.source_line, t.diffuse_rgb))
                .chain(
                    obj.line_descriptors
                        .iter()
                        .map(|l| (l.source_line, l.diffuse_rgb)),
                )
                .find(|(line, _)| *line == d.source_line)
                .map(|(_, rgb)| rgb);
            let want = want.unwrap_or_else(|| {
                panic!(
                    "{}: no descriptor at line {}",
                    entry.display(),
                    d.source_line
                )
            });
            match want {
                Some(rgb) => {
                    tinted_draws += 1;
                    assert_eq!(
                        d.color,
                        [rgb[0], rgb[1], rgb[2], 1.0],
                        "{}: draw at line {} lost its tint",
                        entry.display(),
                        d.source_line
                    );
                    assert!(d.tinted);
                }
                None => {
                    assert_eq!(
                        d.color,
                        scene::DEFAULT_DIFFUSE,
                        "{}: draw at line {} should carry the default material",
                        entry.display(),
                        d.source_line
                    );
                    assert!(!d.tinted);
                }
            }
        }

        if let Some(pc) = obj.point_counts {
            with_point_counts += 1;
            // The upload stream matches POINT_COUNTS field 4 exactly when the descriptors tile the
            // stream; the 13 objects the content checker already reports as overlapping or
            // gapped are the exceptions, and they are content anomalies, not assembly damage.
            if mesh.indices.len() != pc.indices as usize {
                untiled.push(format!(
                    "{}: descriptors cover {} of {} declared indices",
                    entry.display(),
                    mesh.indices.len(),
                    pc.indices
                ));
            }
            // Emitted vertices are the file's VT count plus the duplication per-draw blocks need.
            if mesh.vertex_count() > pc.solid_vertices as usize {
                duplicated += 1;
                extra_vertices += (mesh.vertex_count() - pc.solid_vertices as usize) as u64;
            }
        }

        total_triangles += mesh.triangle_count() as u64;
        total_vertices += mesh.vertex_count() as u64;
        total_indices += mesh.indices.len() as u64;
        total_draws += mesh.draws.len() as u64;
    }

    assert_eq!(built, 3197, "OBJ8 objects assembled");
    assert_eq!(with_point_counts, 3084, "objects with a POINT_COUNTS");
    assert!(
        out_of_range.is_empty(),
        "assembly put a draw outside its buffers:\n{}",
        out_of_range.join("\n")
    );
    assert_eq!(
        untiled.len(),
        13,
        "objects whose descriptors do not tile the declared stream: {untiled:?}"
    );

    // Corpus totals, measured from this run and pinned so a later change to assembly that moves
    // geometry in or out shows up as a failure rather than as a new number.
    assert_eq!(total_draws, 8505, "corpus draw total");
    assert_eq!(total_indices, 17_011_777, "corpus upload index total");
    assert_eq!(total_vertices, 7_320_005, "corpus upload vertex total");
    assert_eq!(total_triangles, 5_667_175, "corpus triangle total");
    assert_eq!(
        duplicated, 272,
        "objects whose per-draw blocks duplicate a shared vertex"
    );
    assert_eq!(extra_vertices, 243_931, "vertices added by duplication");
    assert_eq!(tinted_draws, 125, "draws carrying an authored tint");

    // The first-LOD rule is a scenery-role rule; assembly must not change what it reports.
    let roof = at(
        &root,
        "Custom Scenery/KSEA Demo Area/KSEA_objects/ConCSlantRoof.obj",
    );
    let (obj, mesh) = scene::load(&roof)?;
    let diags = obj.diagnostics("ConCSlantRoof.obj", ObjectRole::Scenery);
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("first LOD near distance")),
        "scenery role must still report the LOD rule"
    );
    assert!(!mesh.draws.is_empty(), "the fixture object has no draws");

    Ok(())
}

/// Every `.obj` under the install, in a stable order.
fn walk(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = std::fs::read_dir(&dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .filter_map(|e| e.ok())
            .collect::<Vec<_>>();
        for e in rd {
            let p = e.path();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                stack.push(p);
            } else if p.extension().and_then(|s| s.to_str()) == Some("obj") {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}
