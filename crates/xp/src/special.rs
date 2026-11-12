//! Window contents the `BTN_*` tables do not carry: drawn by loops over run-time data.
//!
//! * `BTN_list_and_search` (0x100dc2e30), the airport list of `airp_wind` (Select Airport and
//!   QuickFlight): a sunken field (`BTN_frame2` 0x13) 290 × 220 around (x, y), 21 rows 10
//!   apart with the scroll index in the middle, name at x + 20 and ICAO at x + 242 (palette
//!   0x36), the selected row washed with palette 0x60 at 25 % over (x + 18, row − 5, w − 22,
//!   10), and a `BTN_vert_slider` at x + w + 16.
//! * `BTN_vert_slider` (0x100dc2390): `BTN_frame0` track from y0 − 4 to y1 + 4, then (when
//!   max > min) the up cap centred at y1 + 8, the thumb between y0 + 9 (max) and y1 − 10 (min),
//!   the down cap centred at y0 − 10; atlas column u 83…101.
//! * `qflt_butn` (0x1005edb00): QuickFlight's time and weather pictures, cells of
//!   `bitmaps/qflite.png` (column c at u 101c + 1…+91, row r at v 151r + 1…+151), a
//!   `BTN_frame2` 0x12 behind (x − 3, y − 3, 96, 155), the 0x15 glow (x + 5, y + 5, 80, 139) on
//!   the chosen one, its label centred at (x + 45, y + 5) in palette 0x38, opaque when chosen
//!   and at 50 % otherwise. Unchosen pictures are drawn first, the chosen one last.

use anyhow::Result;
use xp_core::apt_index::AptName;
use xp_core::menu::Rect;

use crate::btnui::Place;
use crate::textures::TextureStore;
use crate::ui::{Layer, Ui};

/// Rows each side of the middle one (`BTN_list_and_search` with no height given).
const HALF_ROWS: i32 = 10;

/// The state of one airport list.
#[derive(Debug, Clone, Default)]
pub struct AptList {
    /// Index (into `shown`) of the middle row.
    pub scroll: i32,
    /// Index (into `shown`) of the selected airport.
    pub select: i32,
    /// The filtered airports, as indices into the index.
    pub shown: Vec<usize>,
    /// Select Airport: the selected airport's map caption and number of ramp starts.
    pub title: String,
    pub ramps: usize,
}

impl AptList {
    /// The list with `icao` selected and scrolled into view from the top, as the window opens.
    pub fn new(index: &[AptName], icao: &str) -> AptList {
        let shown = xp_core::apt_index::filter(index, "");
        let select = shown
            .iter()
            .position(|&i| index[i].icao == icao)
            .unwrap_or(0) as i32;
        let mut l = AptList {
            scroll: 0,
            select,
            shown,
            title: String::new(),
            ramps: 0,
        };
        l.into_view();
        l
    }

    fn max(&self) -> i32 {
        self.shown.len() as i32 - 1
    }

    /// Keep the selection on screen: the least scroll that shows it.
    pub fn into_view(&mut self) {
        if self.select > self.scroll + HALF_ROWS {
            self.scroll = self.select - HALF_ROWS;
        } else if self.select < self.scroll - HALF_ROWS {
            self.scroll = self.select + HALF_ROWS;
        }
        self.scroll = self.scroll.clamp(0, self.max().max(0));
    }

    /// A click at GL `gy` in a list centred at GL `y`: select the row under it.
    /// Returns whether the selection changed.
    pub fn click(&mut self, y: f32, gy: f32) -> bool {
        let k = ((gy - y) / 10.0).round() as i32;
        let i = self.scroll - k;
        if (0..=self.max()).contains(&i) && i != self.select {
            self.select = i;
            return true;
        }
        false
    }

    pub fn selected<'a>(&self, index: &'a [AptName]) -> Option<&'a AptName> {
        self.shown.get(self.select as usize).map(|&i| &index[i])
    }
}

fn rect(p: &Place, x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect {
        l: p.sx(x),
        t: p.sy(y + h),
        r: p.sx(x + w),
        b: p.sy(y),
    }
}

/// `BTN_vert_slider(x, y0, y1, …, min, max, &val)`.
#[allow(clippy::too_many_arguments)]
pub fn vert_slider(
    ui: &Ui,
    p: &Place,
    x: f32,
    y0: f32,
    y1: f32,
    val: i32,
    min: i32,
    max: i32,
) -> Vec<Layer> {
    let cx = p.sx(x);
    let mut l = ui.nine_slice(
        Rect {
            l: cx - 6.0,
            t: p.sy(y1 + 4.0),
            r: cx + 5.0,
            b: p.sy(y0 - 4.0),
        },
        [106.0, 228.0, 117.0, 260.0],
        [0.0, 3.0, 0.0, 3.0],
        [0.0; 4],
    );
    if max <= min {
        return l;
    }
    let (top, bottom) = (y1 - 10.0, y0 + 9.0);
    let t = top + (bottom - top) * (val.clamp(min, max) - min) as f32 / (max - min) as f32;
    l.push(ui.atlas_cell(
        [83.0, 224.0, 101.0, 246.0],
        [cx - 9.0, p.sy(y1 + 19.0), 18.0, 22.0],
    ));
    l.push(ui.atlas_cell(
        [83.0, 248.0, 101.0, 272.0],
        [cx - 9.0, p.sy(t + 12.0), 18.0, 24.0],
    ));
    l.push(ui.atlas_cell(
        [83.0, 274.0, 101.0, 296.0],
        [cx - 9.0, p.sy(y0 + 1.0), 18.0, 22.0],
    ));
    l
}

