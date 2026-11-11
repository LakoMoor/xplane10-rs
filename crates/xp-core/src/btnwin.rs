//! The original's settings windows as data: each window is the list of `BTN_*` widget calls its
//! `*_wind()` function makes, extracted from the binary by `re/btnemu.py` (an x86 walk of the
//! function that records every call with its concrete arguments) into
//! `data/windows/<start_win id>.tsv`. Positions are the original's own numbers, in GL window
//! coordinates (origin bottom-left).
//!
//! Values live in [`Prefs`]: the user's `Output/preferences/X-Plane.prf` read once as the
//! starting state, and changes kept in the port's own file — the port never writes into the
//! X-Plane installation.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `BTN_subwin`: a titled pane. a, b = width, height; c = frame type.
    Subwin,
    /// `BTN_bool`: a check box.
    Bool,
    /// `BTN_textlist`: a pop-up button; items `-`-separated.
    Popup,
    /// `BTN_flt_range` / `BTN_dob_range`: a number with one up/down arrow pair per digit.
    /// a = digits, b = decimals, c..d = range.
    Number,
    /// `BTN_is_int`: a radio button setting `key` to a.
    Radio,
    /// `BTN_action`: a push button, a = width.
    Action,
    /// `BTN_horz_slider`: a = width, b..c = range.
    Slider,
    /// `OGL_string`: text in palette colour b, wrapped at width a; `¶` separates paragraphs.
    Text,
    /// `BTN_tab`: tab x = index, a = value it sets `key` to.
    Tab,
    /// `BTN_enterstring`: a text field a wide.
    Enter,
    /// `BTN_display_number`: a read-only number.
    Display,
    /// `BTN_frame1` type 9: a sunken white read-out box a wide, centred on x, y.
    Field,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Widget {
    pub kind: Kind,
    pub x: f32,
    pub y: f32,
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub key: String,
    pub label: String,
    pub items: Vec<String>,
    /// Shown only while this pref is non-zero (`!` = while zero).
    pub cond: String,
}

/// One window: its title and widgets in drawing order.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub id: i32,
    pub title: String,
    pub size: (f32, f32),
    /// Rows above the even vertical split the original puts the window at (`# top`): −1 for
    /// most, 0 for Rendering Options, Planet Map and Select Global Airport (pinstripe phase of
    /// the oracle frames).
    pub top: f32,
    pub widgets: Vec<Widget>,
}

/// A table number: `base` or `base~kw,kh` = base + kw·(W − 1024) + kh·(H − 768) for windows
/// that fill the screen (W×H).
fn num(s: &str, screen: (f32, f32)) -> f32 {
    let s = s.trim();
    match s.split_once('~') {
        None => s.parse().unwrap_or(0.0),
        Some((base, k)) => {
            let base: f32 = base.parse().unwrap_or(0.0);
            let (kw, kh) = k.split_once(',').unwrap_or((k, "0"));
            base + kw.parse::<f32>().unwrap_or(0.0) * (screen.0 - 1024.0)
                + kh.parse::<f32>().unwrap_or(0.0) * (screen.1 - 768.0)
        }
    }
}

/// `screen`: the size a full-screen window takes (`# size screen`); fixed sizes come from
/// `wins_class::start_win`.
pub fn parse(id: i32, title: &str, text: &str, screen: (f32, f32)) -> Result<Window> {
    let mut widgets = Vec::new();
    let mut size = screen;
    let mut top = -1.0;
    for (n, line) in text.lines().enumerate() {
        if let Some(rest) = line.strip_prefix("# top ") {
            top = rest.trim().parse().unwrap_or(-1.0);
            continue;
        }
        if let Some(rest) = line.strip_prefix("# size ") {
            if rest.trim() == "screen" {
                size = screen;
                continue;
            }
            let v: Vec<f32> = rest
                .split_whitespace()
                .filter_map(|t| t.parse().ok())
                .collect();
            if v.len() == 2 {
                size = (v[0], v[1]);
            }
            continue;
        }
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 10 {
            bail!("window {id} line {}: {} fields", n + 1, f.len());
        }
        let kind = match f[0] {
            "subwin" => Kind::Subwin,
            "bool" => Kind::Bool,
            "popup" => Kind::Popup,
            "flt" | "dob" | "int" => Kind::Number,
            "radio" => Kind::Radio,
            "action" => Kind::Action,
            "slider" => Kind::Slider,
            "text" => Kind::Text,
            "tab" => Kind::Tab,
            "enter" => Kind::Enter,
            "display" => Kind::Display,
            "field" => Kind::Field,
            other => bail!("window {id} line {}: unknown widget {other}", n + 1),
        };
        widgets.push(Widget {
            kind,
            x: num(f[1], screen),
            y: num(f[2], screen),
            a: num(f[3], screen),
            b: num(f[4], screen),
            c: num(f[5], screen),
            d: num(f[6], screen),
            key: f[7].to_string(),
            label: f[8].to_string(),
            items: if f[9].is_empty() {
                Vec::new()
            } else {
                f[9].split('-').map(str::to_string).collect()
            },
            cond: f.get(10).map(|s| s.to_string()).unwrap_or_default(),
        });
    }
    Ok(Window {
        id,
        title: title.to_string(),
        size,
        top,
        widgets,
    })
}

