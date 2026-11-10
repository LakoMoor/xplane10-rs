//! Install-wide content checker.
//!
//! Walks a shipped X-Plane 10 installation and reports what it could load, what is missing,
//! what it does not understand, and what is malformed — **without launching the simulator**.
//! Parsing success is never reported as execution compatibility: this says nothing about
//! whether the port can *run* a pack, only about whether it can read it.
//!
//! The diagnostic wording is copied from the original wherever the original has wording, so
//! the two reports can be diffed line for line. Measured against X-Plane 10.51r2's Log.txt:
//!
//! ```text
//! E/SCN: Failed to find resource 'test/AGtrees_test.for', referenced from file
//!        'Custom Scenery/KSEA Demo Area/KSEA_objects/'.
//! W/OBJ: The object … has a first LOD near distance of 2000.000000 but it should start at 0!
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::aptdat::AptDat;
use crate::obj8::{self, ObjectRole};
use crate::scenery::{Agp, Library};

/// How loud a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Something the original would also accept and merely note.
    Info,
    Warning,
    Error,
}

/// One line of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    /// Message with the original's wording where the original has wording.
    pub message: String,
}

/// Per-pack summary, so a huge install can be read at a glance.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackSummary {
    /// Pack root, relative to the installation root. A pack is a directory holding
    /// `library.txt`.
    pub root: String,
    pub exports: usize,
    pub private: bool,
    /// `EXPORT` lines whose source field this reader does not resolve (see [`scan`]).
    pub export_sources_unmodeled: usize,
    /// `EXPORT` lines in this pack's `library.txt` that carry only one field.
    pub malformed_export_lines: usize,
    pub agp_files: usize,
    pub agp_references: usize,
    pub missing_references: usize,
    pub dsf_patches: usize,
    pub dsf_compressed: usize,
    pub obj8_files: usize,
    pub apt_dat_airports: usize,
    /// Directives and chunk semantics this reader keeps but cannot interpret.
    pub unmodeled_directives: usize,
    pub unmodeled_dsf_bytes: usize,
}

/// The whole scan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub root: PathBuf,
    /// Resource kind → file count. Keys are stable and sorted.
    pub counts: BTreeMap<String, usize>,
    pub packs: Vec<PackSummary>,
    pub findings: Vec<Finding>,
    /// Files this port could not parse: install-relative path + the full error chain.
    /// Kept separate from `findings` so a run can list every unparseable file, not just the
    /// first 40 diagnostics.
    pub malformed: Vec<(String, String)>,
}

impl Report {
    /// Findings at or above `severity`, in the order they were produced.
    pub fn findings_at(&self, severity: Severity) -> Vec<&Finding> {
        self.findings
            .iter()
            .filter(|f| f.severity >= severity)
            .collect()
    }

    /// A stable text rendering, for diffing against the original's Log.txt.
    pub fn summary_text(&self) -> String {
        use std::fmt::Write as _;
        let mut s = String::new();
        let _ = writeln!(s, "content scan of {}", self.root.display());
        for (k, v) in &self.counts {
            let _ = writeln!(s, "  {k}: {v}");
        }
        let _ = writeln!(s, "  packs: {}", self.packs.len());
        let _ = writeln!(
            s,
            "  findings: info={} warning={} error={}",
            self.findings
                .iter()
                .filter(|f| f.severity == Severity::Info)
                .count(),
            self.findings
                .iter()
                .filter(|f| f.severity == Severity::Warning)
                .count(),
            self.findings
                .iter()
                .filter(|f| f.severity == Severity::Error)
                .count(),
        );
        for f in self.findings_at(Severity::Error).into_iter().take(40) {
            let _ = writeln!(s, "  [{:?}] {}", f.severity, f.message);
        }
        s
    }
}