/// `BTN_list_and_search` at window GL (x, y) over the airport index.
#[allow(clippy::too_many_arguments)]
pub fn apt_list(
    ui: &Ui,
    d: &wgpu::Device,
    q: &wgpu::Queue,
    index: &[AptName],
    st: &AptList,
    p: &Place,
    x: f32,
    y: f32,
) -> Result<Vec<Layer>> {
    const W: f32 = 290.0;
    let mut l = ui.frame2(0x13, rect(p, x, y - 110.0, W, 220.0));
    l.extend(vert_slider(
        ui,
        p,
        x + W + 16.0,
        y - 110.0,
        y + 110.0,
        st.scroll,
        0,
        st.max(),
    ));
    let rgb = xp_core::menu::palette(0x36);
    for k in -HALF_ROWS..=HALF_ROWS {
        let i = st.scroll + k;
        let Some(&a) = st.shown.get(i as usize).filter(|_| i >= 0) else {
            continue;
        };
        let row = y - 10.0 * k as f32;
        let base = p.sy(row - 4.0);
        l.push(ui.text_at(d, q, &index[a].name, [p.sx(x + 20.0), base], rgb)?);
        l.push(ui.text_at(d, q, &index[a].icao, [p.sx(x + 242.0), base], rgb)?);
        if i == st.select {
            l.push(ui.palette_fill(rect(p, x + 18.0, row - 5.0, W - 22.0, 10.0), 0x60, 0.25));
        }
    }
    Ok(l)
}

const TIMES: [&str; 4] = ["day", "sunset", "twilight", "night"];
const WEATHERS: [&str; 8] = [
    "clear",
    "few",
    "scattered",
    "broken",
    "overcast",
    "low-vis",
    "foggy",
    "stormy",
];

/// One `qflt_butn`: picture `cell` of qflite.png at window GL (x, y).
#[allow(clippy::too_many_arguments)]
fn qflt_butn(
    ui: &Ui,
    d: &wgpu::Device,
    q: &wgpu::Queue,
    atlas: &wgpu::TextureView,
    p: &Place,
    x: f32,
    y: f32,
    cell: i32,
    label: &str,
    chosen: bool,
    active: bool,
) -> Result<Vec<Layer>> {
    let mut l = ui.frame2(0x12, rect(p, x - 3.0, y - 3.0, 96.0, 155.0));
    if chosen && active {
        l.extend(ui.frame2(0x15, rect(p, x + 5.0, y + 5.0, 80.0, 139.0)));
    }
    let (col, row) = ((cell % 4) as f32, (2 - cell / 4) as f32);
    let (u0, v0, v1) = (col * 101.0 + 1.0, row * 151.0 + 1.0, row * 151.0 + 151.0);
    l.push(Layer {
        view: atlas.clone(),
        rect: [p.sx(x), p.sy(y + 149.0), 90.0, 149.0],
        tint: [1.0; 4],
        src: Some([u0, 512.0 - v1, u0 + 90.0, 512.0 - v0]),
        linear: false,
    });
    let mut t = ui.text_centred(
        d,
        q,
        label,
        [p.sx(x + 45.0), p.sy(y + 5.0)],
        xp_core::menu::palette(0x38),
    )?;
    t.tint[3] = if chosen { 1.0 } else { 0.5 };
    l.push(t);
    Ok(l)
}

/// QuickFlight's time (prf `_qfl_tim`) and weather (`_qfl_wxr`) pictures.
#[allow(clippy::too_many_arguments)]
pub fn qflt_pictures(
    ui: &Ui,
    d: &wgpu::Device,
    q: &wgpu::Queue,
    store: &mut TextureStore,
    install: &std::path::Path,
    tim: i32,
    wxr: i32,
    p: &Place,
) -> Result<Vec<Layer>> {
    let atlas = store.get_ui(d, q, &install.join("Resources/bitmaps/qflite.png"))?;
    let mut l = Vec::new();
    for pass in [false, true] {
        for (i, name) in TIMES.iter().enumerate() {
            let i = i as i32;
            if (i == tim) == pass {
                l.extend(qflt_butn(
                    ui,
                    d,
                    q,
                    &atlas,
                    p,
                    616.0 + 100.0 * i as f32,
                    560.0,
                    i + 8,
                    name,
                    i == tim,
                    true,
                )?);
            }
        }
    }
    for pass in [false, true] {
        for (i, name) in WEATHERS.iter().enumerate() {
            let i = i as i32;
            if (i == wxr) == pass {
                let (x, y) = (
                    616.0 + 100.0 * (i % 4) as f32,
                    326.0 - 160.0 * (i / 4) as f32,
                );
                l.extend(qflt_butn(
                    ui,
                    d,
                    q,
                    &atlas,
                    p,
                    x,
                    y,
                    i,
                    name,
                    i == wxr,
                    true,
                )?);
            }
        }
    }
    Ok(l)
}

