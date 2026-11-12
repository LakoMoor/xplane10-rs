//! Drawing and clicking the original's `BTN_*` widgets for a window described by
//! [`xp_core::btnwin`].
//!
//! Geometry is measured on the oracle frame of Rendering Options (`re_work/oracle/win_10.png`,
//! taken by opening the window in the original through `start_win(10)`): every offset below is
//! relative to the (x, y) the original passes to the widget call, with `sy` the call's y turned
//! into a screen row. Atlas cells were located with `re/findcell.py`.

use anyhow::Result;
use xp_core::btnwin::{sides, two_lines, Kind, Prefs, Widget, Window};
use xp_core::menu::Rect;

use crate::ui::{Layer, Ui};

/// Label and title colour of the settings windows (oracle frame).
const TEXT: [u8; 3] = [51, 59, 71];
/// One digit column of a number field.
const DIGIT: f32 = 10.0;

/// Facts the window shows that come from the running port (OpenGL strings in the original).
#[derive(Default, Clone)]
pub struct Info {
    pub vendor: String,
    pub renderer: String,
    pub version: String,
    pub texmem: String,
}

/// Interaction state of an open settings window.
#[derive(Default)]
pub struct State {
    /// The widget whose pop-up list is open.
    pub popup: Option<usize>,
    /// The slider being dragged.
    pub drag: Option<usize>,
}

/// Screen placement of a window: its top-left corner and height (GL y is measured from the
/// bottom).
#[derive(Clone, Copy)]
pub struct Place {
    pub ox: f32,
    pub oy: f32,
    pub h: f32,
    /// Extra rows down for the tab strip (full-screen windows, measured on the oracle frames).
    pub tab_dy: f32,
}

impl Place {
    pub(crate) fn sx(&self, x: f32) -> f32 {
        self.ox + x
    }
    pub(crate) fn sy(&self, y: f32) -> f32 {
        self.oy + self.h - y
    }
}

/// Date & Time read-outs (`xtim_wind`): local and zulu time, the date. The port's sim clock
/// starts at the computer's clock, as the original's does by default.
fn clock_text(key: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let off = local_offset_secs();
    let hm = |t: i64| {
        format!(
            "{:02}:{:02}",
            (t / 3600).rem_euclid(24),
            (t / 60).rem_euclid(60)
        )
    };
    match key {
        "clock:local" => format!("{} locl", hm(now + off)),
        "clock:zulu" => format!("{} zulu", hm(now)),
        _ => {
            let days = (now + off).div_euclid(86400);
            let (_, m, d) = civil(days);
            const M: [&str; 12] = [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ];
            format!("{} {:2}", M[(m - 1) as usize], d)
        }
    }
}

/// Days since 1970-01-01 → (year, month, day) (proleptic Gregorian).
fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// The computer's UTC offset, from `date +%z` (no libc time zone API in std).
fn local_offset_secs() -> i64 {
    static OFF: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| {
        std::process::Command::new("date")
            .arg("+%z")
            .output()
            .ok()
            .and_then(|o| {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                let sign = if s.starts_with('-') { -1 } else { 1 };
                let v: i64 = s.trim_start_matches(['+', '-']).parse().ok()?;
                Some(sign * ((v / 100) * 3600 + (v % 100) * 60))
            })
            .unwrap_or(0)
    })
}

/// BTN_action: a width ≤ 0 means the label's width + 21.
fn action_width(w: &Widget, ui: &Ui) -> f32 {
    if w.a <= 0.0 {
        ui.measure(&w.label) + 21.0
    } else {
        w.a
    }
}

fn number_text(w: &Widget, v: f64) -> Vec<char> {
    let (n, d) = (w.a.max(1.0) as usize, w.b.max(0.0) as usize);
    let s = format!(
        "{:0width$.prec$}",
        v.abs(),
        width = n + usize::from(d > 0),
        prec = d
    );
    let mut c: Vec<char> = s.chars().collect();
    if v < 0.0 {
        c.insert(0, '-');
    }
    c
}

/// Digit column centres of a number field: n digits, 10 px apart, from x + 8 (gamma's digits
/// and arrows in the oracle frame sit at 808 and 818 for x = 800).
fn digit_centres(w: &Widget, p: &Place) -> Vec<f32> {
    let n = w.a.max(1.0) as usize;
    (0..n).map(|i| p.sx(w.x) + 8.0 + DIGIT * i as f32).collect()
}

