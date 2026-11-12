//! The "Open" aircraft dialog the original shows at start-up.
//!
//! Reference: the user's screenshot of the original, `tests/golden/original/frames/
//! user_aircraft_chooser.png` (1920×1080 window, dialog 1000×761 centred in the content area).
//! Every coordinate below is relative to the dialog's top-left corner and was measured on that
//! screenshot. The chrome is the original's atlas cells (`interface.png`): window frame and
//! panes (`BTN_frame2` 0xe / 0x10), scroll bars, folder and aircraft icons, radio buttons and
//! buttons, each located by matching the screenshot; the sunken white fields are painted from
//! their measured edge rows. 94.0 % of the dialog's pixels are identical to the screenshot and
//! 98.2 % within 8/255; the rest is the preview picture's scaling and a few glyphs. The tree's
//! rules — only folders with an `.acf` somewhere below them, case-insensitive alphabetical order,
//! the path to the current aircraft expanded — are read off the screenshot.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::textures::TextureStore;
use crate::ui::{Layer, Ui};

pub const DIALOG_W: f32 = 1000.0;
pub const DIALOG_H: f32 = 761.0;
const ROW_PITCH: f32 = 15.0;
const INDENT: f32 = 15.0;
/// Left list: white field and the first row.
const LIST: [f32; 4] = [23.0, 54.0, 374.0, 627.0];
const TREE_TEXT_X: f32 = 40.0;
const TREE_FIRST_BASELINE: f32 = 71.0;
/// Livery list.
const LIVERIES: [f32; 4] = [451.0, 369.0, 517.0, 312.0];
const LIVERY_FIRST_CENTRE: f32 = 385.0;
const LIVERY_PITCH: f32 = 20.0;
/// Buttons: x, width; all on rows 727…741.
pub const BTN_RESCAN: [f32; 2] = [24.0, 118.0];
pub const BTN_CANCEL: [f32; 2] = [151.0, 118.0];
pub const BTN_OPEN: [f32; 2] = [278.0, 118.0];
pub const BTN_LIVERY: [f32; 2] = [744.0, 242.0];
const BTN_Y: f32 = 727.0;
const BTN_H: f32 = 15.0;
const TEXT: [u8; 3] = [51, 59, 71];
/// The sunken white fields (l, t, r, b): tree, preview, liveries.
const FIELDS: [[f32; 4]; 3] = [
    [20.0, 54.0, 400.0, 683.0],
    [450.0, 49.0, 980.0, 339.0],
    [450.0, 369.0, 970.0, 683.0],
];
/// The three panes' bodies (l, t, r, b): fitted so the pane cell's edge profiles land on the
/// user's screenshot.
const PANES: [[f32; 4]; 3] = [
    [10.0, 29.0, 430.0, 752.0],
    [440.0, 29.0, 990.0, 350.0],
    [440.0, 359.0, 990.0, 752.0],
];

#[derive(Debug, Clone)]
struct Node {
    name: String,
    /// Relative to the install.
    path: PathBuf,
    depth: usize,
    is_acf: bool,
    expanded: bool,
    children: Vec<Node>,
}

/// An aircraft file. The extension is matched without regard to case: `Aircraft/X-Planes/X-15/
/// X-15.ACF` is spelled in capitals and the original lists it.
pub fn is_acf(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("acf"))
}

fn has_acf_below(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    rd.filter_map(|e| e.ok().map(|e| e.path())).any(|p| {
        if p.is_dir() {
            has_acf_below(&p)
        } else {
            is_acf(&p)
        }
    })
}

fn build(install: &Path, rel: &Path, depth: usize) -> Vec<Node> {
    let Ok(rd) = std::fs::read_dir(install.join(rel)) else {
        return Vec::new();
    };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort_by_key(|p| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });
    let mut out = Vec::new();
    for p in entries {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let child_rel = rel.join(&name);
        if p.is_dir() {
            if has_acf_below(&p) {
                out.push(Node {
                    name,
                    children: build(install, &child_rel, depth + 1),
                    path: child_rel,
                    depth,
                    is_acf: false,
                    expanded: false,
                });
            }
        } else if is_acf(&p) {
            out.push(Node {
                name,
                path: child_rel,
                depth,
                is_acf: true,
                expanded: false,
                children: Vec::new(),
            });
        }
    }
    out
}

