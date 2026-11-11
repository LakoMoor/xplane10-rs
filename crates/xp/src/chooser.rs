//! The "Open" aircraft dialog the original shows at start-up.
//!
//! Reference: the user's screenshot of the original, `tests/golden/original/frames/
//! user_aircraft_chooser.png` (1920×1080 window, dialog 1000×761 centred in the content area).
//! Every coordinate below is relative to the dialog's top-left corner and was measured on that
//! screenshot. The title strip is the same `interface.png` cell as the menu bar and is exact;
//! the rest of the chrome (pinstripe body, panes, bevels, buttons) is drawn here from the
//! measured colours as flat fills, NOT from the atlas cells the original uses, which are not yet
//! identified. Folder and aircraft icons in the tree are not drawn. The tree's rules — only
//! folders with an `.acf` somewhere below them, case-insensitive alphabetical order, the path to
//! the current aircraft expanded — are read off the screenshot.

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
const TREE_FIRST_BASELINE: f32 = 70.0;
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
                let hit = self
                    .rows()
                    .get(row as usize)
                    .map(|n| (n.path.clone(), n.is_acf));
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
        let chrome = ui.solid_image(device, queue, &chrome_pixels());
        l.push(Layer::bitmap(chrome, at(0.0, 18.0), [1.0; 4]));
        l.push(ui.title_strip(at(0.0, 0.0), DIALOG_W));
        l.push(ui.text_centred(device, queue, "Open", at(DIALOG_W * 0.5, 13.0), [255; 3])?);
        l.push(ui.text_centred(device, queue, "Aircraft", at(210.0, 44.0), TEXT)?);

        // Tree.
        let rows = self.rows();
        let max_rows = ((LIST[3] - 16.0) / ROW_PITCH) as usize;
        for (i, n) in rows.iter().take(max_rows).enumerate() {
            let baseline = TREE_FIRST_BASELINE + i as f32 * ROW_PITCH;
            if n.is_acf && n.path == self.selected {
                l.push(ui.fill(
                    [
                        at(LIST[0], 0.0)[0],
                        at(0.0, baseline - 11.5)[1],
                        LIST[2],
                        ROW_PITCH,
                    ],
                    [204, 204, 204, 255],
                ));
            }
            let x = TREE_TEXT_X + n.depth as f32 * INDENT;
            l.push(ui.text_at(device, queue, &n.name, at(x, baseline), TEXT)?);
        }

        // Preview: the aircraft's `<name>_icon.png`, and its path in a black caption.
        let acf = self.install.join(&self.selected);
        let icon = acf.with_file_name(format!(
            "{}_icon.png",
            acf.file_stem()
                .map(|s| s.to_string_lossy())
                .unwrap_or_default()
        ));
        if icon.is_file() {
            let view = store.get_ui(device, queue, &icon)?;
            let mut layer = Layer::bitmap(view, at(590.0, 50.0), [1.0; 4]);
            layer.rect[2] = 250.0;
            layer.rect[3] = 250.0;
            l.push(layer);
        }
        l.push(ui.fill(
            [at(590.0, 0.0)[0], at(0.0, 300.0)[1], 250.0, 30.0],
            [0, 0, 0, 255],
        ));
        let shown = self.selected.to_string_lossy().into_owned();
        let (head, tail) = match shown.rfind('/') {
            Some(i) => (shown[..=i].to_string(), shown[i + 1..].to_string()),
            None => (String::new(), shown),
        };
        l.push(ui.text_right(device, queue, &head, at(836.0, 312.0), [255; 3])?);
        l.push(ui.text_right(device, queue, &tail, at(836.0, 326.0), [255; 3])?);

        // Liveries.
        for (i, name) in self.liveries().iter().enumerate() {
            let cy = LIVERY_FIRST_CENTRE + i as f32 * LIVERY_PITCH;
            let ring = if i == self.livery {
                [70, 160, 70, 255]
            } else {
                [180, 180, 180, 255]
            };
            l.push(ui.fill([at(464.0, 0.0)[0], at(0.0, cy - 6.0)[1], 12.0, 12.0], ring));
            l.push(ui.text_at(device, queue, name, at(482.0, cy + 4.0), TEXT)?);
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
                at(b[0] + b[1] * 0.5, BTN_Y + 11.0),
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
    for y in 18..(DIALOG_H as usize) {
        let c = if y % 2 == 0 { 247 } else { 224 };
        rect(0.0, y as f32, DIALOG_W, y as f32 + 1.0, [c; 3]);
    }
    // panes: bevel line, then the recess
    for (x0, y0, x1, y1) in [
        (8.0, 28.0, 432.0, 752.0),
        (437.0, 28.0, 997.0, 350.0),
        (437.0, 358.0, 997.0, 752.0),
    ] {
        rect(x0, y0, x1, y1, [178; 3]);
        rect(x0 + 2.0, y0 + 2.0, x1 - 2.0, y1 - 2.0, [208; 3]);
    }
    rect(
        LIST[0],
        LIST[1],
        LIST[0] + LIST[2],
        LIST[1] + LIST[3],
        [255; 3],
    );
    rect(409.0, 40.0, 419.0, 697.0, [225; 3]);
    rect(451.0, 49.0, 967.0, 336.0, [255; 3]);
    rect(
        LIVERIES[0],
        LIVERIES[1],
        LIVERIES[0] + LIVERIES[2],
        LIVERIES[1] + LIVERIES[3],
        [255; 3],
    );
    rect(974.0, 365.0, 984.0, 697.0, [225; 3]);
    for b in [BTN_RESCAN, BTN_CANCEL, BTN_OPEN, BTN_LIVERY] {
        rect(b[0], BTN_Y, b[0] + b[1], BTN_Y + BTN_H, [103, 105, 102]);
        rect(
            b[0] + 1.0,
            BTN_Y + 1.0,
            b[0] + b[1] - 1.0,
            BTN_Y + BTN_H - 1.0,
            [165, 166, 162],
        );
        rect(
            b[0] + 1.0,
            BTN_Y + 8.0,
            b[0] + b[1] - 1.0,
            BTN_Y + BTN_H - 1.0,
            [185, 186, 181],
        );
    }
    (w as u32, h as u32, px)
}