/// The windows the port has tables for, by `start_win` id.
pub fn load(id: i32, title: &str, screen: (f32, f32)) -> Result<Option<Window>> {
    let text = match id {
        1 => include_str!("../data/windows/1.tsv"),
        2 => include_str!("../data/windows/2.tsv"),
        3 => include_str!("../data/windows/3.tsv"),
        4 => include_str!("../data/windows/4.tsv"),
        5 => include_str!("../data/windows/5.tsv"),
        6 => include_str!("../data/windows/6.tsv"),
        7 => include_str!("../data/windows/7.tsv"),
        8 => include_str!("../data/windows/8.tsv"),
        9 => include_str!("../data/windows/9.tsv"),
        10 => include_str!("../data/windows/10.tsv"),
        11 => include_str!("../data/windows/11.tsv"),
        12 => include_str!("../data/windows/12.tsv"),
        13 => include_str!("../data/windows/13.tsv"),
        14 => include_str!("../data/windows/14.tsv"),
        15 => include_str!("../data/windows/15.tsv"),
        16 => include_str!("../data/windows/16.tsv"),
        17 => include_str!("../data/windows/17.tsv"),
        19 => include_str!("../data/windows/19.tsv"),
        20 => include_str!("../data/windows/20.tsv"),
        22 => include_str!("../data/windows/22.tsv"),
        _ => return Ok(None),
    };
    parse(id, title, text, screen).map(Some)
}

/// Settings values by pref name.
#[derive(Debug, Clone, Default)]
pub struct Prefs {
    values: BTreeMap<String, f64>,
    own: Option<PathBuf>,
    /// Keys the port's own file holds: loaded from it or changed in a window.
    saved: std::collections::BTreeSet<String>,
}

/// Window state the original keeps in function statics or object fields for the session only
/// (tabs, unnamed controls): never written to the settings file.
fn session_only(key: &str) -> bool {
    key.contains("::")
        || key.starts_with("this+")
        || key.starts_with("lbl:")
        || key.starts_with("m:")
}

impl Prefs {
    /// `X-Plane.prf` lines are `name value`; the port's own file has the same form and wins.
    pub fn load(install: &Path, own: Option<PathBuf>) -> Prefs {
        let mut p = Prefs {
            values: BTreeMap::new(),
            own,
            saved: Default::default(),
        };
        let read = |path: &Path, v: &mut BTreeMap<String, f64>| {
            if let Ok(t) = crate::read_text_lossy(path) {
                for l in t.lines() {
                    let mut it = l.split_whitespace();
                    if let (Some(k), Some(val)) = (it.next(), it.next()) {
                        if let Ok(x) = val.parse::<f64>() {
                            v.insert(k.to_string(), x);
                        }
                    }
                }
            }
        };
        read(
            &install.join("Output/preferences/X-Plane.prf"),
            &mut p.values,
        );
        if let Some(o) = p.own.clone() {
            let mut mine = BTreeMap::new();
            read(&o, &mut mine);
            for (k, v) in mine {
                // Earlier builds copied all of X-Plane.prf here: keep only real changes.
                if !session_only(&k) && p.values.get(&k) != Some(&v) {
                    p.saved.insert(k.clone());
                    p.values.insert(k, v);
                }
            }
        }
        p
    }

    pub fn get(&self, key: &str) -> f64 {
        self.values.get(key).copied().unwrap_or(0.0)
    }

    /// Set a value and save the port's file.
    pub fn set(&mut self, key: &str, v: f64) -> Result<()> {
        self.values.insert(key.to_string(), v);
        if session_only(key) {
            return Ok(());
        }
        self.saved.insert(key.to_string());
        if let Some(o) = &self.own {
            let mut text = String::new();
            for k in &self.saved {
                text.push_str(&format!("{k} {}\n", self.values[k]));
            }
            if let Some(d) = o.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            std::fs::write(o, text).with_context(|| format!("saving {}", o.display()))?;
        }
        Ok(())
    }

    /// Whether a widget's condition holds.
    /// `key`, `!key` or `key=v1/v2/…`; several joined by `,` must all hold.
    pub fn shows(&self, cond: &str) -> bool {
        cond.split(',').filter(|c| !c.is_empty()).all(|c| {
            if let Some((k, vs)) = c.split_once('=') {
                let cur = self.get(k);
                return vs
                    .split('/')
                    .any(|v| (cur - v.parse::<f64>().unwrap_or(0.0)).abs() < 0.5);
            }
            match c.strip_prefix('!') {
                Some(k) => self.get(k) == 0.0,
                None => self.get(c) != 0.0,
            }
        })
    }