fn expand_to(nodes: &mut [Node], target: &Path) {
    for n in nodes {
        if !n.is_acf && target.starts_with(&n.path) {
            n.expanded = true;
            expand_to(&mut n.children, target);
        }
    }
}

fn visible<'a>(nodes: &'a [Node], out: &mut Vec<&'a Node>) {
    for n in nodes {
        out.push(n);
        if n.expanded {
            visible(&n.children, out);
        }
    }
}

fn toggle(nodes: &mut [Node], path: &Path) {
    for n in nodes {
        if n.path == path {
            n.expanded = !n.expanded;
            return;
        }
        if path.starts_with(&n.path) {
            toggle(&mut n.children, path);
        }
    }
}

/// What a click on the dialog asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,
    Open(PathBuf),
    Cancel,
}

pub struct Chooser {
    install: PathBuf,
    /// The tree's root node, "X-Plane 10".
    root: Node,
    pub selected: PathBuf,
    pub livery: usize,
}

impl Chooser {
    pub fn new(install: &Path, current: &Path) -> Chooser {
        let mut children = Vec::new();
        for top in ["Aircraft", "Extra Aircraft"] {
            if has_acf_below(&install.join(top)) {
                children.push(Node {
                    name: top.to_string(),
                    children: build(install, Path::new(top), 2),
                    path: PathBuf::from(top),
                    depth: 1,
                    is_acf: false,
                    expanded: false,
                });
            }
        }
        expand_to(&mut children, current);
        Chooser {
            install: install.to_path_buf(),
            root: Node {
                name: "X-Plane 10".to_string(),
                path: PathBuf::new(),
                depth: 0,
                is_acf: false,
                expanded: true,
                children,
            },
            selected: current.to_path_buf(),
            livery: 0,
        }
    }

    fn rows(&self) -> Vec<&Node> {
        let mut out = Vec::new();
        visible(std::slice::from_ref(&self.root), &mut out);
        out
    }

    /// "default paint" and the folders under the selected aircraft's `liveries/`.
    pub fn liveries(&self) -> Vec<String> {
        let mut out = vec!["default paint".to_string()];
        if let Some(dir) = self.install.join(&self.selected).parent() {
            if let Ok(rd) = std::fs::read_dir(dir.join("liveries")) {
                let mut names: Vec<String> = rd
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect();
                names.sort_by_key(|n| n.to_lowercase());
                out.extend(names);
            }
        }
        out
    }

    /// A click on tree row `row`: an aircraft is selected (opened on a second click), a
    /// folder opens or closes.
    pub fn click_row(&mut self, row: usize) -> Action {
        let hit = self.rows().get(row).map(|n| (n.path.clone(), n.is_acf));
        match hit {
            Some((path, true)) => {
                if self.selected == path {
                    return Action::Open(path);
                }
                self.selected = path;
                self.livery = 0;
            }
            Some((path, false)) if !path.as_os_str().is_empty() => {
                toggle(&mut self.root.children, &path)
            }
            _ => {}
        }
        Action::None
    }

