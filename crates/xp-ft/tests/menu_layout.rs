//! The menu bar's layout, with the original's font metrics, against frames of the original with
//! a drop-down open (`menu_1_about.png`, `menu_2_file.png`, taken with the in-process probe).

use anyhow::Result;
use xp_core::menu;
use xp_ft::{render_line, Face, LOAD_TARGET_LIGHT};

#[test]
fn drop_downs_land_where_the_original_draws_them() -> Result<()> {
    let Some(root) = std::env::var_os("XP_INSTALL").map(std::path::PathBuf::from) else {
        eprintln!("SKIP drop_downs_land_where_the_original_draws_them: XP_INSTALL unset (no coverage claimed)");
        return Ok(());
    };
    let face = Face::open(&root.join("Resources/fonts/DejaVuSans.ttf"), 10.5)?;
    let measure = |t: &str| {
        render_line(&face, t, LOAD_TARGET_LIGHT)
            .map(|b| b.advance as f32)
            .unwrap_or(0.0)
    };
    let menus = menu::load(&root, "English", &measure)?;
    let titles: Vec<&str> = menus.iter().map(|m| m.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "About",
            "File",
            "Aircraft",
            "Location",
            "Environment",
            "Settings",
            "View",
            "Special",
            "Plugins"
        ][..titles.len()]
    );
    // title starts measured earlier on cockpit_menubar.png
    let starts: Vec<f32> = menus.iter().map(|m| m.title_rect.l.floor()).collect();
    assert_eq!(
        &starts[..8],
        &[0.0, 61.0, 109.0, 177.0, 251.0, 348.0, 421.0, 476.0]
    );

    // frames measured on the oracle's own drop-downs (right and bottom exclusive)
    let about = &menus[0].frame;
    assert_eq!(
        (about.l, about.t, about.r, about.b),
        (0.0, 18.0, 113.0, 46.0)
    );
    let file = &menus[1];
    assert_eq!(
        (file.frame.l, file.frame.t, file.frame.r, file.frame.b),
        (61.0, 18.0, 256.0, 178.0)
    );
    assert_eq!(file.items.iter().filter(|i| i.separator).count(), 6);

    // keys as the original prints them, from X-Plane Keys.prf
    let key = |id| {
        file.items
            .iter()
            .find(|i| i.id == id)
            .map(|i| i.key.clone())
    };
    assert_eq!(key(311).as_deref(), Some("SHIFT-space"));
    assert_eq!(key(312).as_deref(), Some("CTRL-space"));
    assert!(file
        .items
        .iter()
        .find(|i| i.id == 312)
        .is_some_and(|i| i.checkable));

    // actions recovered from APP_handlemenu
    let find = |id: i32| {
        menus
            .iter()
            .flat_map(|m| m.items.iter())
            .find(|i| i.id == id)
            .cloned()
    };
    assert_eq!(find(1504).map(|i| i.action), Some(menu::Action::Window(10)));
    assert_eq!(
        find(1807).map(|i| i.action),
        Some(menu::Action::Command("sim/view/chase".into()))
    );
    // hit testing: separators never, items from one pixel under their top
    let it = &file.items[0];
    assert_eq!(menu::item_at(file, 100.0, it.rect.t), None);
    assert_eq!(menu::item_at(file, 100.0, it.rect.t + 1.0), Some(0));
    assert_eq!(menu::item_at(file, 100.0, file.items[1].rect.t + 1.0), None);
    Ok(())
}