/// The control's own box, for clicks and for placing its labels.
fn control_rect(w: &Widget, p: &Place, ui: &Ui) -> Rect {
    let (x, y) = (p.sx(w.x), p.sy(w.y));
    match w.kind {
        // BTN_bool (0x1003d90b0): the box cell centred on x (x − 9 … x + 9 with its margin),
        // labels at x ∓ 12.
        Kind::Bool => Rect {
            l: x - 6.0,
            t: y - 12.0,
            r: x + 8.0,
            b: y + 2.0,
        },
        // As wide as the longest item + 28 (arrow box included), centred on x + 12: texture
        // resolution 368…456, number of cars 440…584 in the oracle frame.
        Kind::Popup => {
            // `a` = 1: the failure-window variant of BTN_textlist, longest item × 0.95 (min 20).
            let longest = w.items.iter().map(|s| ui.measure(s)).fold(0.0, f32::max);
            // OGL_measure_string sums unhinted advances, about 2 px over the port's hinted
            // measure for these items (oracle frame of System Failures).
            let wid = if w.a == 1.0 {
                (longest * 0.95).max(20.0) + 2.0
            } else {
                longest
            } + 28.0;
            let l = (x + 10.0 - wid / 2.0).round();
            Rect {
                l,
                t: y - 12.0,
                r: l + wid,
                b: y + 3.0,
            }
        }
        // BTN_flt_range (0x1003dcd20): BTN_frame1 type 9, 10·n + 15 wide, centred on
        // (x + 5·n − 1, y + 3); labels at x − 12 and past the box.
        Kind::Number => {
            let n = w.a.max(1.0);
            let l = x + 5.0 * n - 1.0 - (5.0 * n + 7.5).floor();
            Rect {
                l,
                t: y - 11.0,
                r: l + 10.0 * n + 15.0,
                b: y + 4.0,
            }
        }
        Kind::Radio => Rect {
            l: x - 8.0,
            t: y - 13.0,
            r: x + 10.0,
            b: y + 5.0,
        },
        Kind::Action => Rect {
            l: x - action_width(w, ui) / 2.0 + 6.0,
            t: y - 8.0,
            r: x + action_width(w, ui) / 2.0 - 3.0,
            b: y + 7.0,
        },
        Kind::Slider => Rect {
            l: x + 4.0,
            t: y - 11.0,
            r: x + w.a + 19.0,
            b: y + 5.0,
        },
        // Tabs: 86 wide, 88 apart from x 10, top 25 below the window's top (Instructions).
        Kind::Tab => {
            let l = p.ox + 10.0 + 88.0 * w.x;
            // BTN_tab's second argument is the row; rows are 32 apart (failure sub-tabs).
            let t = p.oy + 25.0 + p.tab_dy + 32.0 * w.y;
            Rect {
                l,
                t,
                r: l + 86.0,
                b: t + 20.0,
            }
        }
        Kind::Enter => Rect {
            l: x - 7.0,
            t: y - 12.0,
            r: x - 7.0 + w.a.max(40.0),
            b: y + 3.0,
        },
        // Calibration (oracle Aircraft & Situations frame): the field's sunken box covers
        // x − 7 … x + w + 2, 12 rows above y to 3 below.
        Kind::Path => Rect {
            l: x - 7.0,
            t: y - 12.0,
            r: x + w.a + 2.0,
            b: y + 3.0,
        },
        Kind::Field => Rect {
            l: x - (w.a * 0.5).floor(),
            t: y - 8.0,
            r: x - (w.a * 0.5).floor() + w.a,
            b: y + 7.0,
        },
        Kind::Subwin | Kind::Text | Kind::Display | Kind::Dir => Rect {
            l: x,
            t: y,
            r: x,
            b: y,
        },
    }
}

