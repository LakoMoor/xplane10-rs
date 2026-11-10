//! Scenery pack description: `library.txt` and `.agp` aggregate-placement files.
//!
//! Measured from shipped packs. `library.txt` is an OBJ8-family text file:
//!
//! ```text
//! A
//! 800
//! LIBRARY
//! PRIVATE                      <- this pack's exports are not public
//! EXPORT KSEA_objects/KSEA_objects/Alaska.agp KSEA_objects/Alaska.agp
//! ```
//!
//! Bigger packs add `EXPORT_EXCLUDE`, `REGION_DEFINE`, `REGION` and `REGION_BITMAP`
//! (`Resources/default scenery/1000 world terrain/library.txt` carries 8 068
//! `EXPORT_EXCLUDE` lines against 4 017 `EXPORT` lines). Every directive this reader does not
//! model is kept verbatim, so a pack is never silently half-read.
//!
//! `.agp` files (`A` / `1000` / `AG_POINT`) name the resources an aggregate places:
//! `TEXTURE`, `TEXTURE_LIT`, `TEXTURE_NORMAL <bump> <path>`, `TEXTURE_SCALE`,
//! `TEXTURE_WIDTH`, `VEGETATION <path.for>` and `OBJECT <path.obj>`.
//!
//! Resource paths are resolved against the **pack root** — the directory holding
//! `library.txt` — which is what the original's own report implies: it printed
//! `Failed to find resource 'test/AGtrees_test.for', referenced from file
//! 'Custom Scenery/KSEA Demo Area/KSEA_objects/'`, where `test/` sits beside `KSEA_objects/`,
//! not inside it.

use anyhow::{bail, Context, Result};

/// Header check shared by the OBJ8-family text files (`A`, `<version>`, `<flavor>`).
fn parse_header(lines: &[&str], flavor: &str, file: &str) -> Result<u32> {
    if lines.len() < 3 {
        bail!("{file} is too short ({} lines)", lines.len());
    }
    if lines[0].trim_end() != "A" {
        bail!(
            "line 1: {file} must start with 'A', got {:?}",
            lines[0].trim_end()
        );
    }
    let version = lines[1]
        .trim_end()
        .parse::<u32>()
        .with_context(|| format!("line 2: {file} version is not a number"))?;
    if lines[2].trim_end() != flavor {
        bail!(
            "line 3: {file} must declare {flavor:?}, got {:?}",
            lines[2].trim_end()
        );
    }
    Ok(version)
}

/// One `EXPORT <source> <public name>` line. The separator is any whitespace run — shipped
/// files use either a single space or two TABs. `source` is a library alias (`/lib/global8/us/…`,
/// `KSEA_objects/…`), not a path relative to the pack root; see `content` for why it is counted
/// rather than resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    pub source: String,
    pub name: String,
    pub line: usize,
}

/// A parsed `library.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    pub version: u32,
    /// `PRIVATE` seen anywhere in the file.
    pub private: bool,
    /// Exports in file order. Shipped packs repeat each line up to three times; that
    /// repetition is preserved rather than deduplicated, because the original reads the file
    /// sequentially and the duplicates are part of the content.
    pub exports: Vec<Export>,
    /// Directives this reader keeps but does not interpret (`EXPORT_EXCLUDE`, `REGION*`, …).
    pub other: Vec<(usize, String)>,
    /// `#` comment lines, kept verbatim.
    pub comments: Vec<(usize, String)>,
    /// `EXPORT` lines that carry only one field: line number + the raw line. Content defects
    /// in individual lines, not a format this reader lacks.
    pub malformed: Vec<(usize, String)>,
}

