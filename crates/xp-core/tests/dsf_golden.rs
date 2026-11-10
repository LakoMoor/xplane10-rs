//! Golden tests for the DSF reader. Expectations come from
//! `tests/golden/dsf/corpus_summary.txt`, measured from the shipped install.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use xp_core::dsf::{self, Dsf, DsfKind};

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
            Some(i) => (&line[..i], &line[i + 2..]),
            None => match line.find(" = ") {
                Some(i) => (&line[..i], &line[i + 3..]),
                None => bail!("{}: bad golden line {line:?}", path.display()),
            },
        };
        out.insert(k.trim().to_string(), v.trim().to_string());
    }
    if out.is_empty() {
        bail!("golden {} is empty", path.display());
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

/// Guards the reader's own MD5: if this fails, every "trailer is the MD5" claim below is
/// meaningless, including the corpus-wide one.
#[test]
fn md5_matches_published_vectors() {
    // RFC 1321 appendix test vectors.
    let cases: [(&[u8], &str); 4] = [
        (b"", "d41d8cd98f00b204e9800998ecf8427e"),
        (b"a", "0cc175b9c0f1b6a831c399e269772661"),
        (b"abc", "900150983cd24fb0d6963f7d28e17f72"),
        (
            b"The quick brown fox jumps over the lazy dog",
            "9e107d9d372bb6826bd81d3542a419d6",
        ),
    ];
    for (input, expect) in cases {
        let got: String = dsf::md5(input).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(got, expect, "MD5 of {input:?}");
    }
}

#[test]
fn classify_distinguishes_a_compressed_patch() {
    assert_eq!(
        dsf::classify(b"XPLNEDSF\x01\0\0\0xxxx"),
        DsfKind::Plain { version: 1 }
    );
    assert_eq!(
        dsf::classify(b"7z\xbc\xaf'\x1c\x00\x03abcd"),
        DsfKind::Compressed7z
    );
    assert_eq!(dsf::classify(b"not a patch at all"), DsfKind::Unknown);
    assert_eq!(dsf::classify(b"short"), DsfKind::Unknown);
}

/// The reference patch, chunk for chunk, against the measured golden.
#[test]
fn reference_patch_matches_golden() -> Result<()> {
    let Some(root) = xp_core::install_root() else {
        eprintln!("SKIP reference_patch_matches_golden: XP_INSTALL unset (no coverage claimed)");
        return Ok(());
    };
    let rel = "Custom Scenery/KSEA Demo Area/Earth nav data/+40-130/+47-123.dsf";
    let path = root.join(rel);
    if !path.is_file() {
        eprintln!(
            "SKIP reference_patch_matches_golden: {} missing",
            path.display()
        );
        return Ok(());
    }
    let g = read_golden(&golden_dir().join("dsf/corpus_summary.txt"))?;
    let bytes = std::fs::read(&path)?;
    let d = Dsf::parse(&bytes)?;

    assert_eq!(d.kind, DsfKind::Plain { version: 1 });
    assert_eq!(d.trailer_len, dsf::TRAILER_LEN);
    assert!(
        d.trailer_is_md5,
        "the trailer must be the MD5 of the preceding bytes"
    );
    assert_eq!(d.gap_bytes, 0, "no byte may go unaccounted between chunks");
    assert_eq!(
        d.chunks.len(),
        usize_of(&g, "ref.top_level_chunk_count")?,
        "top-level chunk count drifted"
    );

    // The whole tree, in document order, must reproduce entry for entry.
    let mut flat: Vec<&dsf::Chunk> = Vec::new();
    fn visit<'a>(c: &'a dsf::Chunk, out: &mut Vec<&'a dsf::Chunk>) {
        out.push(c);
        for k in &c.children {
            visit(k, out);
        }
    }
    for c in &d.chunks {
        visit(c, &mut flat);
    }
    assert_eq!(flat.len(), usize_of(&g, "ref.chunk_count_total")?);
    for (i, c) in flat.iter().enumerate() {
        let expect = want(&g, &format!("ref.chunk[{i}]"))?;
        assert_eq!(
            format!("{} offset={} size={}", c.name, c.offset, c.size),
            expect,
            "chunk {i} drifted"
        );
    }
    for (i, c) in d.chunks.iter().enumerate() {
        assert_eq!(
            format!("{} offset={} size={}", c.name, c.offset, c.size),
            want(&g, &format!("ref.top[{i}]"))?,
            "top-level chunk {i} drifted"
        );
    }

    // OBJT is nested inside HEAD; a top-level-only search would find nothing.
    let objt = d
        .find("OBJT")
        .context("OBJT must be reachable in the tree")?;
    assert!(
        objt.offset > 12,
        "OBJT is nested, so it cannot be the first chunk"
    );
    let count = usize_of(&g, "ref.objt_name_count")?;
    assert_eq!(d.object_names.len(), count);
    assert_eq!(d.object_names[0], want(&g, "ref.objt_name[0]")?);
    assert_eq!(
        d.object_names[count - 1],
        want(&g, &format!("ref.objt_name[{}]", count - 1))?
    );
    assert!(
        d.object_names
            .iter()
            .all(|n| n.ends_with(".obj") || n.ends_with(".agp")),
        "OBJT entries are library object references"
    );
    assert!(
        d.unmodeled_bytes > 0,
        "chunk semantics beyond OBJT are unrecovered, so this must not read as zero"
    );
    assert!(
        d.diagnostics(rel).is_empty(),
        "the reference patch is clean: {:?}",
        d.diagnostics(rel)
    );
    Ok(())
}

