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
}

impl Place {
    fn sx(&self, x: f32) -> f32 {
        self.ox + x
    }
    fn sy(&self, y: f32) -> f32 {
        self.oy + self.h - y
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
        Kind::Bool => Rect {
            l: x - 4.0,
            t: y - 12.0,
            r: x + 10.0,
            b: y + 2.0,
        },
        // As wide as the longest item + 28 (arrow box included), centred on x + 12: texture
        // resolution 368…456, number of cars 440…584 in the oracle frame.
        Kind::Popup => {
            let wid = w.items.iter().map(|s| ui.measure(s)).fold(0.0, f32::max) + 28.0;
            let l = (x + 12.0 - wid / 2.0).round();
            Rect {
                l,
                t: y - 12.0,
                r: l + wid,
                b: y + 3.0,
            }
        }
        Kind::Number => {
            let c = digit_centres(w, p);
            Rect {
                l: c[0] - 15.0,
                t: y - 12.0,
                r: c[c.len() - 1] + 10.0,
                b: y + 3.0,
            }
        }
        Kind::Radio => Rect {
            l: x - 8.0,
            t: y - 13.0,
            r: x + 10.0,
            b: y + 5.0,
        },
        Kind::Action => Rect {
            l: x - w.a / 2.0 + 6.0,
            t: y - 8.0,
            r: x + w.a / 2.0 - 3.0,
            b: y + 7.0,
        },
        Kind::Slider => Rect {
            l: x + 4.0,
            t: y - 11.0,
            r: x + w.a + 19.0,
            b: y + 5.0,
        },
        Kind::Subwin | Kind::Text => Rect {
            l: x,
            t: y,
            r: x,
            b: y,
        },
    }
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
                l.extend(ui.frame2(w.c as i32, rect));
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
                let i = prefs.widget_value(w).max(0.0) as usize;
                if let Some(item) = w.items.get(i).or(w.items.first()) {
                    l.push(ui.text_centred(
                        d,
                        q,
                        item,
                        [((r.l + r.r) * 0.5).floor() - 10.0, y - 1.0],
                        [255; 3],
                    )?);
                }
                label_lines(ui, d, q, left, r.l - 1.0, y, true, &mut l)?;
                label_lines(ui, d, q, right, r.r + 6.0, y, false, &mut l)?;
            }
            Kind::Number => {
                l.push(ui.fill([r.l, r.t, r.r - r.l, r.b - r.t], [255, 255, 255, 255]));
                let c = digit_centres(w, &p);
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
                label_lines(ui, d, q, left, r.l - 3.0, y, true, &mut l)?;
                label_lines(ui, d, q, right, r.r + 5.0, y, false, &mut l)?;
            }
            Kind::Radio => {
                let on = (prefs.get(&w.key) - w.a as f64).abs() < 0.5;
                let v = if on { 352.0 } else { 371.0 };
                l.push(ui.atlas_cell([83.0, v, 101.0, v + 18.0], [r.l, r.t, 18.0, 18.0]));
                label_lines(ui, d, q, right, x + 14.0, y, false, &mut l)?;
            }
            Kind::Action => {
                l.extend(ui.nine_slice(
                    Rect {
                        l: r.l - 4.0,
                        t: r.t - 5.0,
                        r: r.r + 4.0,
                        b: r.t + 21.0,
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
                label_lines(ui, d, q, right, x + w.a + 10.0, y, false, &mut l)?;
            }
            Kind::Text => {
                let rgb = xp_core::menu::palette(w.b as usize);
                let text = match w.key.as_str() {
                    "gl:vendor" => info.vendor.clone(),
                    "gl:renderer" => info.renderer.clone(),
                    "gl:version" => info.version.clone(),
                    "gl:texmem" => info.texmem.clone(),
                    _ => w.label.clone(),
                };
                for (i, line) in wrap(ui, &text, w.a).iter().enumerate() {
                    l.push(ui.text_at(d, q, line, [x + 2.0, y - 1.0 + 12.0 * i as f32], rgb)?);
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
    let r = control_rect(w, p, ui);
    let t = ((x - r.l - 5.0) / (w.a - 28.0)).clamp(0.0, 1.0);
    save(prefs, w, (w.b + t * (w.c - w.b)) as f64);
}

fn save(prefs: &mut Prefs, w: &Widget, v: f64) {
    if let Err(e) = prefs.set_widget_value(w, v) {
        log::warn!("{e:#}");
    }
}
