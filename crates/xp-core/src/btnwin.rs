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
    /// `OGL_string`: text in palette colour b, wrapped at width a.
    Text,
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
    pub widgets: Vec<Widget>,
}

fn num(s: &str) -> f32 {
    s.trim().parse().unwrap_or(0.0)
}

pub fn parse(id: i32, title: &str, text: &str) -> Result<Window> {
    let mut widgets = Vec::new();
    for (n, line) in text.lines().enumerate() {
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
            other => bail!("window {id} line {}: unknown widget {other}", n + 1),
        };
        widgets.push(Widget {
            kind,
            x: num(f[1]),
            y: num(f[2]),
            a: num(f[3]),
            b: num(f[4]),
            c: num(f[5]),
            d: num(f[6]),
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
        size: (1024.0, 768.0),
        widgets,
    })
}

/// The windows the port has tables for, by `start_win` id.
pub fn load(id: i32, title: &str) -> Result<Option<Window>> {
    let text = match id {
        10 => include_str!("../data/windows/10.tsv"),
        _ => return Ok(None),
    };
    parse(id, title, text).map(Some)
}

/// Settings values by pref name.
#[derive(Debug, Clone, Default)]
pub struct Prefs {
    values: BTreeMap<String, f64>,
    own: Option<PathBuf>,
}

impl Prefs {
    /// `X-Plane.prf` lines are `name value`; the port's own file has the same form and wins.
    pub fn load(install: &Path, own: Option<PathBuf>) -> Prefs {
        let mut p = Prefs {
            values: BTreeMap::new(),
            own,
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
        if let Some(o) = &p.own {
            read(o, &mut p.values);
        }
        p
    }

    pub fn get(&self, key: &str) -> f64 {
        self.values.get(key).copied().unwrap_or(0.0)
    }

    /// Set a value and save the port's file.
    pub fn set(&mut self, key: &str, v: f64) -> Result<()> {
        self.values.insert(key.to_string(), v);
        if let Some(o) = &self.own {
            let mut text = String::new();
            for (k, v) in &self.values {
                text.push_str(&format!("{k} {v}\n"));
            }
            if let Some(d) = o.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            std::fs::write(o, text).with_context(|| format!("saving {}", o.display()))?;
        }
        Ok(())
    }

    /// Whether a widget's condition holds.
    pub fn shows(&self, cond: &str) -> bool {
        match cond.strip_prefix('!') {
            _ if cond.is_empty() => true,
            Some(k) => self.get(k) == 0.0,
            None => self.get(cond) != 0.0,
        }
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

/// Split a label at its `|`: text left of the control and text right of it.
pub fn sides(label: &str) -> (&str, &str) {
    match label.split_once('|') {
        Some((l, r)) => (l.trim(), r.trim()),
        None => (label.trim(), ""),
    }
}

/// The original sets a label in two lines, broken where the first line's width comes nearest
/// half the whole label's width (`measure` = the interface font's advance), at a space or after
/// a hyphen, the earlier break on a tie: "draw / volumetric fog", "number / of objects",
/// "3-D bump- / maps" (oracle frame of Rendering Options).
pub fn two_lines(text: &str, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let half = measure(text) / 2.0;
    let mut best: Option<(f32, usize, bool)> = None;
    for (i, c) in chars.iter().enumerate() {
        let (cut, keep) = match c {
            ' ' => (i, false),
            '-' if i + 1 < chars.len() && i > 0 => (i + 1, true),
            _ => continue,
        };
        let first: String = chars[..cut].iter().collect();
        let d = (measure(first.trim_end()) - half).abs();
        if best.is_none_or(|b| d < b.0) {
            best = Some((d, cut, keep));
        }
    }
    match best {
        Some((_, cut, keep)) => {
            let a: String = chars[..cut].iter().collect();
            let b: String = chars[if keep { cut } else { cut + 1 }..].iter().collect();
            vec![a.trim().to_string(), b.trim().to_string()]
        }
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
    fn rendering_options_table_parses() {
        let w = load(10, "Rendering Options")
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