    /// A click at `(x, y)` relative to the dialog's top-left corner.
    pub fn click(&mut self, x: f32, y: f32) -> Action {
        let inside =
            |b: [f32; 2]| (b[0]..b[0] + b[1]).contains(&x) && (BTN_Y..BTN_Y + BTN_H).contains(&y);
        if inside(BTN_OPEN) {
            return Action::Open(self.selected.clone());
        }
        if inside(BTN_CANCEL) || inside(BTN_LIVERY) {
            return Action::Cancel;
        }
        if (LIST[0]..LIST[0] + LIST[2]).contains(&x) && (LIST[1]..LIST[1] + LIST[3]).contains(&y) {
            let first_top = TREE_FIRST_BASELINE - 11.5;
            let row = ((y - first_top) / ROW_PITCH).floor();
            if row >= 0.0 {
                if let Action::Open(p) = self.click_row(row as usize) {
                    return Action::Open(p);
                }
            }
        }
        let (lx, ly, lw, lh) = (LIVERIES[0], LIVERIES[1], LIVERIES[2], LIVERIES[3]);
        if (lx..lx + lw).contains(&x) && (ly..ly + lh).contains(&y) {
            let i = ((y - (LIVERY_FIRST_CENTRE - LIVERY_PITCH * 0.5)) / LIVERY_PITCH).floor();
            if i >= 0.0 && (i as usize) < self.liveries().len() {
                self.livery = i as usize;
            }
        }
        Action::None
    }

    /// Tree rows of the file list: `field` is the white field's left, top and right, `first`
    /// the first row's text start and baseline, `skip` rows scrolled off the top.
    #[allow(clippy::too_many_arguments)]
    pub fn tree_layers(
        &self,
        ui: &Ui,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        field: [f32; 3],
        first: [f32; 2],
        max_rows: usize,
        skip: usize,
    ) -> Result<Vec<Layer>> {
        let mut l = Vec::new();
        let rows = self.rows();
        for (i, n) in rows.iter().skip(skip).take(max_rows).enumerate() {
            let baseline = first[1] + i as f32 * ROW_PITCH;
            if n.is_acf && n.path == self.selected {
                // Rows baseline − 12 … baseline + 3 across the field, 204, the field's sunken
                // edge darkened over it (144/176/195), as on the user's screenshot.
                let y = baseline - 12.0;
                l.push(ui.fill(
                    [field[0], y, field[2] - field[0], 16.0],
                    [204, 204, 204, 255],
                ));
                for (k, c) in [144u8, 176, 195].into_iter().enumerate() {
                    for x in [field[0] + k as f32, field[2] - 1.0 - k as f32] {
                        l.push(ui.fill([x, y, 1.0, 16.0], [c, c, c, 255]));
                    }
                }
            }
            let x = first[0] + n.depth as f32 * INDENT;
            // Folder (atlas 34…47 × 418…431) or aircraft (48…61) icon, 16 left of the text and
            // 10 above the baseline: matched on the user's screenshot (0.16/255 rms).
            let u = if n.is_acf { 48.0 } else { 34.0 };
            l.push(ui.atlas_cell(
                [u, 418.0, u + 13.0, 431.0],
                [x - 16.0, baseline - 10.0, 13.0, 13.0],
            ));
            // Tree text is black; the selected row's is the dialog text colour.
            let selected = n.is_acf && n.path == self.selected;
            let rgb = if selected { TEXT } else { [0; 3] };
            l.push(ui.text_at(device, queue, &n.name, [x, baseline], rgb)?);
        }
        Ok(l)
    }

    /// Visible tree rows.
    pub fn row_count(&self) -> usize {
        self.rows().len()
    }

