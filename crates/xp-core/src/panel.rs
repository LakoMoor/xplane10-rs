//! The 2-D cockpit panel: where each instrument of an ACF's `PANEL_2D` block goes on screen.
//!
//! Measured against the oracle frame `tests/golden/original/frames/cockpit_default.png`
//! (1024×768, default Cessna, default view):
//!
//! - the panel bitmap `cockpit/-PANELS-/Panel.png` (1480×1024) is drawn one texel per pixel
//!   with its top-left corner at screen (0, −100);
//! - an instrument's `POS x y` is the **centre** of its bitmap, x to the right from the
//!   panel's left edge and y **upward** from a line 664 px below the top of the screen:
//!   `asi_preset_GA_200` at `POS 295 267` is centred at screen (295, 397) and `vvi2k_GA` at
//!   `POS 577 116` at (577, 548).
//!
//! Both numbers are FITTED to that one frame. They are consistent with each other — the
//! panel's bottom edge is at screen y = 924, so `POS y = 0` is 260 px above it — but the rule
//! behind them (the panel scroll, `zoom_class::Pscroll_y_ctr_now`) is not ported.
//!
//! An instrument's bitmap comes from its `ins_type`, through the original's own table:
//! `ins_paths(int, …)` (oracle `0x1000693d0`) builds `"cockpit/" + ins_path[ins_type]`, first
//! under the aircraft's folder and then under `Resources/bitmaps/`. The table (`_ins_path`,
//! 916 entries, with the matching `_ins_enum` names) is dumped from the binary into
//! `data/ins_table.tsv`. The instance *name* in the ACF is the enum name, not a file name:
//! `vvi2k_GA` is `standard six/vertical speed/VVI_2000_GA.png`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::acf::Acf;

/// Screen position of the panel bitmap's top-left corner on a 1024×768 screen. FITTED.
pub const PANEL_ORIGIN_PX: [f32; 2] = [0.0, -100.0];

/// Screen y of the panel coordinate `POS y = 0` on a 1024×768 screen. FITTED.
pub const POS_ZERO_SCREEN_Y: f32 = 664.0;

/// One instrument of the 2-D panel.
#[derive(Debug, Clone, PartialEq)]
pub struct Instrument {
    pub name: String,
    pub ins_type: i64,
    /// `POS x y`, panel coordinates.
    pub pos: [f32; 2],
    /// `SIZE`, when the instance carries one. Parsed, not yet applied.
    pub size: Option<f32>,
}

impl Instrument {
    /// Centre of the instrument on a 1024×768 screen, pixels from the top-left corner.
    pub fn screen_center(&self) -> [f32; 2] {
        [
            PANEL_ORIGIN_PX[0] + self.pos[0],
            POS_ZERO_SCREEN_Y - self.pos[1],
        ]
    }
}

/// The instruments of an aircraft's `PANEL_2D` block that carry a `POS`, in file order.
pub fn instruments(acf: &Acf) -> Result<Vec<Instrument>> {
    let mut out = Vec::new();
    for inst in &acf.panel_2d.instances {
        let Some((_, pos)) = inst.attrs.iter().find(|(k, _)| k == "POS") else {
            continue;
        };
        let mut it = pos.split_whitespace();
        let (Some(x), Some(y)) = (it.next(), it.next()) else {
            anyhow::bail!("line {}: POS wants two numbers, got {pos:?}", inst.line);
        };
        let name = inst
            .name_line
            .split_whitespace()
            .next()
            .with_context(|| format!("line {}: instrument has no name", inst.line))?;
        let size = inst
            .attrs
            .iter()
            .find(|(k, _)| k == "SIZE")
            .and_then(|(_, v)| v.trim().parse::<f32>().ok());
        out.push(Instrument {
            name: name.to_string(),
            ins_type: inst.ins_type,
            pos: [
                x.parse()
                    .with_context(|| format!("line {}: bad POS x", inst.line))?,
                y.parse()
                    .with_context(|| format!("line {}: bad POS y", inst.line))?,
            ],
            size,
        });
    }
    Ok(out)
}

const INS_TABLE: &str = include_str!("../data/ins_table.tsv");

/// One row of the original's instrument table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsEntry {
    /// `_ins_enum[ins_type]`, the name an ACF instance carries.
    pub name: &'static str,
    /// `_ins_path[ins_type]`, relative to a `cockpit/` folder.
    pub path: &'static str,
}

/// The table row for an instrument type, if the type is in range.
pub fn ins_entry(ins_type: i64) -> Option<InsEntry> {
    let want = ins_type.to_string();
    INS_TABLE
        .lines()
        .filter(|l| !l.starts_with('#'))
        .find_map(|l| {
            let mut f = l.split('\t');
            (f.next()? == want).then_some(())?;
            Some(InsEntry {
                name: f.next()?,
                path: f.next()?,
            })
        })
}

/// Every `.png` under the aircraft's `cockpit/` and under `Resources/bitmaps/cockpit/`, by
/// lower-cased path relative to that `cockpit/` folder. The aircraft's own copy wins, which is
/// the order `ins_paths` tries them in.
pub struct BitmapIndex {
    by_rel: BTreeMap<String, PathBuf>,
}

fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, PathBuf>) -> Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("listing {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(root, &p, out)?;
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("png"))
        {
            if let Ok(rel) = p.strip_prefix(root) {
                out.entry(rel.to_string_lossy().to_lowercase())
                    .or_insert(p.clone());
            }
        }
    }
    Ok(())
}

impl BitmapIndex {
    pub fn build(install: &Path, aircraft_dir: &Path) -> Result<BitmapIndex> {
        let mut by_rel = BTreeMap::new();
        let own = aircraft_dir.join("cockpit");
        if own.is_dir() {
            walk(&own, &own, &mut by_rel)?;
        }
        let shared = install.join("Resources/bitmaps/cockpit");
        walk(&shared, &shared, &mut by_rel)?;
        Ok(BitmapIndex { by_rel })
    }

    /// The base bitmap of an instrument type. Matched without regard to case: the table says
    /// `rad_navcom1_GA.png` and the file is `rad_NAVCOM1_GA.png`.
    pub fn base(&self, ins_type: i64) -> Option<&Path> {
        let entry = ins_entry(ins_type)?;
        self.by_rel
            .get(&entry.path.to_lowercase())
            .map(|p| p.as_path())
    }
}
