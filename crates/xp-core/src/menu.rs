//! The menu bar and its drop-downs, as the original builds and lays them out.
//!
//! Sources, all from the pinned oracle build:
//!
//! - **Contents**: `Resources/menus/<language>/X-Plane.txt`, read by
//!   `mbar_class::mbar_read_menus` (`0x100e02c60`). Line 1 is the line-ending marker, line 2 the
//!   version; then `0 <title>` opens a menu, `<id> <text>` adds an item (text `-` is a
//!   separator), and a line `-` closes the menu. Item ids are `menu_index · 300 + position`.
//! - **Geometry**: `mbar_create_menu` (`0x100dfa710`) and `mbar_append_menu_item`
//!   (`0x100dffdb0`). A title is its measured text plus 30 wide, laid left to right from x = 0,
//!   18 pixels tall at the top of the screen. A drop-down hangs 2 pixels below its title; items
//!   are 12 pixels tall, separators 2; every item of a menu is as wide as the widest
//!   `40 + text + key` of the menu, and never less than 40.
//! - **Drawing** (`mbar_draw_menus`, `0x100dfd910`): the frame is `BTN_frame2` type 0x14 around
//!   the items' box grown by 2 above and below; the open title and the item under the pointer are
//!   filled with palette 0x2d at 25 %; item text at (left + 18, bottom + 2) in palette 0x2f
//!   (0x30 when disabled); `[key]` right-aligned 10 pixels inside the right edge.
//! - **Actions**: `APP_handlemenu` (`0x1005de650`), recovered into `data/menu_actions.tsv`.
//! - **Keys and check boxes**: `mbar_get_check_cmnd` (`0x100dfb5b0`) into `data/menu_marks.tsv`;
//!   the key text is `keys_class::get_key_for_cmnd` over `Output/preferences/X-Plane Keys.prf`.
//!
//! Coordinates here are top-down screen pixels (the original's are GL, y up); the conversion is
//! `y_screen = height − y_gl`, and every rectangle is the original's exactly.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};

const ACTIONS: &str = include_str!("../data/menu_actions.tsv");
const MARKS: &str = include_str!("../data/menu_marks.tsv");
const COMMANDS: &str = include_str!("../data/commands.tsv");
const PALETTE: &str = include_str!("../data/ui_palette.tsv");

/// Height of the menu bar and of a title.
pub const BAR_HEIGHT: f32 = 18.0;
/// Extra width of a title beyond its text (`mbar_create_menu`, the float 30 at `0x1010b2fd8`).
pub const TITLE_PAD: f32 = 30.0;
/// Item and separator heights (`0x1010b3128` = 12, `0x1010b3064` = 2).
pub const ITEM_HEIGHT: f32 = 12.0;
pub const SEPARATOR_HEIGHT: f32 = 2.0;
/// Gap between the bar and the first item, and the frame's margin above and below the items.
pub const DROP_GAP: f32 = 2.0;
/// Minimum item width and the extra width beyond text + key (`0x1010b2fec` = 40).
pub const ITEM_PAD: f32 = 40.0;
/// Text inset from an item's left edge (`mbar_draw_menus`: 18 for the bar's menus).
pub const TEXT_INSET: f32 = 18.0;
/// Key text inset from the right edge (`0x101089530` = 10).
pub const KEY_INSET: f32 = 10.0;

/// A rectangle in top-down screen pixels: left, top, right, bottom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub l: f32,
    pub t: f32,
    pub r: f32,
    pub b: f32,
}

impl Rect {
    /// The original's hit test: inclusive on every edge.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.l <= x && x <= self.r && self.t <= y && y <= self.b
    }
    pub fn w(&self) -> f32 {
        self.r - self.l
    }
    pub fn h(&self) -> f32 {
        self.b - self.t
    }
}

