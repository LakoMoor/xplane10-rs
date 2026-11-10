//! `xp-content` — walk a shipped X-Plane 10 installation and report what the port can read.
//!
//! Usage: `cargo run -p xp-core --example xp-content -- "/path/to/X-Plane 10"`
//!
//! This never launches the simulator, and reading a file is not the same as running it: see
//! `docs/COMPATIBILITY.md` for the four separate columns that distinction is tracked in.

use anyhow::{Context, Result};
use xp_core::content::{self, Finding, Severity};

fn main() -> Result<()> {
    let root = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .or_else(xp_core::install_root)
        .context("usage: xp-content <install-root>  (or set XP_INSTALL)")?;

    if !root.is_dir() {
        anyhow::bail!("not a directory: {}", root.display());
    }

    let report = content::scan(&root)?;
    print!("{}", report.summary_text());

    let mut findings: Vec<&Finding> = report.findings.iter().collect();
    findings.sort_by_key(|f| (f.severity, f.message.clone()));
    let mut split: [Vec<&Finding>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for f in findings {
        split[f.severity as usize].push(f);
    }

    // Every unparseable file, with the format-level cause: the outer message is only the path,
    // and the cause is the part that says which rule the parser is still missing.
    println!();
    println!(
        "unparseable files: {} (first 20 shown)",
        report.malformed.len()
    );
    for (path, chain) in report.malformed.iter().take(20) {
        println!("  {path}\n      {chain}");
    }

    let limit = std::env::var("XP_CONTENT_LIMIT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(20);
    for (name, list) in [
        ("errors", &split[Severity::Error as usize]),
        ("warnings", &split[Severity::Warning as usize]),
        ("info", &split[Severity::Info as usize]),
    ] {
        println!();
        println!("{name}: {} (first {limit} shown)", list.len());
        for f in list.iter().take(limit) {
            println!("  {}", f.message);
        }
    }

    println!();
    println!("packs scanned: {}", report.packs.len());
    for p in &report.packs {
        println!(
            "  {:<52} exports={:<5} private={:<5} agp={:<4} refs={:<5} missing={:<4} dsf={:<5} ({} compressed) obj8={:<5} airports={}",
            p.root,
            p.exports,
            p.private,
            p.agp_files,
            p.agp_references,
            p.missing_references,
            p.dsf_patches,
            p.dsf_compressed,
            p.obj8_files,
            p.apt_dat_airports,
        );
    }
    Ok(())
}