#[allow(clippy::too_many_arguments)]
/// `BTN_horz_slider(x, y, w, palette, min, max, *v, editable, label, value)` at screen x (left
/// end) and y: the track (`BTN_frame1` type 7, palette 0x48) centred on y over x … x + w; unless
/// the palette is 0x60, the fill (`BTN_frame1` type 8 tinted with the palette) over the first
/// pos = w·(v − min)/(max − min) columns rounded up to even, 4 rows up; the value string in
/// palette 0x34 centred at x + w/2, 15 up; the thumb at x + pos when editable.
/// items: ["horz", palette or the pref holding it, editable, format].
fn horz_slider(
    ui: &Ui,
    d: &wgpu::Device,
    q: &wgpu::Queue,
    w: &Widget,
    prefs: &Prefs,
    x: f32,
    y: f32,
) -> Result<Vec<Layer>> {
    let mut l = Vec::new();
    let v = prefs.widget_value(w) as f32;
    let pal = match w.items.get(1) {
        Some(s) if s.starts_with("m:") => prefs.get(s) as usize,
        Some(s) => s.parse().unwrap_or(0x49),
        None => 0x49,
    };
    let editable = w.items.get(2).map(String::as_str) == Some("1");
    let pos = if w.c > w.b {
        (w.a * (v - w.b) / (w.c - w.b)).clamp(0.0, w.a)
    } else {
        w.a * 0.5
    };
    // Track: atlas cell 240…305 × 246…270, 10-column ends, top = y − 12 − 1.
    l.extend(ui.nine_slice(
        Rect {
            l: x,
            t: y - 13.0,
            r: x + w.a,
            b: y + 11.0,
        },
        [240.0, 246.0, 305.0, 270.0],
        [10.0, 0.0, 10.0, 0.0],
        [0.0; 4],
    ));
    if pal != 0x60 && pos > 0.0 {
        let iw = 2.0 * ((pos.trunc() * 0.5 + 0.5).trunc());
        let c = xp_core::menu::palette(pal);
        let tint = [
            f32::from(c[0]) / 255.0,
            f32::from(c[1]) / 255.0,
            f32::from(c[2]) / 255.0,
            1.0,
        ];
        // Fill: atlas cell 289…305 × 277…293, 7-column ends, centred 4 rows above y.
        for mut layer in ui.nine_slice(
            Rect {
                l: x,
                t: y - 4.0 - 9.0,
                r: x + iw,
                b: y - 4.0 + 7.0,
            },
            [289.0, 277.0, 305.0, 293.0],
            [7.0, 0.0, 7.0, 0.0],
            [0.0; 4],
        ) {
            layer.tint = tint;
            l.push(layer);
        }
    }
    let text = match w.items.get(3).map(String::as_str) {
        // flt_to_str(v × 12 × 3.2808, 3, 1): inches from the default CG.
        Some("cg") => format!("{:.1}", v * 12.0 * 3.280_84),
        _ => format!("{}", v as i32),
    };
    l.push(ui.text_centred(
        d,
        q,
        &text,
        [x + w.a / 2.0, y - 16.0],
        xp_core::menu::palette(0x34),
    )?);
    if editable {
        let tx = (x + pos - 5.5).round();
        l.push(ui.atlas_cell([267.0, 225.0, 278.0, 241.0], [tx, y - 12.0, 11.0, 16.0]));
    }
    Ok(l)
}

fn label_lines(
    ui: &Ui,
    d: &wgpu::Device,
    q: &wgpu::Queue,
    text: &str,
    x: f32,
    y: f32,
    right: bool,
    out: &mut Vec<Layer>,
) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    let lines = two_lines(text, &|t| ui.measure(t));
    let base = if lines.len() == 1 { y } else { y - 5.0 };
    for (i, l) in lines.iter().enumerate() {
        let at = [x, base + 10.0 * i as f32];
        out.push(if right {
            ui.text_right(d, q, l, at, TEXT)?
        } else {
            ui.text_at(d, q, l, at, TEXT)?
        });
    }
    Ok(())
}

