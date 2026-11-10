//! The 2-D panel layout against the shipped Cessna and the oracle frame's measured positions.

use anyhow::{Context, Result};
use xp_core::acf::Acf;
use xp_core::panel::{self, BitmapIndex};

#[test]
fn cessna_panel_layout_matches_the_measured_positions() -> Result<()> {
    let Some(root) = xp_core::install_root() else {
        eprintln!("SKIP cessna_panel_layout_matches_the_measured_positions: XP_INSTALL unset (no coverage claimed)");
        return Ok(());
    };
    let dir = root.join("Aircraft/General Aviation/Cessna 172SP");
    let acf = Acf::load(dir.join("Cessna_172SP.acf"))?;
    let ins = panel::instruments(&acf)?;
    assert_eq!(ins.len(), 76);

    let find = |name: &str| {
        ins.iter()
            .find(|i| i.name == name)
            .with_context(|| format!("no instrument {name}"))
    };
    // centres measured on tests/golden/original/frames/cockpit_default.png
    assert_eq!(find("asi_preset_GA_200")?.screen_center(), [295.0, 397.0]);
    assert_eq!(find("vvi2k_GA")?.screen_center(), [577.0, 548.0]);
    assert_eq!(find("ins_stall")?.size, Some(0.25));
    // switches sit below POS y = 0, still on the 1480x1024 bitmap whose bottom is at y = 924
    let battery = find("but_battery")?.screen_center();
    assert!(battery[1] > 768.0 && battery[1] < 924.0, "{battery:?}");

    let index = BitmapIndex::build(&root, &dir)?;
    // every instance name in the ACF is the binary's enum name for its type
    for i in &ins {
        let e = panel::ins_entry(i.ins_type).with_context(|| format!("type {}", i.ins_type))?;
        assert_eq!(e.name, i.name, "ins_type {}", i.ins_type);
    }
    assert_eq!(
        panel::ins_entry(find("vvi2k_GA")?.ins_type).map(|e| e.path),
        Some("standard six/vertical speed/VVI_2000_GA.png")
    );
    let unresolved: Vec<&str> = ins
        .iter()
        .filter(|i| index.base(i.ins_type).is_none())
        .map(|i| i.name.as_str())
        .collect();
    assert_eq!(unresolved, UNRESOLVED, "instruments with no bitmap on disk");
    // the aircraft's own copy wins over Resources/bitmaps/cockpit
    let asi = index
        .base(find("asi_preset_GA_200")?.ins_type)
        .context("asi bitmap")?;
    assert!(asi.starts_with(&dir), "{}", asi.display());
    Ok(())
}

/// Filled in from the first run; see the test.
const UNRESOLVED: [&str; 0] = [];
