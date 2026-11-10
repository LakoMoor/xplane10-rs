//! Golden tests for the OBJ8 parser. Expectations come from
//! `tests/golden/obj8/fuse_summary.txt`, extracted from shipped content.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use xp_core::obj8::{self, Obj8, TextureKind};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden")
}

fn read_golden(path: &Path) -> Result<BTreeMap<String, String>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading golden {}", path.display()))?;
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = match line.find(": ") {
            Some(i) => (&line[..i], line[i + 2..].trim()),
            None => match line.find('=') {
                Some(i) => (&line[..i], line[i + 1..].trim()),
                None => bail!("{}: bad golden line {line:?}", path.display()),
            },
        };
        out.insert(k.trim().to_string(), v.trim().to_string());
    }
    Ok(out)
}

fn want(g: &BTreeMap<String, String>, key: &str) -> Result<String> {
    g.get(key)
        .cloned()
        .with_context(|| format!("golden has no key {key:?}"))
}

fn usize_of(g: &BTreeMap<String, String>, key: &str) -> Result<usize> {
    want(g, key)?
        .parse::<usize>()
        .with_context(|| format!("golden {key:?} is not a number"))
}

/// A complete small shipped object: CRLF line endings and tab-separated fields, so this
/// also proves the reader is not space-and-LF only.
#[test]
fn complete_shipped_object_parses() -> Result<()> {
    let path = golden_dir().join("fixtures/conc_slant_roof.obj");
    let o = Obj8::load(&path)?;
    assert_eq!(o.version, 800);
    assert_eq!(o.textures.len(), 2);
    assert_eq!(o.textures[0].kind, TextureKind::Diffuse);
    assert_eq!(o.textures[0].path, "textures/kseatk.png");
    assert_eq!(o.textures[1].kind, TextureKind::Lit);
    assert_eq!(o.global_specular, Some(1.0));

    let pc = o.point_counts.context("POINT_COUNTS missing")?;
    assert_eq!(
        (
            pc.solid_vertices,
            pc.line_vertices,
            pc.particles,
            pc.indices
        ),
        (66, 0, 0, 120)
    );
    assert_eq!(o.vertex_count(), 66);
    assert_eq!(o.line_vertices.len(), 0);
    assert_eq!(o.index_count_actual(), 120);
    assert_eq!(o.triangles_actual(), 40);
    assert_eq!(o.indices.iter().min().copied(), Some(0));
    assert_eq!(o.indices.iter().max().copied(), Some(65));

    // TRIS names a range of the index stream that was written before it.
    assert_eq!(o.tri_descriptors.len(), 2);
    assert_eq!(
        o.tri_descriptors
            .iter()
            .map(|d| (d.first_index, d.index_count))
            .collect::<Vec<_>>(),
        vec![(0, 48), (48, 72)]
    );
    let declared: u32 = o.tri_descriptors.iter().map(|d| d.index_count).sum();
    assert_eq!(declared as usize, o.index_count_actual());

    // No LOD directive at all, so the original's implicit 2000 m near distance applies and
    // the object draws the warning the original itself drew.
    assert!(o.lod_blocks.is_empty());
    let diags = o.diagnostics("x", obj8::ObjectRole::Scenery);
    let msgs: Vec<String> = diags.iter().map(|d| d.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m.contains("first LOD near distance of 2000.000000 but it should start at 0!")),
        "expected the implicit-LOD warning, got {msgs:?}"
    );
    assert_eq!(
        diags
            .iter()
            .filter(|d| d.severity == obj8::Severity::Error)
            .count(),
        0,
        "counts reconcile, so only the LOD warning may fire"
    );
    Ok(())
}

#[test]
fn classify_separates_obj8_from_wavefront() {
    assert_eq!(
        obj8::classify("A\n800\nOBJ\n\nVT 0 0 0 0 0 1 0 0\n"),
        obj8::ObjKind::Obj8 { version: 800 }
    );
    assert_eq!(
        obj8::classify("v 0 0 0\nf 1 2 3\n"),
        obj8::ObjKind::Wavefront
    );
    assert_eq!(obj8::classify("usemtl paint\n"), obj8::ObjKind::Wavefront);
    assert_eq!(obj8::classify("whatever\n"), obj8::ObjKind::Unknown);
}