/// Scan an installation root.
pub fn scan(root: &Path) -> Result<Report> {
    let mut report = Report {
        root: root.to_path_buf(),
        ..Default::default()
    };

    // Packs first: a pack root is what resource paths resolve against.
    let mut pack_roots: Vec<PathBuf> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
    collect(root, &mut pack_roots, &mut files)?;
    pack_roots.sort();
    files.sort();

    let mut packs: BTreeMap<PathBuf, PackSummary> = BTreeMap::new();
    for path in &files {
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let pack_root = pack_root_for(path, root, &pack_roots);

        match name {
            "library.txt" => match Library::load(path) {
                Ok(lib) => {
                    bump(&mut report.counts, "library.txt");
                    let p = ensure_pack(&mut packs, root, &pack_root);
                    p.exports = lib.exports.len();
                    p.private = lib.private;
                    p.unmodeled_directives += lib.other.len();
                    // The first EXPORT field is *not* a path relative to the pack root.
                    // Measured: `EXPORT KSEA_Objects/KSEA_objects/_Fuel_Depot.agp …` names a
                    // directory twice nested while the file sits at `KSEA_objects/_Fuel_Depot.agp`
                    // (`KSEA_Objects` and `KSEA_objects` are one case-insensitive directory), and
                    // the original reports nothing about it. Counted, not resolved.
                    p.export_sources_unmodeled += lib.exports.len();
                    // One-field EXPORT lines are a defect in the line, not in the file: the file
                    // still loads and its other exports still resolve. Reported per line so the
                    // count is checkable against the shipped text.
                    for (line_no, raw) in &lib.malformed {
                        bump(&mut report.counts, "library.txt EXPORT line malformed");
                        p.malformed_export_lines += 1;
                        report.findings.push(Finding {
                            severity: Severity::Warning,
                            message: format!(
                                "{rel} line {line_no}: EXPORT has one field, wants \
                                 <source> <name>: {raw:?}"
                            ),
                        });
                    }
                }
                Err(err) => {
                    bump(&mut report.counts, "library.txt malformed");
                    report.findings.push(Finding {
                        severity: Severity::Error,
                        message: format!("{rel}: {err:#}"),
                    });
                    report.malformed.push((rel.clone(), format!("{err:#}")));
                }
            },
            _ if name.ends_with(".agp") => match Agp::load(path) {
                Ok(agp) => {
                    bump(&mut report.counts, "agp");
                    let p = ensure_pack(&mut packs, root, &pack_root);
                    p.agp_files += 1;
                    p.agp_references += agp.references.len();
                    p.unmodeled_directives += agp.other.len();
                    // Measured against the original: `Alaska.agp` names `textures/kseatk.png`,
                    // `AlaskaLite.obj` and `objects/com_Hangars.obj`, all of which exist beside
                    // the .agp, and X-Plane 10.51r2 reported only `test/AGtrees_test.for`. So
                    // references resolve against the referencing file's own directory.
                    let agp_dir = path.parent().unwrap_or(root);
                    for r in &agp.references {
                        if !agp_dir.join(&r.path).exists() {
                            p.missing_references += 1;
                            let dir = relative_dir(path, root);
                            report.findings.push(Finding {
                                severity: Severity::Error,
                                message: format!(
                                    "Failed to find resource '{}', referenced from file '{dir}/'",
                                    r.path
                                ),
                            });
                        }
                    }
                }
                Err(err) => {
                    bump(&mut report.counts, "agp malformed");
                    report.findings.push(Finding {
                        severity: Severity::Error,
                        message: format!("{rel}: {err:#}"),
                    });
                    report.malformed.push((rel.clone(), format!("{err:#}")));
                }
            },
            _ if name.ends_with(".acf") => match crate::acf::Acf::load(path) {
                Ok(acf) => {
                    bump(&mut report.counts, "acf");
                    let _ = acf;
                }
                Err(err) => {
                    // A text-ACF parse failure has two very different causes in this install,
                    // and reporting them alike would be a false claim about the content.
                    if crate::acf::is_legacy_binary_file(path) {
                        bump(&mut report.counts, "acf (legacy binary)");
                        report.findings.push(Finding {
                            severity: Severity::Info,
                            message: format!(
                                "{rel}: not the X-Plane 10 text ACF format — legacy binary ACF \
                                 ({size} bytes); recognized by this port, not yet modeled",
                                size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
                            ),
                        });
                    } else {
                        bump(&mut report.counts, "acf malformed");
                        report.findings.push(Finding {
                            severity: Severity::Error,
                            message: format!("{rel}: {err:#}"),
                        });
                        report.malformed.push((rel.clone(), format!("{err:#}")));
                    }
                }
            },
            _ if name.ends_with(".obj") => {
                let Ok(text) = crate::read_text_lossy(path) else {
                    bump(&mut report.counts, "obj unreadable");
                    continue;
                };
                match obj8::classify(&text) {
                    obj8::ObjKind::Obj8 { .. } => {
                        bump(&mut report.counts, "obj8");
                        match obj8::Obj8::parse(&text) {
                            Ok(o) => {
                                ensure_pack(&mut packs, root, &pack_root).obj8_files += 1;
                                ensure_pack(&mut packs, root, &pack_root).unmodeled_directives +=
                                    o.raw_directives.len();
                                let role = if rel.starts_with("Aircraft/") {
                                    ObjectRole::Aircraft
                                } else {
                                    ObjectRole::Scenery
                                };
                                for d in o.diagnostics(&rel, role) {
                                    report.findings.push(Finding {
                                        severity: match d.severity {
                                            obj8::Severity::Warning => Severity::Warning,
                                            obj8::Severity::Error => Severity::Error,
                                        },
                                        message: d.message,
                                    });
                                }
                            }
                            Err(err) => {
                                bump(&mut report.counts, "obj8 malformed");
                                report.findings.push(Finding {
                                    severity: Severity::Error,
                                    message: format!("{rel}: {err:#}"),
                                });
                                report.malformed.push((rel.clone(), format!("{err:#}")));
                            }
                        }
                    }
                    obj8::ObjKind::Wavefront => bump(&mut report.counts, "obj (wavefront)"),
                    obj8::ObjKind::Unknown => bump(&mut report.counts, "obj (unrecognized)"),
                }
            }
            _ if name.ends_with(".dsf") => {
                let Ok(bytes) = std::fs::read(path) else {
                    bump(&mut report.counts, "dsf unreadable");
                    continue;
                };
                match crate::dsf::Dsf::parse(&bytes) {
                    Ok(f) => {
                        match f.kind {
                            crate::dsf::DsfKind::Plain { .. } => {
                                bump(&mut report.counts, "dsf");
                                let p = ensure_pack(&mut packs, root, &pack_root);
                                p.dsf_patches += 1;
                                p.unmodeled_dsf_bytes += f.unmodeled_bytes;
                            }
                            crate::dsf::DsfKind::Compressed7z => {
                                bump(&mut report.counts, "dsf (7z-compressed)");
                                ensure_pack(&mut packs, root, &pack_root).dsf_compressed += 1;
                            }
                            crate::dsf::DsfKind::Unknown => {
                                bump(&mut report.counts, "dsf (unrecognized)");
                            }
                        }
                        for m in f.diagnostics(&rel) {
                            report.findings.push(Finding {
                                severity: Severity::Info,
                                message: m,
                            });
                        }
                    }
                    Err(err) => {
                        bump(&mut report.counts, "dsf malformed");
                        report.findings.push(Finding {
                            severity: Severity::Error,
                            message: format!("{rel}: {err:#}"),
                        });
                        report.malformed.push((rel.clone(), format!("{err:#}")));
                    }
                }
            }
            _ if name == "apt.dat" => match AptDat::load(path) {
                Ok(a) => {
                    bump(&mut report.counts, "apt.dat");
                    let p = ensure_pack(&mut packs, root, &pack_root);
                    p.apt_dat_airports += a.airport_count();
                    if !a.skipped_lines.is_empty() {
                        report.findings.push(Finding {
                            severity: Severity::Warning,
                            message: format!(
                                "{rel} leaves {} records unattached to any group",
                                a.skipped_lines.len()
                            ),
                        });
                    }
                }
                Err(err) => {
                    bump(&mut report.counts, "apt.dat malformed");
                    report.findings.push(Finding {
                        severity: Severity::Error,
                        message: format!("{rel}: {err:#}"),
                    });
                    report.malformed.push((rel.clone(), format!("{err:#}")));
                }
            },
            _ if name.ends_with(".for") => bump(&mut report.counts, "for (vegetation)"),
            _ if name.ends_with(".png") => bump(&mut report.counts, "png"),
            _ if name.ends_with(".dds") => bump(&mut report.counts, "dds"),
            _ if name.ends_with(".glsl") || name.ends_with(".vert") || name.ends_with(".frag") => {
                bump(&mut report.counts, "shader source")
            }
            _ => {}
        }
    }

    report.packs = packs.into_values().collect();
    report.packs.sort_by(|a, b| a.root.cmp(&b.root));
    Ok(report)
}