    /// The preview tile (`plot_a_tile`, 0x1005b2dc0, at full size): a grey (130) 250×250
    /// quad from (cx − 125, cy − 50) to (cx + 125, cy + 200), the aircraft's `<name>_icon.png`
    /// 10 inside it at its own size (230×230 here), a black `OGL_grayrect` 30 high under it, and
    /// the path split at its last '/' right-aligned 5 in from the right edge, 12 and 26 below
    /// the tile's bottom. `at` is the tile's top-left corner.
    pub fn preview_layers(
        &self,
        ui: &Ui,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        store: &mut TextureStore,
        at: [f32; 2],
    ) -> Result<Vec<Layer>> {
        let mut l = Vec::new();
        let acf = self.install.join(&self.selected);
        let icon = acf.with_file_name(format!(
            "{}_icon.png",
            acf.file_stem()
                .map(|s| s.to_string_lossy())
                .unwrap_or_default()
        ));
        l.push(ui.fill([at[0], at[1], 250.0, 250.0], [130, 130, 130, 255]));
        if icon.is_file() {
            let view = store.get_ui(device, queue, &icon)?;
            let mut layer = Layer::bitmap(view, [at[0] + 10.0, at[1] + 10.0], [1.0; 4]);
            layer.rect[2] = 230.0;
            layer.rect[3] = 230.0;
            l.push(layer);
        }
        l.push(ui.fill([at[0], at[1] + 250.0, 250.0, 30.0], [0, 0, 0, 255]));
        let shown = self.selected.to_string_lossy().into_owned();
        let (head, tail) = match shown.rfind('/') {
            Some(i) => (shown[..=i].to_string(), shown[i + 1..].to_string()),
            None => (String::new(), shown),
        };
        l.push(ui.text_right(
            device,
            queue,
            &head,
            [at[0] + 245.0, at[1] + 262.0],
            [255; 3],
        )?);
        l.push(ui.text_right(
            device,
            queue,
            &tail,
            [at[0] + 245.0, at[1] + 276.0],
            [255; 3],
        )?);
        Ok(l)
    }

    /// The dialog as layers, with its top-left corner at `origin`.
    pub fn layers(
        &self,
        ui: &Ui,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        store: &mut TextureStore,
        origin: [f32; 2],
    ) -> Result<Vec<Layer>> {
        let at = |x: f32, y: f32| [origin[0] + x, origin[1] + y];
        let mut l = Vec::new();
        let r = |x0: f32, y0: f32, x1: f32, y1: f32| xp_core::menu::Rect {
            l: origin[0] + x0,
            t: origin[1] + y0,
            r: origin[0] + x1,
            b: origin[1] + y1,
        };
        // Window and panes: `BTN_frame2` types 0xe and 0x10 from the atlas.
        l.extend(ui.window(
            device,
            queue,
            r(0.0, 0.0, DIALOG_W, DIALOG_H),
            "Open",
            false,
        )?);
        for p in PANES {
            l.extend(ui.pane(r(p[0], p[1], p[2], p[3])));
        }
        let chrome = ui.solid_image(device, queue, &chrome_pixels());
        l.push(Layer::bitmap(chrome, at(0.0, 18.0), [1.0; 4]));
        // Scroll bars (atlas column x 83…101: up cap, thumb, down cap; track x 106…117),
        // places matched on the user's screenshot to within 1.2/255 rms.
        for (x, top, bottom) in [(406.0, 35.0, 682.0), (971.0, 360.0, 682.0)] {
            l.extend(ui.nine_slice(
                r(x + 3.0, top + 11.0, x + 14.0, bottom + 11.0),
                [106.0, 228.0, 117.0, 260.0],
                [0.0, 3.0, 0.0, 3.0],
                [0.0; 4],
            ));
            l.push(ui.atlas_cell(
                [83.0, 224.0, 101.0, 246.0],
                [at(x, 0.0)[0], at(0.0, top)[1], 18.0, 22.0],
            ));
            l.push(ui.atlas_cell(
                [83.0, 274.0, 101.0, 296.0],
                [at(x, 0.0)[0], at(0.0, bottom)[1], 18.0, 22.0],
            ));
            l.push(ui.atlas_cell(
                [83.0, 247.0, 101.0, 273.0],
                [at(x, 0.0)[0], at(0.0, top + 16.0)[1], 18.0, 26.0],
            ));
        }
        l.push(ui.text_centred(device, queue, "Aircraft", at(221.0, 45.0), TEXT)?);

        l.extend(self.tree_layers(
            ui,
            device,
            queue,
            [
                at(FIELDS[0][0], 0.0)[0],
                at(0.0, FIELDS[0][1])[1],
                at(FIELDS[0][2], 0.0)[0],
            ],
            at(TREE_TEXT_X, TREE_FIRST_BASELINE),
            ((LIST[3] - 16.0) / ROW_PITCH) as usize,
            0,
        )?);
        l.extend(self.preview_layers(ui, device, queue, store, at(590.0, 51.0))?);

        // Liveries.
        for (i, name) in self.liveries().iter().enumerate() {
            let cy = LIVERY_FIRST_CENTRE + i as f32 * LIVERY_PITCH;
            // Radio button: atlas 83…101 × 352…370 (on) / 371…389 (off) at (461, cy − 8),
            // matched on the user's screenshot (0.25/255 rms for the off state).
            let v = if i == self.livery { 352.0 } else { 371.0 };
            l.push(ui.atlas_cell([83.0, v, 101.0, v + 18.0], {
                let p = at(461.0, cy - 8.0);
                [p[0], p[1], 18.0, 18.0]
            }));
            l.push(ui.text_at(device, queue, name, at(482.0, cy + 5.0), TEXT)?);
        }

        // Buttons: the atlas cell x 304…356, y 0…26 (found by matching the user's screenshot),
        // its 8-texel caps kept and the middle stretched, 4 px outside the face each side.
        for b in [BTN_RESCAN, BTN_CANCEL, BTN_OPEN, BTN_LIVERY] {
            l.extend(ui.nine_slice(
                r(b[0] - 4.0, BTN_Y - 5.0, b[0] + b[1] + 4.0, BTN_Y + 21.0),
                [304.0, 0.0, 356.0, 26.0],
                [8.0, 0.0, 8.0, 0.0],
                [0.0; 4],
            ));
        }
        // Button labels.
        for (b, label) in [
            (BTN_RESCAN, "Re-Scan"),
            (BTN_CANCEL, "Cancel"),
            (BTN_OPEN, "Open Aircraft"),
            (BTN_LIVERY, "Change Livery and Keep Flying"),
        ] {
            l.push(ui.text_centred(
                device,
                queue,
                label,
                at(b[0] + b[1] * 0.5 - 1.0, BTN_Y + 12.0),
                TEXT,
            )?);
        }
        Ok(l)
    }
}