#[test]
fn full_fuse_matches_golden() -> Result<()> {
    let Some(root) = xp_core::install_root() else {
        eprintln!("SKIP full_fuse_matches_golden: XP_INSTALL unset (no coverage claimed)");
        return Ok(());
    };
    let path = root.join("Aircraft/General Aviation/Cessna 172SP/objects/fuse.obj");
    if !path.is_file() {
        eprintln!("SKIP full_fuse_matches_golden: {} missing", path.display());
        return Ok(());
    }
    let g = read_golden(&golden_dir().join("obj8/fuse_summary.txt"))?;
    let o = Obj8::load(&path)?;

    assert_eq!(o.version, want(&g, "version")?.parse::<u32>()?);
    assert_eq!(o.vertex_count(), usize_of(&g, "vertex_lines")?);
    assert_eq!(o.index_count_actual(), usize_of(&g, "total_indices")?);
    assert_eq!(o.triangles_actual(), usize_of(&g, "triangles_actual")?);
    assert_eq!(o.tri_descriptors.len(), usize_of(&g, "tris_block_count")?);
    assert_eq!(
        o.point_counts.context("POINT_COUNTS missing")?.indices as usize,
        usize_of(&g, "total_indices")?
    );
    assert_eq!(
        o.indices.iter().min().copied(),
        Some(want(&g, "index_min")?.parse::<u32>()?)
    );
    assert_eq!(
        o.indices.iter().max().copied(),
        Some(want(&g, "index_max")?.parse::<u32>()?)
    );

    for (n, d) in o.tri_descriptors.iter().enumerate() {
        assert_eq!(
            format!("{} {}", d.first_index, d.index_count),
            want(&g, &format!("tris_block[{n}]"))?,
            "TRIS descriptor {n} drifted"
        );
    }
    assert_eq!(o.point_counts.context("POINT_COUNTS missing")?, {
        let f: Vec<u32> = want(&g, "point_counts")?
            .split_whitespace()
            .map(|s| s.parse::<u32>())
            .collect::<Result<Vec<_>, _>>()?;
        obj8::PointCounts {
            solid_vertices: f[0],
            line_vertices: f[1],
            particles: f[2],
            indices: f[3],
        }
    });

    assert_eq!(o.textures.len(), 2);
    assert_eq!(o.textures[0].path, want(&g, "texture_0")?);
    assert_eq!(o.textures[1].kind, TextureKind::Normal);
    assert_eq!(o.textures[1].path, want(&g, "texture_normal_0")?);
    assert_eq!(o.global_specular, Some(1.0));
    assert_eq!(o.vertices[0].pos, [0.424484, 0.121857, -0.885086]);
    assert_eq!(o.vertices[2849].pos, [-0.572433, 0.073486, 0.803360]);

    // The index-count identity is the real check: declared == held == descriptors' sum.
    assert_eq!(
        o.index_count_declared(),
        Some(o.index_count_actual() as u32)
    );
    let declared: u32 = o.tri_descriptors.iter().map(|d| d.index_count).sum();
    assert_eq!(declared as usize, o.index_count_actual());
    // fuse.obj is an aircraft part: the original ran this exact file in the scripted boot and
    // printed nothing about it, so the scenery-only LOD rule must not fire here.
    let aircraft_diags = o.diagnostics("fuse.obj", obj8::ObjectRole::Aircraft);
    assert!(
        aircraft_diags.is_empty(),
        "fuse.obj must validate clean as an aircraft object, got {aircraft_diags:?}"
    );
    Ok(())
}

/// The original warns about this exact file with this exact number (Log.txt, 10.51r2).
#[test]
fn lod_warning_matches_the_originals_own_message() -> Result<()> {
    let Some(root) = xp_core::install_root() else {
        eprintln!("SKIP lod_warning: XP_INSTALL unset (no coverage claimed)");
        return Ok(());
    };
    let rel = "Custom Scenery/KSEA Demo Area/KSEA_objects/ConCSlantRoof.obj";
    let path = root.join(rel);
    if !path.is_file() {
        eprintln!("SKIP lod_warning: {} missing", path.display());
        return Ok(());
    }
    let o = Obj8::load(&path)?;
    assert!(o.lod_blocks.is_empty(), "this file declares no LOD block");
    let diags = o.diagnostics(rel, obj8::ObjectRole::Scenery);
    let msgs: Vec<String> = diags.iter().map(|d| d.message.clone()).collect();
    assert!(
        msgs.contains(&"The object Custom Scenery/KSEA Demo Area/KSEA_objects/ConCSlantRoof.obj has a first LOD near distance of 2000.000000 but it should start at 0!".to_string()),
        "expected the original's LOD warning, got {msgs:?}"
    );
    Ok(())
}

/// Framing must never fail across the shipped objects; unmodeled directives are reported.
#[test]
fn every_shipped_obj_frames() -> Result<()> {
    let Some(root) = xp_core::install_root() else {
        eprintln!("SKIP every_shipped_obj_frames: XP_INSTALL unset (no coverage claimed)");
        return Ok(());
    };
    let mut obj8_files = 0usize;
    let mut wavefront = 0usize;
    let mut unrecognized = 0usize;
    let mut failed: Vec<String> = Vec::new();
    let mut tokens: BTreeMap<String, usize> = BTreeMap::new();

    for dir in ["Aircraft", "Custom Scenery", "Resources"] {
        let base = root.join(dir);
        if !base.is_dir() {
            continue;
        }
        let mut stack = vec![base];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if p.extension().and_then(|s| s.to_str()) != Some("obj") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&p) else {
                    unrecognized += 1;
                    continue;
                };
                match obj8::classify(&text) {
                    obj8::ObjKind::Obj8 { .. } => {
                        obj8_files += 1;
                        match Obj8::parse(&text) {
                            Ok(o) => {
                                for r in &o.raw_directives {
                                    let t = r.text.split_whitespace().next().unwrap_or("");
                                    *tokens.entry(t.to_string()).or_default() += 1;
                                }
                            }
                            Err(err) => failed.push(format!("{}: {err}", p.display())),
                        }
                    }
                    obj8::ObjKind::Wavefront => wavefront += 1,
                    obj8::ObjKind::Unknown => unrecognized += 1,
                }
            }
        }
    }

    eprintln!("OBJ8 parsed: {obj8_files}; Wavefront .obj (a different format, not misparsed): {wavefront}; unrecognized: {unrecognized}");
    eprintln!(
        "directives kept but not yet modeled: {}",
        tokens
            .iter()
            .take(30)
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    for f in failed.iter().take(10) {
        eprintln!("  framing failure: {f}");
    }
    assert!(
        failed.is_empty(),
        "{} OBJ8 files failed to frame, e.g. {:?}",
        failed.len(),
        failed.iter().take(5).collect::<Vec<_>>()
    );
    assert!(
        obj8_files > 100,
        "expected the shipped OBJ8 corpus, found {obj8_files}"
    );
    Ok(())
}