fn wrap(ui: &Ui, text: &str, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split(' ') {
        let cand = if cur.is_empty() {
            word.to_string()
        } else {
            format!("{cur} {word}")
        };
        if ui.measure(&cand) > width && !cur.is_empty() {
            lines.push(std::mem::replace(&mut cur, word.to_string()));
        } else {
            cur = cand;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Everything a window draws, back to front.
#[allow(clippy::too_many_arguments)]
pub fn layers(
    ui: &Ui,
    d: &wgpu::Device,
    q: &wgpu::Queue,
    win: &Window,
    prefs: &Prefs,
    info: &Info,
    state: &State,
    p: Place,
) -> Result<Vec<Layer>> {
    let mut l = Vec::new();
    for w in &win.widgets {
        if !prefs.shows(&w.cond) {
            continue;
        }
        let (x, y) = (p.sx(w.x), p.sy(w.y));
        let r = control_rect(w, &p, ui);
        let (left, right) = sides(&w.label);
        match w.kind {
            Kind::Subwin => {
                let rect = Rect {
                    l: x,
                    t: p.sy(w.y + w.b),
                    r: x + w.a,
                    b: y,
                };
                // Calibration (measured on every full-screen oracle frame): the panes sit one row
                // above the widgets' 2-row drop; their titles stay with the widgets.
                let up = if p.tab_dy > 0.0 { 1.0 } else { 0.0 };
                l.extend(ui.frame2(
                    w.c as i32,
                    Rect {
                        t: rect.t - up,
                        b: rect.b - up,
                        ..rect
                    },
                ));
                if !w.label.is_empty() {
                    l.push(ui.text_centred(
                        d,
                        q,
                        &w.label,
                        [((rect.l + rect.r) * 0.5).floor() - 1.0, rect.t + 17.0],
                        TEXT,
                    )?);
                }
            }
            Kind::Bool => {
                let v = if prefs.widget_value(w) != 0.0 {
                    298.0
                } else {
                    318.0
                };
                l.push(ui.atlas_cell([142.0, v, 156.0, v + 14.0], [r.l, r.t, 14.0, 14.0]));
                label_lines(ui, d, q, left, r.l - 6.0, y, true, &mut l)?;
                label_lines(ui, d, q, right, r.r + 4.0, y, false, &mut l)?;
            }
            Kind::Popup => {
                l.extend(ui.nine_slice(
                    Rect {
                        l: r.l - 5.0,
                        t: r.t - 6.0,
                        r: r.r + 4.0,
                        b: r.t + 20.0,
                    },
                    [357.0, 0.0, 425.0, 26.0],
                    [8.0, 0.0, 25.0, 0.0],
                    [0.0; 4],
                ));
                let v = prefs.widget_value(w);
                let i = v.max(0.0) as usize;
                // BTN_textlist in the failures window: green while working, red when
                // inoperative, amber for any timed or triggered failure.
                let rgb = if w.a == 1.0 {
                    xp_core::menu::palette(match v as i32 {
                        0 => 0x49,
                        6 => 0x69,
                        n if n < 0 => 0x2f,
                        _ => 0x51,
                    })
                } else {
                    [255; 3]
                };
                if let Some(item) = w.items.get(i).or(w.items.first()) {
                    l.push(ui.text_centred(
                        d,
                        q,
                        item,
                        [((r.l + r.r) * 0.5).floor() - 10.0, y - 1.0],
                        rgb,
                    )?);
                }
                let gap = if w.a == 1.0 { 4.0 } else { 1.0 };
                label_lines(ui, d, q, left, r.l - gap, y, true, &mut l)?;
                label_lines(ui, d, q, right, r.r + 6.0, y, false, &mut l)?;
            }
            Kind::Number => {
                l.extend(ui.nine_slice(
                    r,
                    [264.0, 358.0, 312.0, 373.0],
                    [4.0, 4.0, 4.0, 4.0],
                    [0.0; 4],
                ));
                let c = digit_centres(w, &p);
                // OGL_number: a comma after every three integer digits of the field, between
                // the digit columns (not taking one).
                let int_digits = (w.a - w.b.max(0.0)).max(0.0) as usize;
                for m in [3, 6, 9] {
                    if int_digits > m {
                        if let Some(cx) = c.get(int_digits - m - 1) {
                            l.push(ui.text_centred(d, q, ",", [cx + 5.0, y], [0; 3])?);
                        }
                    }
                }
                let text = number_text(w, prefs.widget_value(w));
                let mut col = 0;
                for ch in text {
                    if ch == '.' {
                        if col > 0 {
                            l.push(ui.text_centred(d, q, ".", [c[col - 1] + 5.0, y], [0; 3])?);
                        }
                        continue;
                    }
                    if let Some(cx) = c.get(col) {
                        l.push(ui.text_centred(d, q, &ch.to_string(), [*cx, y], [0; 3])?);
                    }
                    col += 1;
                }
                for cx in &c {
                    // Digit arrows: atlas 86…95 × 302…312 (up) and × 319…329 (down), matched on
                    // the oracle frame to 1.1/255 rms at (cx − 4, y − 23) and (cx − 4, y + 3).
                    l.push(
                        ui.atlas_cell([86.0, 302.0, 95.0, 312.0], [cx - 4.0, y - 23.0, 9.0, 10.0]),
                    );
                    l.push(
                        ui.atlas_cell([86.0, 319.0, 95.0, 329.0], [cx - 4.0, y + 3.0, 9.0, 10.0]),
                    );
                }
                label_lines(ui, d, q, left, r.l - 4.0, y, true, &mut l)?;
                label_lines(ui, d, q, right, r.r + 5.0, y, false, &mut l)?;
            }
            Kind::Radio => {
                let on = (prefs.get(&w.key) - w.a as f64).abs() < 0.5;
                let v = if on { 352.0 } else { 371.0 };
                l.push(ui.atlas_cell([83.0, v, 101.0, v + 18.0], [r.l, r.t, 18.0, 18.0]));
                label_lines(ui, d, q, right, x + 14.0, y, false, &mut l)?;
            }
            Kind::Action => {
                // BTN_action draws BTN_frame1(3, x, y, w, 1, 0x48): the cell over x − w/2 … x + w/2
                // (control_rect is x − w/2 + 6 … x + w/2 − 3).
                // Calibration (oracle frames, like the panes): one row higher in the full-screen
                // windows (Aircraft & Situations, Weight, Data Output), not in the fixed-size ones.
                let up = if p.tab_dy > 0.0 { 1.0 } else { 0.0 };
                l.extend(ui.nine_slice(
                    Rect {
                        l: r.l - 6.0,
                        t: r.t - 5.0 - up,
                        r: r.r + 3.0,
                        b: r.t + 21.0 - up,
                    },
                    [304.0, 0.0, 356.0, 26.0],
                    [8.0, 0.0, 8.0, 0.0],
                    [0.0; 4],
                ));
                l.push(ui.text_centred(
                    d,
                    q,
                    &w.label,
                    [((r.l + r.r) * 0.5).floor(), r.t + 11.0],
                    TEXT,
                )?);
            }
            Kind::Slider if w.items.first().map(String::as_str) == Some("horz") => {
                l.extend(horz_slider(ui, d, q, w, prefs, x, y)?);
                // BTN_util_draw_label at x − 8 and x + w + 8 (the constant 8.0 at 0x1010b3048).
                label_lines(ui, d, q, left, x - 8.0, y, true, &mut l)?;
                label_lines(ui, d, q, right, x + w.a + 8.0, y, false, &mut l)?;
            }
            Kind::Slider => {
                let v = prefs.widget_value(w) as f32;
                let t = ((v - w.b) / (w.c - w.b).max(1e-6)).clamp(0.0, 1.0);
                // The groove: seven rows with the oracle frame's profile, x + 4 … x + w − 2.
                for (k, c) in [86u8, 217, 133, 153, 180, 200, 118].into_iter().enumerate() {
                    l.push(ui.fill(
                        [x + 4.0, y - 8.0 + k as f32, w.a - 6.0, 1.0],
                        [c, c, c, 255],
                    ));
                }
                let tx = (x + 4.0 + t * (w.a - 28.0)).round();
                l.push(ui.atlas_cell([267.0, 225.0, 278.0, 241.0], [tx, y - 12.0, 11.0, 16.0]));
                if w.items.first().map(String::as_str) == Some("cloudpct") {
                    let cap = xp_core::btnwin::cloud_caption(v as f64);
                    l.push(ui.text_centred(d, q, &cap, [x + w.a / 2.0 + 1.0, y - 16.0], TEXT)?);
                }
                // BTN_horz_slider: labels at x − 8 (right-aligned) and x + w + 8.
                label_lines(ui, d, q, left, x - 10.0, y, true, &mut l)?;
                label_lines(ui, d, q, right, x + w.a + 10.0, y, false, &mut l)?;
            }
            Kind::Tab => {
                let sel = (prefs.get(&w.key) - w.a as f64).abs() < 0.5;
                // The bar under the tabs, once per tab row: `BTN_fillwin(…, 2)` draws
                // `BTN_frame1(W/2, H − 50, W, 1, 2)`, the atlas cell 102…202 × 0…26 (green bar in
                // rows 4…10, its shadow in 11…16) with 20-column ends.
                if w.x == 0.0 {
                    let t = p.oy + 38.0 + p.tab_dy + 32.0 * w.y;
                    l.extend(ui.nine_slice(
                        Rect {
                            l: p.ox,
                            t,
                            r: p.ox + win.size.0,
                            b: t + 26.0,
                        },
                        [102.0, 0.0, 202.0, 26.0],
                        [20.0, 0.0, 20.0, 0.0],
                        [0.0; 4],
                    ));
                }
                // The cell's opaque part (210…296 × 3…24) covers the tab's 86 columns.
                let cell = Rect {
                    l: r.l - 7.0,
                    t: r.t - 3.0,
                    r: r.r + 7.0,
                    b: r.t + 23.0,
                };
                // Selected: the green cell 203…303 × 0…26; unselected: the grey outline cell
                // right below it (× 27…53).
                let v = if sel { 0.0 } else { 27.0 };
                let q9 = ui.nine_slice(
                    cell,
                    [203.0, v, 303.0, v + 26.0],
                    [8.0, 0.0, 8.0, 0.0],
                    [0.0; 4],
                );
                l.extend(q9);
                let rgb = if sel { [0; 3] } else { [96, 96, 96] };
                l.push(ui.text_at(d, q, &w.label, [r.l + 3.0, r.t + 14.0], rgb)?);
            }
            Kind::Enter => {
                l.push(ui.fill([r.l, r.t, r.r - r.l, r.b - r.t], [255, 255, 255, 255]));
                l.push(ui.fill([r.l, r.t, r.r - r.l, 1.0], [119, 119, 119, 255]));
                label_lines(ui, d, q, left, r.l - 3.0, y, true, &mut l)?;
                label_lines(ui, d, q, right, r.r + 5.0, y, false, &mut l)?;
            }
            Kind::Path => {
                // The browse box: OGL_2dquad of the 17×17 atlas cell at (212, 357), from x − 26.
                l.push(ui.atlas_cell([212.0, 357.0, 229.0, 374.0], [x - 26.0, r.t - 1.0, 17.0, 17.0]));
                l.extend(ui.nine_slice(
                    r,
                    [264.0, 358.0, 312.0, 373.0],
                    [4.0, 4.0, 4.0, 4.0],
                    [0.0; 4],
                ));
                // BTN_select_path_stl_common shows the part after the last directory separator.
                let path = prefs.text(&w.key).unwrap_or("");
                let name = path.rsplit(['/', '\\', ':']).next().unwrap_or(path);
                l.push(ui.text_at(d, q, name, [x, y], TEXT)?);
                // BTN_util_draw_label at x − 32 and x + w + 6.
                label_lines(ui, d, q, left, x - 32.0, y, true, &mut l)?;
                label_lines(ui, d, q, right, x + w.a + 6.0, y, false, &mut l)?;
            }
            Kind::Dir => {
                // plot_dir: the needle (atlas 156…166 × 155…197) over x ± 5, y ± 21 turned by the
                // heading, then the glass ball (178…234 × 294…352) over x ± 28, y ± 29, then the
                // heading in 3 digits, palette 0x36, from x − 14 at GL y − 3.
                // Not yet: the needle's rotation (drawn upright, right for heading 0).
                l.push(ui.atlas_cell([156.0, 155.0, 166.0, 197.0], [x - 5.0, y - 21.0, 10.0, 42.0]));
                l.push(ui.atlas_cell([178.0, 294.0, 234.0, 352.0], [x - 28.0, y - 29.0, 56.0, 58.0]));
                let v = prefs.get(&w.key).rem_euclid(360.0).round() as i32 % 360;
                let rgb = xp_core::menu::palette(0x36);
                // OGL_number sets digits 10 apart: centres x − 11, x − 1, x + 9 (oracle frame).
                for (i, ch) in format!("{v:03}").chars().enumerate() {
                    let c = [x - 11.0 + 10.0 * i as f32, y + 2.0];
                    l.push(ui.text_centred(d, q, &ch.to_string(), c, rgb)?);
                }
            }
            Kind::Field => {
                // interface.png 264..312 × 358..373: the sunken white read-out box.
                l.extend(ui.nine_slice(
                    r,
                    [264.0, 358.0, 312.0, 373.0],
                    [4.0, 4.0, 4.0, 4.0],
                    [0.0; 4],
                ));
            }
            Kind::Display => {
                let text = format!("{:.*}", w.c.max(0.0) as usize, w.a);
                l.push(ui.text_at(d, q, &text, [x, y], TEXT)?);
                label_lines(ui, d, q, left, x - 6.0, y, true, &mut l)?;
            }
            Kind::Text => {
                let rgb = xp_core::menu::palette(w.b as usize);
                let text = match w.key.as_str() {
                    "gl:vendor" => info.vendor.clone(),
                    "gl:renderer" => info.renderer.clone(),
                    "gl:version" => info.version.clone(),
                    "gl:texmem" => info.texmem.clone(),
                    k if k.starts_with("clock:") => clock_text(k),
                    _ => w.label.clone(),
                };
                // Net Connections: the addresses one per 10 rows, the idle line 20 below the
                // last of them (inet_wind).
                if w.key == "net:ips" {
                    for (i, ip) in crate::netinfo::local_addresses().iter().enumerate() {
                        let at = [x + 2.0, y - 1.0 + 10.0 * i as f32];
                        l.push(ui.text_at(d, q, &ip.to_string(), at, rgb)?);
                    }
                    continue;
                }
                let y = if w.key == "net:idle" {
                    let n = crate::netinfo::local_addresses().len().max(1) as f32;
                    y + 10.0 * (n - 1.0)
                } else {
                    y
                };
                let text = if w.key == "net:host" {
                    crate::netinfo::computer_name().to_string()
                } else {
                    text
                };
                // Paragraphs (`¶`) are separated by a blank line, as in Instructions.
                let mut row = 0.0;
                for para in text.split('¶') {
                    for line in wrap(ui, para, w.a) {
                        // OGL_string's justification: 0 left, 1 centred, 2 right of x.
                        let lx = match w.c as i32 {
                            1 => (x - ui.measure(&line) * 0.5).floor(),
                            2 => x + 2.0 - ui.measure(&line),
                            _ => x + 2.0,
                        };
                        l.push(ui.text_at(d, q, &line, [lx, y - 1.0 + 12.0 * row], rgb)?);
                        row += 1.0;
                    }
                    row += 1.0;
                }
            }
        }
    }
    // An open pop-up list: the menus' drop-down frame under the button, one item per row.
    if let Some(i) = state.popup {
        if let Some(w) = win.widgets.get(i) {
            let r = control_rect(w, &p, ui);
            let rows = popup_rows(w, &r, ui);
            if let (Some(first), Some(last)) = (rows.first(), rows.last()) {
                let frame = Rect {
                    l: first.1.l,
                    t: first.1.t - 2.0,
                    r: first.1.r,
                    b: last.1.b + 2.0,
                };
                l.extend(ui.nine_slice(
                    frame,
                    [71.0, 156.0, 111.0, 195.0],
                    [11.0, 6.0, 11.0, 20.0],
                    [7.0, 1.0, 7.0, 11.0],
                ));
                let cur = prefs.widget_value(w) as usize;
                for (k, rr) in &rows {
                    if *k == cur {
                        l.push(ui.palette_fill(*rr, 0x2d, 0.25));
                    }
                    l.push(ui.text_at(
                        d,
                        q,
                        &w.items[*k],
                        [rr.l + 18.0, rr.b - 2.0],
                        xp_core::menu::palette(0x2f),
                    )?);
                }
            }
        }
    }
    Ok(l)
}

fn popup_rows(w: &Widget, r: &Rect, ui: &Ui) -> Vec<(usize, Rect)> {
    let width = w
        .items
        .iter()
        .map(|s| ui.measure(s))
        .fold(r.r - r.l, f32::max)
        + 30.0;
    (0..w.items.len())
        .map(|k| {
            let t = r.b + 2.0 + 12.0 * k as f32;
            (
                k,
                Rect {
                    l: r.l,
                    t,
                    r: r.l + width,
                    b: t + 12.0,
                },
            )
        })
        .collect()
}

/// A click inside the window. Returns true when something changed and the window must redraw.
#[allow(clippy::too_many_arguments)]
pub fn click(
    win: &Window,
    prefs: &mut Prefs,
    presets: &[Vec<(String, f64)>],
    state: &mut State,
    p: Place,
    ui: &Ui,
    x: f32,
    y: f32,
) -> bool {
    if let Some(i) = state.popup.take() {
        if let Some(w) = win.widgets.get(i) {
            let r = control_rect(w, &p, ui);
            if let Some((k, _)) = popup_rows(w, &r, ui)
                .into_iter()
                .find(|(_, rr)| rr.contains(x, y))
            {
                save(prefs, w, k as f64);
            }
        }
        return true;
    }
    for (i, w) in win.widgets.iter().enumerate() {
        if !prefs.shows(&w.cond) || w.key.is_empty() && w.kind != Kind::Action {
            continue;
        }
        let r = control_rect(w, &p, ui);
        match w.kind {
            Kind::Bool if r.contains(x, y) => {
                let v = if prefs.widget_value(w) != 0.0 {
                    0.0
                } else {
                    1.0
                };
                save(prefs, w, v);
                return true;
            }
            Kind::Popup if r.contains(x, y) => {
                state.popup = Some(i);
                return true;
            }
            Kind::Tab if r.contains(x, y) => {
                save(prefs, w, w.a as f64);
                return true;
            }
            Kind::Radio if r.contains(x, y) => {
                save(prefs, w, w.a as f64);
                return true;
            }
            Kind::Number => {
                let n = w.a.max(1.0) as i32;
                let dec = w.b.max(0.0) as i32;
                for (k, cx) in digit_centres(w, &p).iter().enumerate() {
                    let step = 10f64.powi(n - dec - 1 - k as i32);
                    let up = Rect {
                        l: cx - 5.0,
                        t: r.t - 12.0,
                        r: cx + 5.0,
                        b: r.t,
                    };
                    let down = Rect {
                        l: cx - 5.0,
                        t: r.b,
                        r: cx + 5.0,
                        b: r.b + 12.0,
                    };
                    let delta = if up.contains(x, y) {
                        step
                    } else if down.contains(x, y) {
                        -step
                    } else {
                        continue;
                    };
                    let v = (prefs.widget_value(w) + delta).clamp(w.c as f64, w.d as f64);
                    save(prefs, w, v);
                    return true;
                }
            }
            Kind::Action if r.contains(x, y) => {
                if let Some(level) = [
                    "Set to minimum",
                    "Set to low",
                    "Set to medium",
                    "Set to high",
                    "Set to extreme",
                ]
                .iter()
                .position(|s| *s == w.label)
                {
                    for (k, v) in presets.get(level).into_iter().flatten() {
                        if let Err(e) = prefs.set(k, *v) {
                            log::warn!("{e:#}");
                        }
                    }
                }
                return true;
            }
            Kind::Slider if r.contains(x, y) => {
                state.drag = Some(i);
                slide(w, prefs, &p, ui, x);
                return true;
            }
            _ => {}
        }
    }
    false
}

/// Dragging a slider to pointer x.
pub fn drag(win: &Window, prefs: &mut Prefs, state: &State, p: Place, ui: &Ui, x: f32) -> bool {
    match state.drag.and_then(|i| win.widgets.get(i)) {
        Some(w) => {
            slide(w, prefs, &p, ui, x);
            true
        }
        None => false,
    }
}

fn slide(w: &Widget, prefs: &mut Prefs, p: &Place, ui: &Ui, x: f32) {
    if w.items.first().map(String::as_str) == Some("horz") {
        // BTN_horz_slider: the value under the pointer, x … x + w spanning min … max; read-only
        // rows do not move.
        if w.items.get(2).map(String::as_str) != Some("1") {
            return;
        }
        let t = ((x - p.sx(w.x)) / w.a).clamp(0.0, 1.0);
        save(prefs, w, (w.b + t * (w.c - w.b)) as f64);
        if w.key.starts_with("m:wgt_") {
            let cell = std::cell::RefCell::new(prefs);
            let get = |k: &str| cell.borrow().get(k);
            let mut set = |k: &str, v: f64| {
                let _ = cell.borrow_mut().set(k, v);
            };
            xp_core::weights::sync(&get, &mut set, &w.key);
        }
        return;
    }
    let r = control_rect(w, p, ui);
    let t = ((x - r.l - 5.0) / (w.a - 28.0)).clamp(0.0, 1.0);
    save(prefs, w, (w.b + t * (w.c - w.b)) as f64);
}

fn save(prefs: &mut Prefs, w: &Widget, v: f64) {
    if let Err(e) = prefs.set_widget_value(w, v) {
        log::warn!("{e:#}");
    }
}
