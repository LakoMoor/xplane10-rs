//! The flight model against the original's own `Cycle Dump.txt`
//! (`tests/golden/original/cycle_dump.txt`, written by the oracle for the default Cessna).

use std::path::Path;

use anyhow::{Context, Result};
use xp_core::acf::Acf;
use xp_core::flight::wing::Wing;

/// The report's wing names and the ACF wing each one is. The pairing is read off the two
/// files (spans, chords and airfoils agree); the original's own name table is not ported.
const WINGS: [(&str, usize); 8] = [
    ("PROPELLER 1", 0),
    ("LEFT WING 1", 8),
    ("RIGT WING 1", 9),
    ("LEFT WING 2", 10),
    ("RIGT WING 2", 11),
    ("LEFT H STAB", 16),
    ("RIGT H STAB", 17),
    ("VERT STAB 1", 18),
];

#[test]
fn wing_element_build_up_reproduces_the_cycle_dump() -> Result<()> {
    let Some(root) = xp_core::install_root() else {
        eprintln!("SKIP wing_element_build_up_reproduces_the_cycle_dump: XP_INSTALL unset (no coverage claimed)");
        return Ok(());
    };
    let dump = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/original/cycle_dump.txt"),
    )?;
    let dump: Vec<&str> = dump.lines().collect();
    let acf = Acf::load(root.join("Aircraft/General Aviation/Cessna 172SP/Cessna_172SP.acf"))?;

    let (mut elements, mut lines) = (0usize, 0usize);
    for (name, index) in WINGS {
        let header = format!("Finite-Wing & Element Build-Up for {name}:");
        let at = dump
            .iter()
            .position(|l| *l == header)
            .with_context(|| format!("dump has no section {header:?}"))?;
        let wing =
            Wing::from_acf(&acf, index)?.with_context(|| format!("ACF has no wing {index}"))?;
        for (i, got) in wing.report_lines().iter().enumerate() {
            assert_eq!(got, dump[at + 1 + i], "{name}, line {i}");
            lines += 1;
        }
        elements += wing.elements.len();
    }
    assert_eq!(elements, 54, "strips over the eight wings");
    assert_eq!(lines, 54 + 8 * 5);
    // every other wing slot of the ACF is empty, as the report has no section for it
    let present: Vec<usize> = (0..60)
        .filter(|i| matches!(Wing::from_acf(&acf, *i), Ok(Some(_))))
        .collect();
    assert_eq!(present, WINGS.map(|(_, i)| i));
    Ok(())
}