    /// The value a widget shows: the anisotropic filter is stored as the multiplier (1…16), the
    /// pop-up shows its log2.
    pub fn widget_value(&self, w: &Widget) -> f64 {
        let v = self.get(&w.key);
        if w.key == "_aniso_filter" {
            return v.max(1.0).log2().round();
        }
        v
    }

    pub fn set_widget_value(&mut self, w: &Widget, v: f64) -> Result<()> {
        if w.key == "_aniso_filter" {
            self.set(&w.key, 2f64.powf(v))
        } else {
            self.set(&w.key, v)
        }
    }
}

/// Split a label at its `|`: text left of the control and text right of it. A side that is
/// just `-` is empty; a side starting with `-` is set on one line (`-|-Show mouse click-regions`
/// in Instructions).
pub fn sides(label: &str) -> (&str, &str) {
    let (l, r) = match label.split_once('|') {
        Some((l, r)) => (l.trim(), r.trim()),
        None => (label.trim(), ""),
    };
    (if l == "-" { "" } else { l }, if r == "-" { "" } else { r })
}

/// The original sets a label in two lines, broken where the first line's width comes nearest
/// half the whole label's width (`measure` = the interface font's advance), at a space or after
/// a hyphen, the earlier break on a tie: "draw / volumetric fog", "number / of objects",
/// "3-D bump- / maps" (oracle frame of Rendering Options).
pub fn two_lines(text: &str, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    // BTN_util_draw_label (0x1003d8a30): a leading '-' keeps one line; otherwise labels longer
    // than 12 characters break at the space, '-' or '/' that makes the two lines' widths closest
    // (a space is dropped, '-' and '/' stay on the first line).
    if let Some(one) = text.strip_prefix('-') {
        return vec![one.to_string()];
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= 12 {
        return vec![text.to_string()];
    }
    let mut best: Option<(f32, String, String)> = None;
    for i in 0..chars.len() - 1 {
        let cut = match chars[i] {
            ' ' => i,
            '-' | '/' => i + 1,
            _ => continue,
        };
        let a: String = chars[..cut].iter().collect();
        let b: String = chars[i + 1..].iter().collect();
        let d = (measure(&a) - measure(&b)).abs().trunc();
        if best.as_ref().is_none_or(|x| d < x.0) {
            best = Some((d, a, b));
        }
    }
    match best {
        Some((_, a, b)) => vec![a, b],
        None => vec![text.to_string()],
    }
}

/// The cloud-puff slider's caption (reno_wind): `clamp(140·v + 10, 10, 150)` percent and a word
/// for the band v falls in.
pub fn cloud_caption(v: f64) -> String {
    let pct = (140.0 * v as f32 + 10.0).clamp(10.0, 150.0) as i32;
    let word = if v > 0.8 {
        "too many!"
    } else if v > 0.6 {
        "a lot."
    } else if v > 0.4 {
        "a good number."
    } else if v > 0.2 {
        "some."
    } else {
        "not many!"
    };
    format!("{pct}%: {word}")
}

/// `PRESET n` / `PRESET_SETTING name value` blocks of `Resources/settings.txt`: what the five
/// preset buttons of Rendering Options set (`REN_set_for_preset`).
pub fn presets(settings_txt: &str) -> Vec<Vec<(String, f64)>> {
    let mut out: Vec<Vec<(String, f64)>> = Vec::new();
    for l in settings_txt.lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        match f.as_slice() {
            ["PRESET", n] => {
                let n: usize = n.parse().unwrap_or(out.len());
                out.resize(out.len().max(n + 1), Vec::new());
            }
            ["PRESET_SETTING", k, v, ..] => {
                if let (Some(last), Ok(v)) = (out.last_mut(), v.parse()) {
                    last.push((k.to_string(), v));
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_break_like_the_original() {
        let m = |s: &str| s.chars().count() as f32;
        assert_eq!(
            two_lines("draw Aurora Borealis", &m),
            ["draw Aurora", "Borealis"]
        );
        assert_eq!(two_lines("gamma", &m), ["gamma"]);
        assert_eq!(cloud_caption(0.25), "45%: some.");
    }

    #[test]
    fn screen_relative_numbers() {
        assert_eq!(num("690~0,1", (1024.0, 868.0)), 790.0);
        assert_eq!(num("837~0.5,0", (1224.0, 768.0)), 937.0);
        assert_eq!(num("12", (2000.0, 2000.0)), 12.0);
    }

    #[test]
    fn every_window_table_parses() {
        for id in 1..=22 {
            load(id, "w", (1280.0, 800.0)).unwrap_or_else(|e| panic!("window {id}: {e:#}"));
        }
    }

    #[test]
    fn rendering_options_table_parses() {
        let w = load(10, "Rendering Options", (1024.0, 768.0))
            .expect("parses")
            .expect("table");
        assert_eq!(
            w.widgets.iter().filter(|w| w.kind == Kind::Subwin).count(),
            7
        );
        assert!(w
            .widgets
            .iter()
            .any(|w| w.key == "renopt_for_05" && w.items.len() == 6));
    }
}