/// All 1 747 shipped patches: framing, the MD5 trailer and the chunk census.
#[test]
fn every_shipped_dsf_frames() -> Result<()> {
    let Some(root) = xp_core::install_root() else {
        eprintln!("SKIP every_shipped_dsf_frames: XP_INSTALL unset (no coverage claimed)");
        return Ok(());
    };
    let g = read_golden(&golden_dir().join("dsf/corpus_summary.txt"))?;

    let mut plain = 0usize;
    let mut compressed = 0usize;
    let mut unrecognized = 0usize;
    let mut md5_ok = 0usize;
    let mut gaps_nonzero = 0usize;
    let mut objt_names = 0usize;
    let mut census: BTreeMap<String, usize> = BTreeMap::new();
    let mut failed: Vec<String> = Vec::new();

    for dir in ["Custom Scenery", "Resources", "Global Scenery"] {
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
                if p.extension().and_then(|s| s.to_str()) != Some("dsf") {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&p) else {
                    unrecognized += 1;
                    continue;
                };
                match Dsf::parse(&bytes) {
                    Ok(f) => match f.kind {
                        DsfKind::Plain { .. } => {
                            plain += 1;
                            if f.trailer_is_md5 {
                                md5_ok += 1;
                            }
                            if f.gap_bytes != 0 {
                                gaps_nonzero += 1;
                            }
                            objt_names += f.object_names.len();
                            for (k, v) in f.chunk_counts() {
                                *census.entry(k).or_default() += v;
                            }
                        }
                        DsfKind::Compressed7z => compressed += 1,
                        DsfKind::Unknown => unrecognized += 1,
                    },
                    Err(err) => failed.push(format!("{}: {err}", p.display())),
                }
            }
        }
    }

    eprintln!(
        "DSF files: {plain} plain, {compressed} 7z-compressed, {unrecognized} unrecognized; \
         MD5 trailer verified on {md5_ok}; OBJT names read: {objt_names}"
    );
    for f in failed.iter().take(10) {
        eprintln!("  framing failure: {f}");
    }

    assert!(
        failed.is_empty(),
        "{} patches failed to frame, e.g. {:?}",
        failed.len(),
        failed.iter().take(5).collect::<Vec<_>>()
    );
    assert_eq!(plain, usize_of(&g, "plain_xplnedsf")?);
    assert_eq!(compressed, usize_of(&g, "compressed_7z")?);
    assert_eq!(unrecognized, usize_of(&g, "unrecognized")?);
    assert_eq!(
        plain + compressed + unrecognized,
        usize_of(&g, "dsf_files_total")?,
        "the sweep did not cover every patch"
    );
    assert_eq!(md5_ok, usize_of(&g, "trailer_is_md5_of_preceding_bytes")?);
    assert_eq!(
        gaps_nonzero, 0,
        "every shipped patch accounts for all its bytes"
    );
    assert_eq!(objt_names, usize_of(&g, "objt_names_total")?);

    for (name, count) in &census {
        let key = format!("chunk[{name}]");
        if let Some(expect) = g.get(&key) {
            assert_eq!(
                *count,
                expect.parse::<usize>()?,
                "chunk {name} census drifted"
            );
        } else {
            bail!("golden has no census entry for chunk {name} (seen {count} times)");
        }
    }
    for key in g.keys().filter(|k| k.starts_with("chunk[")) {
        let name = key.trim_start_matches("chunk[").trim_end_matches(']');
        assert!(
            census.contains_key(name),
            "golden expects chunk {name}, sweep found none"
        );
    }
    Ok(())
}