/// The dialog body below the title strip, `DIALOG_W` × (`DIALOG_H` − 18), as flat measured
/// colours: the pinstripe (rows alternate 247/224), the two recessed panes (201), the white
/// fields, the scrollbar gutters and the button faces.
fn chrome_pixels() -> (u32, u32, Vec<u8>) {
    let (w, h) = (DIALOG_W as usize, (DIALOG_H - 18.0) as usize);
    let mut px = vec![0u8; w * h * 4];
    let mut rect = |x0: f32, y0: f32, x1: f32, y1: f32, c: [u8; 3]| {
        for y in (y0 as usize).max(18)..(y1 as usize).min(h + 18) {
            for x in (x0 as usize)..(x1 as usize).min(w) {
                let i = ((y - 18) * w + x) * 4;
                px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
    };
    // The white fields are sunken: their edge rows and columns measured on the user's
    // screenshot (top 119/184/220/244, sides 180/220/244, bottom 244/231).
    for f in FIELDS {
        rect(f[0], f[1], f[2], f[3], [255; 3]);
        for (k, c) in [119u8, 184, 220, 244].into_iter().enumerate() {
            let y = f[1] + k as f32;
            rect(f[0], y, f[2], y + 1.0, [c; 3]);
        }
        for (k, c) in [180u8, 220, 244].into_iter().enumerate() {
            let (xl, xr) = (f[0] + k as f32, f[2] - 1.0 - k as f32);
            rect(xl, f[1] + 1.0 + k as f32, xl + 1.0, f[3], [c; 3]);
            rect(xr, f[1] + 1.0 + k as f32, xr + 1.0, f[3], [c; 3]);
        }
        rect(f[0] + 3.0, f[3] - 2.0, f[2] - 3.0, f[3] - 1.0, [244; 3]);
        rect(f[0] + 1.0, f[3] - 1.0, f[2] - 1.0, f[3], [231; 3]);
    }
    (w as u32, h as u32, px)
}