/// `opensave_perframe` in QuickFlight mode (file type 0x16): the aircraft tree in the field
/// (358, 495)…(568, 715) with its slider at x 583, and the preview tile in the lower-left
/// pane.
#[allow(clippy::too_many_arguments)]
pub fn qflt_aircraft(
    ui: &Ui,
    d: &wgpu::Device,
    q: &wgpu::Queue,
    store: &mut TextureStore,
    tree: &crate::chooser::Chooser,
    p: &Place,
) -> Result<Vec<Layer>> {
    let (x1, x2, y1, y2) = (358.0, 568.0, 495.0, 715.0);
    let top = p.sy(y2);
    // Rows 15 apart from 17 below the field's top, while the baseline stays 5 above its
    // bottom (the user's screenshot of the Open dialog and the oracle frame agree).
    let first = top + 17.0;
    let rows = ((p.sy(y1) - 5.0 - first) / 15.0).floor() as usize + 1;
    let mut l = tree.tree_layers(
        ui,
        d,
        q,
        [p.sx(x1), top, p.sx(x2)],
        [p.sx(x1 + 20.0), first],
        rows,
        0,
    )?;
    let max = tree.row_count().saturating_sub(rows) as i32;
    l.extend(vert_slider(ui, p, x2 + 15.0, y1, y2, 0, 0, max));
    // plot_a_tile at x = (10 + 598) / 2 = 304, y = 150 (opensave_perframe's preview pane).
    l.extend(tree.preview_layers(ui, d, q, store, [p.sx(304.0 - 125.0), p.sy(150.0 + 200.0)])?);
    Ok(l)
}

/// A click at window GL (gx, gy) on QuickFlight's own parts: the time and weather pictures
/// (prf `_qfl_tim`, `_qfl_wxr`, as `qflt_butn` sets them on a press inside the 90 × 149
/// picture) and the aircraft tree rows. Returns the prf key and value to set, if any.
pub fn qflt_click(
    tree: Option<&mut crate::chooser::Chooser>,
    gx: f32,
    gy: f32,
) -> Option<(&'static str, f64)> {
    let inside = |x: f32, y: f32| (x..=x + 90.0).contains(&gx) && (y..=y + 149.0).contains(&gy);
    for i in 0..4 {
        if inside(616.0 + 100.0 * i as f32, 560.0) {
            return Some(("_qfl_tim", i as f64));
        }
    }
    for i in 0..8 {
        if inside(
            616.0 + 100.0 * (i % 4) as f32,
            326.0 - 160.0 * (i / 4) as f32,
        ) {
            return Some(("_qfl_wxr", i as f64));
        }
    }
    if let Some(t) = tree {
        if (358.0..=568.0).contains(&gx) && (495.0..=715.0).contains(&gy) {
            // Rows from 17 below the top (baseline), 11.5 above the baseline to 3.5 below.
            let row = ((715.0 - gy) - (17.0 - 11.5)) / 15.0;
            if row >= 0.0 {
                t.click_row(row as usize);
            }
        }
    }
    None
}

/// A click at window GL (gx, gy) on an airport list centred at (x, y).
/// Returns whether the selection changed.
pub fn apt_click(list: &mut AptList, x: f32, y: f32, gx: f32, gy: f32) -> bool {
    (x..=x + 290.0).contains(&gx) && (y - 110.0..=y + 110.0).contains(&gy) && list.click(y, gy)
}

/// Select Airport's map caption (airp_wind: `OGL_string(341 + 10, 705 - 15, 0x38)`) and the
/// ramp starts' slider (`BTN_vert_slider` at x 625, two starts a row; its y0 / y1, 25 / 385,
/// are the oracle frame's: the call's float arguments are not recovered).
#[allow(clippy::too_many_arguments)]
pub fn airp_extras(
    ui: &Ui,
    d: &wgpu::Device,
    q: &wgpu::Queue,
    list: &AptList,
    p: &Place,
) -> Result<Vec<Layer>> {
    let mut l = Vec::new();
    if !list.title.is_empty() {
        l.push(ui.text_at(
            d,
            q,
            &list.title,
            [p.sx(351.0), p.sy(690.0)],
            xp_core::menu::palette(0x38),
        )?);
    }
    let rows = list.ramps.div_ceil(2) as i32;
    l.extend(vert_slider(ui, p, 625.0, 25.0, 385.0, 0, 0, rows - 1));
    Ok(l)
}
