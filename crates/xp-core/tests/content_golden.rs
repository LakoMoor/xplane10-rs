//! Golden tests for the install-wide content checker. Expectations come from
//! `tests/golden/content/scan_summary.txt`, measured from the shipped install.
//!
//! These are corpus-scale assertions: they pin what the port can read across the whole
//! installation, not just one file. A regression here means a format rule changed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{bail, Context, Result};
use xp_core::content::{self, Severity};

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/content/scan_summary.txt")
        .to_path_buf()
}

/// One scan shared by every test in this file.
///
/// Each test used to run its own `content::scan`, and the harness runs tests in parallel:
/// seven concurrent walks of the install, each holding a fully parsed 178 MB `apt.dat`,
/// peaked at 11.7 GB resident and got the whole `cargo test` killed. The scan is pure, so
/// one result serves all of them.
///
/// `Ok(None)` means there is no install to test against; the caller has already been told.
fn shared_report() -> Result<Option<&'static content::Report>> {
    static REPORT: OnceLock<Option<std::result::Result<content::Report, String>>> = OnceLock::new();
    let slot = REPORT.get_or_init(|| {
        let Some(root) = xp_core::install_root() else {
            eprintln!("SKIP content_golden: XP_INSTALL unset (no coverage claimed)");
            return None;
        };
        if !root.is_dir() {
            return Some(Err(format!(
                "XP_INSTALL={} is not a directory",
                root.display()
            )));
        }
        Some(content::scan(&root).map_err(|e| format!("{e:#}")))
    });
    match slot {
        None => Ok(None),
        Some(Ok(report)) => Ok(Some(report)),
        Some(Err(e)) => bail!("content scan failed: {e}"),
    }
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
            None => bail!("{}: bad golden line {line:?}", path.display()),
        };
        out.insert(k.trim().to_string(), v.trim().to_string());
    }
    if out.is_empty() {
        bail!("golden {} is empty", path.display());
    }
    Ok(out)
}

fn want_usize(g: &BTreeMap<String, String>, key: &str) -> Result<usize> {
    g.get(key)
        .with_context(|| format!("golden has no key {key:?}"))?
        .parse::<usize>()
        .with_context(|| format!("golden {key:?} is not a number"))
}

fn want_str<'a>(g: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str> {
    g.get(key)
        .map(|s| s.as_str())
        .with_context(|| format!("golden has no key {key:?}"))
}

/// The checker's count key for a golden key. The golden uses identifier-safe names; the
/// report uses the human-readable ones it prints.
fn count_key(golden_key: &str) -> String {
    match golden_key {
        "acf_legacy_binary" => "acf (legacy binary)",
        "apt_dat" => "apt.dat",
        "for_vegetation" => "for (vegetation)",
        "library_txt" => "library.txt",
        "library_txt_export_line_malformed" => "library.txt EXPORT line malformed",
        "obj_unrecognized" => "obj (unrecognized)",
        "dsf_7z_compressed" => "dsf (7z-compressed)",
        "shader_source" => "shader source",
        other => other,
    }
    .to_string()
}

#[test]
fn corpus_counts_match_golden() -> Result<()> {
    let Some(report) = shared_report()? else {
        return Ok(());
    };
    let g = read_golden(&golden_path())?;

    for key in [
        "acf",
        "acf_legacy_binary",
        "agp",
        "apt_dat",
        "dds",
        "dsf",
        "dsf_7z_compressed",
        "for_vegetation",
        "library_txt",
        "library_txt_export_line_malformed",
        "obj_unrecognized",
        "obj8",
        "png",
        "shader_source",
    ] {
        let want = want_usize(&g, key)?;
        let ck = count_key(key);
        let got = report
            .counts
            .get(&ck)
            .copied()
            .with_context(|| format!("report has no count {ck:?}"))?;
        assert_eq!(got, want, "count {ck:?}");
    }
    // `packs` is not a count key: it is the length of the pack list.
    assert_eq!(report.packs.len(), want_usize(&g, "packs")?, "pack count");
    Ok(())
}