/// What choosing an item does (`APP_handlemenu`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// `wins_class::start_win(n)`: open window `n`.
    Window(i32),
    /// `cmnd_class::single_press(n)`: run command `n` of the command table.
    Command(String),
    /// Handled by its own code in the original (About, Open Aircraft, situations, replays…).
    Special(String),
    /// Nothing recovered.
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: i32,
    pub text: String,
    pub separator: bool,
    /// Key binding text without brackets, empty when none.
    pub key: String,
    pub checkable: bool,
    pub rect: Rect,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub title: String,
    pub title_rect: Rect,
    pub items: Vec<Item>,
    /// The drop-down's frame: the items' box grown by 2 above and below.
    pub frame: Rect,
}

/// A menu as the file declares it: its title and `(id, text)` items.
pub type Declared = (String, Vec<(i32, String)>);

/// The parsed menu file: titles and their items, in file order.
pub fn parse(text: &str) -> Result<Vec<Declared>> {
    let mut lines = text.lines();
    let marker = lines.next().unwrap_or("").trim();
    if !matches!(marker, "A" | "I") {
        bail!("menu file: line 1 is {marker:?}, want A or I");
    }
    lines.next();
    let mut menus: Vec<Declared> = Vec::new();
    let mut open = false;
    for (n, raw) in lines.enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line == "-" {
            open = false;
            continue;
        }
        let (id, rest) = match line.split_once(char::is_whitespace) {
            Some((id, rest)) => (id, rest.trim()),
            None => bail!("menu file line {}: {line:?}", n + 3),
        };
        let id: i32 = id
            .parse()
            .with_context(|| format!("menu file line {}: bad id {id:?}", n + 3))?;
        if id == 0 && !open {
            menus.push((rest.to_string(), Vec::new()));
            open = true;
        } else if let Some(m) = menus.last_mut() {
            m.1.push((id, rest.to_string()));
        }
    }
    Ok(menus)
}

fn actions() -> BTreeMap<i32, Action> {
    let mut out = BTreeMap::new();
    for l in ACTIONS.lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = l.split('\t').collect();
        let (Some(id), Some(kind), Some(v)) = (f.first(), f.get(1), f.get(2)) else {
            continue;
        };
        let Ok(id) = id.parse::<i32>() else { continue };
        let a = match *kind {
            "win" => Action::Window(v.parse().unwrap_or(-1)),
            "cmd" => Action::Command(f.get(3).unwrap_or(&"").to_string()),
            "special" => Action::Special(f.get(3).unwrap_or(&"").to_string()),
            _ => Action::None,
        };
        out.insert(id, a);
    }
    out
}

fn marks() -> BTreeMap<i32, (String, bool)> {
    MARKS
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            Some((
                f.first()?.parse().ok()?,
                (
                    f.get(1)
                        .map(|s| if *s == "-" { "" } else { s })
                        .unwrap_or("")
                        .to_string(),
                    f.get(2) == Some(&"1"),
                ),
            ))
        })
        .collect()
}

/// The command table's paths by index.
pub fn command_name(index: usize) -> Option<&'static str> {
    COMMANDS
        .lines()
        .filter(|l| !l.starts_with('#'))
        .nth(index)
        .and_then(|l| l.split('\t').nth(2))
}

/// The interface palette entry `i` as 8-bit RGB.
pub fn palette(i: usize) -> [u8; 3] {
    PALETTE
        .lines()
        .filter(|l| !l.starts_with('#'))
        .nth(i)
        .map(|l| {
            let f: Vec<f32> = l
                .split('\t')
                .skip(1)
                .filter_map(|v| v.parse().ok())
                .collect();
            [0, 1, 2].map(|k| (f.get(k).copied().unwrap_or(0.0) * 255.0).round() as u8)
        })
        .unwrap_or([255, 0, 255])
}

/// Key bindings by command path, as `get_key_for_cmnd` prints them: the modifiers as the file
/// spells them (`SHIFT`, `SHIFT+CTRL`), a `-`, then the key name with its first letter in lower
/// case; no modifiers, no `-`. The first binding of a command in the file wins.
pub fn key_bindings(prf: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in prf.lines().skip(2) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 3 {
            continue;
        }
        let (key, mods, cmd) = (f[0], f[1], f[2]);
        let mut name = key.to_string();
        if let Some(c) = name.get(0..1) {
            name.replace_range(0..1, &c.to_lowercase());
        }
        let text = if mods == "<none>" {
            name
        } else {
            format!("{mods}-{name}")
        };
        out.entry(cmd.to_string()).or_insert(text);
    }
    out
}

