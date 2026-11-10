//! The interface font against numbers measured on the oracle's menu bar.

use anyhow::Result;
use xp_ft::{render_line, Face, LOAD_TARGET_LIGHT};

#[test]
fn menu_titles_have_the_advances_the_oracle_frame_implies() -> Result<()> {
    let Some(root) = std::env::var_os("XP_INSTALL").map(std::path::PathBuf::from) else {
        eprintln!("SKIP menu_titles_have_the_advances_the_oracle_frame_implies: XP_INSTALL unset (no coverage claimed)");
        return Ok(());
    };
    let face = Face::open(&root.join("Resources/fonts/DejaVuSans.ttf"), 10.5)?;
    // item starts on cockpit_menubar.png: 0, 61, 109, 177, 251, 348, 421, 476, 544 — each step is
    // the title's advance + 30
    let titles = [
        "About",
        "File",
        "Aircraft",
        "Location",
        "Environment",
        "Settings",
        "View",
        "Special",
    ];
    let want = [31.0, 18.0, 38.0, 44.0, 67.0, 43.0, 25.0, 38.0];
    for (t, w) in titles.iter().zip(want) {
        let line = render_line(&face, t, LOAD_TARGET_LIGHT)?;
        assert_eq!(line.advance, w, "{t}");
        assert!(
            line.coverage.iter().any(|c| *c > 200),
            "{t} has solid texels"
        );
    }
    // cap height 8 px: rows 5…12 of the bar with the baseline on row 13
    let a = face.glyph('A', LOAD_TARGET_LIGHT)?;
    assert_eq!((a.top, a.rows), (8, 8));
    assert!(xp_ft::version().0 >= 2);
    Ok(())
}