/// The pack a file belongs to: the nearest ancestor holding `library.txt`, else its own
/// directory. Resource paths in `.agp` and `library.txt` resolve against this.
fn pack_root_for(path: &Path, root: &Path, pack_roots: &[PathBuf]) -> PathBuf {
    let dir = path.parent().unwrap_or(root).to_path_buf();
    let mut cur: Option<&Path> = Some(dir.as_path());
    while let Some(c) = cur {
        if pack_roots.iter().any(|p| p == c) {
            return c.to_path_buf();
        }
        if c == root {
            break;
        }
        cur = c.parent();
    }
    dir
}

fn relative_dir(path: &Path, root: &Path) -> String {
    let dir = path.parent().unwrap_or(root);
    dir.strip_prefix(root)
        .unwrap_or(dir)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Get (creating if needed) the summary for one pack.
fn ensure_pack<'a>(
    packs: &'a mut BTreeMap<PathBuf, PackSummary>,
    root: &Path,
    pack_root: &Path,
) -> &'a mut PackSummary {
    packs
        .entry(pack_root.to_path_buf())
        .or_insert_with(|| PackSummary {
            root: pack_root
                .strip_prefix(root)
                .unwrap_or(pack_root)
                .to_string_lossy()
                .into_owned(),
            ..Default::default()
        })
}

fn bump(m: &mut BTreeMap<String, usize>, key: &str) {
    *m.entry(key.to_string()).or_default() += 1;
}

fn collect(dir: &Path, packs: &mut Vec<PathBuf>, files: &mut Vec<PathBuf>) -> Result<()> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, packs, files)?;
        } else if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
            if name == "library.txt" {
                packs.push(p.parent().unwrap_or(&p).to_path_buf());
            }
            files.push(p);
        }
    }
    Ok(())
}