/// Read the bar for `language` and lay it out for a screen `width` wide.
pub fn load(install: &Path, language: &str, measure: &dyn Fn(&str) -> f32) -> Result<Vec<Menu>> {
    let path = install.join(format!("Resources/menus/{language}/X-Plane.txt"));
    let text =
        crate::read_text_lossy(&path).with_context(|| format!("reading {}", path.display()))?;
    let keys = crate::read_text_lossy(install.join("Output/preferences/X-Plane Keys.prf"))
        .map(|t| key_bindings(&t))
        .unwrap_or_default();
    let mut declared = parse(&text)?;
    // The plugin system adds "Plugins" at run time (`XPCreateMenu`; `mbar_create_menu` checks
    // for that title by name). With no plugin menus it has no items.
    declared.push(("Plugins".to_string(), Vec::new()));
    Ok(layout(&declared, &keys, measure))
}

/// Lay out parsed menus with the original's rules.
pub fn layout(
    menus: &[Declared],
    keys: &BTreeMap<String, String>,
    measure: &dyn Fn(&str) -> f32,
) -> Vec<Menu> {
    let actions = actions();
    let marks = marks();
    let mut x = 0.0f32;
    let mut out = Vec::with_capacity(menus.len());
    for (title, items) in menus {
        let title_w = measure(title) + TITLE_PAD;
        let title_rect = Rect {
            l: x,
            t: 0.0,
            r: (x + title_w).trunc(),
            b: BAR_HEIGHT,
        };
        x += title_w;
        let mut built: Vec<Item> = Vec::with_capacity(items.len());
        let mut top = BAR_HEIGHT + DROP_GAP;
        let mut width = ITEM_PAD;
        for (id, text) in items {
            let separator = text.starts_with('-');
            let (key_cmd, checkable) = marks.get(id).cloned().unwrap_or_default();
            let key = keys.get(&key_cmd).cloned().unwrap_or_default();
            width = width
                .max(ITEM_PAD + measure(text) + if key.is_empty() { 0.0 } else { measure(&key) });
            let h = if separator {
                SEPARATOR_HEIGHT
            } else {
                ITEM_HEIGHT
            };
            built.push(Item {
                id: *id,
                text: text.clone(),
                separator,
                key,
                checkable,
                rect: Rect {
                    l: title_rect.l,
                    t: top,
                    r: title_rect.l,
                    b: top + h,
                },
                action: actions.get(id).cloned().unwrap_or(Action::None),
            });
            top += h;
        }
        for it in &mut built {
            it.rect.r = (title_rect.l + width).trunc();
        }
        let frame = match (built.first(), built.last()) {
            (Some(f), Some(l)) => Rect {
                l: f.rect.l,
                t: f.rect.t - DROP_GAP,
                r: f.rect.r,
                b: l.rect.b + DROP_GAP,
            },
            _ => Rect {
                l: title_rect.l,
                t: BAR_HEIGHT,
                r: title_rect.l,
                b: BAR_HEIGHT,
            },
        };
        out.push(Menu {
            title: title.clone(),
            title_rect,
            items: built,
            frame,
        });
    }
    out
}

/// The menu whose title is under the pointer.
pub fn title_at(menus: &[Menu], x: f32, y: f32) -> Option<usize> {
    menus.iter().position(|m| m.title_rect.contains(x, y))
}

/// The selectable item under the pointer in an open menu (`mbar_select_an_item`: separators
/// and `=` items never are; the hover highlight tests the item's top edge one pixel inside).
pub fn item_at(menu: &Menu, x: f32, y: f32) -> Option<usize> {
    menu.items.iter().position(|it| {
        !it.separator
            && !it.text.starts_with('=')
            && it.rect.l <= x
            && x <= it.rect.r
            && it.rect.t + 1.0 <= y
            && y <= it.rect.b
    })
}