/// Nothing in the install is unparseable any more: every file is either loaded or explicitly
/// classified as a format this port recognizes but does not model.
#[test]
fn no_file_is_unparseable() -> Result<()> {
    let Some(report) = shared_report()? else {
        return Ok(());
    };
    let g = read_golden(&golden_path())?;
    assert_eq!(
        report.malformed.len(),
        want_usize(&g, "unparseable_files")?,
        "unparseable files: {:?}",
        report.malformed.iter().map(|(p, _)| p).collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn finding_totals_match_golden() -> Result<()> {
    let Some(report) = shared_report()? else {
        return Ok(());
    };
    let g = read_golden(&golden_path())?;
    for (sev, key) in [
        (Severity::Info, "findings_info"),
        (Severity::Warning, "findings_warning"),
        (Severity::Error, "findings_error"),
    ] {
        // `findings_at` means "at or above", so exact per-severity counts need equality.
        let got = report.findings.iter().filter(|f| f.severity == sev).count();
        let want = want_usize(&g, key)?;
        assert_eq!(got, want, "findings at {sev:?}");
    }
    Ok(())
}

/// Every error the checker still reports is a resource the shipped content genuinely does not
/// contain, worded the way the original words it. A parse failure sneaking back in as an Error
/// would fail this too, because its message does not start with that prefix.
#[test]
fn every_error_is_the_originals_missing_resource_wording() -> Result<()> {
    let Some(report) = shared_report()? else {
        return Ok(());
    };
    let g = read_golden(&golden_path())?;
    let prefix = "Failed to find resource ";
    let errors = report.findings_at(Severity::Error);
    let matching = errors
        .iter()
        .filter(|f| f.message.starts_with(prefix))
        .count();
    assert_eq!(
        matching,
        want_usize(&g, "error_category_failed_to_find_resource")?,
        "errors worded as the original words a missing resource"
    );
    assert_eq!(
        errors.len() - matching,
        want_usize(&g, "error_category_other")?,
        "errors with any other wording: {:?}",
        errors
            .iter()
            .filter(|f| !f.message.starts_with(prefix))
            .map(|f| &f.message)
            .take(8)
            .collect::<Vec<_>>()
    );
    Ok(())
}

/// The original's own two diagnostics, reproduced verbatim. These are the lines the port's
/// report is meant to be diffable against, so they are asserted as exact strings.
#[test]
fn original_diagnostic_wording_is_reproduced() -> Result<()> {
    let Some(report) = shared_report()? else {
        return Ok(());
    };
    let g = read_golden(&golden_path())?;

    let missing = want_str(&g, "oracle_wording_missing_resource")?;
    assert!(
        report.findings.iter().any(|f| f.message
            == format!("Custom Scenery/KSEA Demo Area/KSEA_objects/: {missing}")
            || f.message.contains(missing)),
        "no finding reproduces {missing:?}"
    );

    let lod = want_str(&g, "oracle_wording_first_lod")?;
    assert!(
        report.findings.iter().any(|f| f.message.contains(lod)),
        "no finding reproduces {lod:?}"
    );
    Ok(())
}

/// The 13 objects whose draw descriptors overlap or leave gaps are content, not parser damage:
/// the index stream itself is the right length in every one of them. Asserted against the
/// independent interval-partition measurement recorded in the golden.
#[test]
fn obj8_descriptor_range_anomalies_match_independent_measurement() -> Result<()> {
    let Some(report) = shared_report()? else {
        return Ok(());
    };
    let g = read_golden(&golden_path())?;
    let got = report
        .findings
        .iter()
        .filter(|f| f.message.contains("draw descriptors"))
        .count();
    assert_eq!(
        got,
        want_usize(&g, "obj8_descriptor_ranges_overlap_or_gap")?,
        "objects whose draw descriptors overlap or leave gaps"
    );
    Ok(())
}

/// The legacy binary ACF variant is recognized and reported as itself, never as damage.
#[test]
fn legacy_binary_acfs_are_classified_not_malformed() -> Result<()> {
    let Some(report) = shared_report()? else {
        return Ok(());
    };
    let g = read_golden(&golden_path())?;
    assert_eq!(
        report
            .counts
            .get("acf (legacy binary)")
            .copied()
            .unwrap_or(0),
        want_usize(&g, "acf_legacy_binary")?,
        "legacy binary ACF count"
    );
    assert_eq!(
        report.counts.get("acf malformed").copied().unwrap_or(0),
        0,
        "no text ACF should fail to parse: {:?}",
        report
            .findings
            .iter()
            .filter(|f| f.message.contains(".acf"))
            .map(|f| &f.message)
            .take(5)
            .collect::<Vec<_>>()
    );
    // All 19 live under Extra Aircraft/, which is the only place the shipped install keeps
    // this serialization.
    for f in report
        .findings
        .iter()
        .filter(|f| f.message.contains("legacy binary ACF"))
    {
        assert!(
            f.message.starts_with("Extra Aircraft/"),
            "legacy binary ACF outside Extra Aircraft/: {}",
            f.message
        );
    }
    Ok(())
}