impl Library {
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Library> {
        let path = path.as_ref();
        let text = crate::read_text_lossy(path)
            .with_context(|| format!("reading library.txt {}", path.display()))?;
        Library::parse(&text).with_context(|| format!("parsing library.txt {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Library> {
        let lines: Vec<&str> = text.lines().collect();
        let version = parse_header(&lines, "LIBRARY", "library.txt")?;

        let mut out = Library {
            version,
            private: false,
            exports: Vec::new(),
            other: Vec::new(),
            comments: Vec::new(),
            malformed: Vec::new(),
        };

        for (n, raw) in lines.iter().enumerate().skip(3) {
            let line_no = n + 1;
            let line = raw.trim_end();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('#') {
                out.comments.push((line_no, line.to_string()));
                continue;
            }
            let mut it = line.split_whitespace();
            match it.next().unwrap_or("") {
                "PRIVATE" => out.private = true,
                "EXPORT" => {
                    let source = it.next().unwrap_or("");
                    let name = it.next().unwrap_or("");
                    if source.is_empty() || name.is_empty() {
                        // Measured: `Resources/default scenery/900 us objects/library.txt`
                        // carries four EXPORT lines with no separator at all between the two
                        // fields (`…_600r60.objbuildings/B2_e2_52x25.obj`). The rest of that
                        // file parses, and the separator elsewhere in it is whitespace of any
                        // kind — 450 of its 705 EXPORT lines separate with two TABs. A
                        // one-field EXPORT is a defect in that line, not in the file, so it is
                        // recorded and the scan continues.
                        out.malformed.push((line_no, line.to_string()));
                        continue;
                    }
                    out.exports.push(Export {
                        source: source.to_string(),
                        name: name.to_string(),
                        line: line_no,
                    });
                }
                other => {
                    out.other.push((
                        line_no,
                        format!("{other} {}", it.collect::<Vec<_>>().join(" ")),
                    ));
                }
            }
        }
        Ok(out)
    }
}

/// A resource reference inside an `.agp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRef {
    /// The directive that named it: `OBJECT`, `VEGETATION`, `TEXTURE`, …
    pub directive: String,
    /// Path exactly as written, relative to the pack root.
    pub path: String,
    pub line: usize,
}

/// A parsed `.agp` aggregate-placement file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agp {
    pub version: u32,
    /// Line 3 flavor, normally `AG_POINT`.
    pub flavor: String,
    pub references: Vec<ResourceRef>,
    /// Directives kept but not interpreted.
    pub other: Vec<(usize, String)>,
}

impl Agp {
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Agp> {
        let path = path.as_ref();
        let text = crate::read_text_lossy(path)
            .with_context(|| format!("reading AGP {}", path.display()))?;
        Agp::parse(&text).with_context(|| format!("parsing AGP {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Agp> {
        let lines: Vec<&str> = text.lines().collect();
        let flavor = lines
            .get(2)
            .map(|l| l.trim_end().to_string())
            .unwrap_or_default();
        let version = parse_header(&lines, &flavor, "AGP")?;

        let mut out = Agp {
            version,
            flavor,
            references: Vec::new(),
            other: Vec::new(),
        };

        for (n, raw) in lines.iter().enumerate().skip(3) {
            let line_no = n + 1;
            let line = raw.trim_end();
            if line.is_empty() || line.starts_with('#') || line.starts_with('*') {
                continue;
            }
            let mut it = line.split_whitespace();
            let tok = it.next().unwrap_or("");
            let rest: Vec<&str> = it.collect();

            // `TEXTURE_NORMAL` carries a bump strength before its path, so the path is the
            // last field, not the first. Measured from Alaska.agp:
            //   TEXTURE_NORMAL 1.0 textures/kseatk_NML.png
            let path_field = match tok {
                "OBJECT" | "VEGETATION" | "TEXTURE" | "TEXTURE_LIT" | "TEXTURE_DRAPED" => {
                    rest.first().copied()
                }
                "TEXTURE_NORMAL" => rest.last().copied(),
                _ => None,
            };

            if let Some(path) = path_field {
                out.references.push(ResourceRef {
                    directive: tok.to_string(),
                    path: path.to_string(),
                    line: line_no,
                });
            } else if !rest.is_empty() || !tok.is_empty() {
                out.other
                    .push((line_no, format!("{tok} {}", rest.join(" "))));
            }
        }
        Ok(out)
    }
}
