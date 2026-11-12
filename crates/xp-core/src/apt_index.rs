//! The airport list of Select Airport and QuickFlight (`airp_wind`'s `BTN_list_and_search`).
//!
//! `__vec_apt` holds one entry per ICAO: the scenery packs are read in `scenery_packs.ini`
//! order and the default apt.dat last, the first pack that defines an ICAO wins. The window
//! lists them sorted by name without regard to case (the oracle frame around KSEA — "Scrappin
//! Valley" … "Seattle Tacoma Intl" — is exactly that order over the user's 34 744 airports).
//! Land airports (row 1), seaplane bases (16) and heliports (17) are all listed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct AptName {
    pub icao: String,
    pub name: String,
}

/// The apt.dat files in the order the original loads them.
pub fn apt_files(install: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(ini) = crate::read_text_lossy(install.join("Custom Scenery/scenery_packs.ini")) {
        for l in ini.lines() {
            if let Some(p) = l.strip_prefix("SCENERY_PACK ") {
                let f = install.join(p.trim()).join("Earth nav data/apt.dat");
                if f.is_file() {
                    out.push(f);
                }
            }
        }
    }
    out.push(install.join("Resources/default scenery/default apt dat/Earth nav data/apt.dat"));
    out
}

/// Airport headers of one apt.dat text, in file order.
pub fn parse_headers(text: &str, out: &mut Vec<AptName>, seen: &mut HashSet<String>) {
    for l in text.lines() {
        let mut f = l.split_whitespace();
        if !matches!(f.next(), Some("1" | "16" | "17")) {
            continue;
        }
        // code, elevation, two deprecated fields, ICAO, then the name with its spacing kept.
        let (Some(_), Some(_), Some(_), Some(icao)) = (f.next(), f.next(), f.next(), f.next())
        else {
            continue;
        };
        let Some(at) = l.find(icao).map(|i| i + icao.len()) else {
            continue;
        };
        let name = l[at..].trim();
        if seen.insert(icao.to_string()) {
            out.push(AptName {
                icao: icao.to_string(),
                name: name.to_string(),
            });
        }
    }
}

/// Sort as the list shows them: by name, case-insensitive; a stable sort keeps load order
/// for equal names.
pub fn sort(list: &mut [AptName]) {
    list.sort_by_cached_key(|a| a.name.to_ascii_lowercase());
}

pub fn load(install: &Path) -> Vec<AptName> {
    let (mut out, mut seen) = (Vec::new(), HashSet::new());
    for f in apt_files(install) {
        if let Ok(t) = crate::read_text_lossy(&f) {
            parse_headers(&t, &mut out, &mut seen);
        }
    }
    sort(&mut out);
    out
}

/// `apt_refilter`: the entries whose name or ICAO contains `filter`, case-insensitive (all of
/// them for an empty filter).
pub fn filter(list: &[AptName], filter: &str) -> Vec<usize> {
    let f = filter.to_ascii_lowercase();
    (0..list.len())
        .filter(|&i| {
            f.is_empty()
                || list[i].name.to_ascii_lowercase().contains(&f)
                || list[i].icao.to_ascii_lowercase().contains(&f)
        })
        .collect()
}

/// `rway_to_cs(get_rway_strings(…, spaced = 1))`: "RWY " (or "PAD " for a helipad) and the
/// designator's characters a space apart, a trailing 'x' (no suffix) shown as a space.
pub fn runway_label(name: &str) -> String {
    let prefix = if name.starts_with('H') {
        "PAD "
    } else {
        "RWY "
    };
    let mut chars: Vec<char> = name.chars().collect();
    while chars.len() < 3 {
        chars.push('x');
    }
    let mut out = prefix.to_string();
    for (i, c) in chars.iter().take(3).enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push(if i == 2 && *c == 'x' { ' ' } else { *c });
    }
    out
}

/// The start locations Select Airport lists, in file order: `1300` (name after six fields)
/// and the older `15` (after four).
pub fn ramp_names(apt: &crate::aptdat::Airport) -> Vec<String> {
    apt.records
        .iter()
        .filter_map(|r| {
            let n = match r.kind {
                1300 => 6,
                15 => 4,
                _ => return None,
            };
            let mut rest = r.raw.trim_start();
            for _ in 0..n {
                let end = rest.find(char::is_whitespace)?;
                rest = rest[end..].trim_start();
            }
            Some(rest.trim_end().to_string())
        })
        .collect()
}

/// The map's caption: `name (ICAO) elev <feet> ft.` (airp_wind).
pub fn map_title(apt: &crate::aptdat::Airport) -> String {
    let h = &apt.header;
    let elev = h.field(1).unwrap_or("0");
    let icao = h.field(4).unwrap_or("");
    let name = h
        .raw
        .find(icao)
        .map(|i| h.raw[i + icao.len()..].trim())
        .unwrap_or("");
    format!("{name} ({icao}) elev {elev} ft.")
}

fn action(x: f32, y: f32, w: f32, key: String, label: String) -> crate::btnwin::Widget {
    crate::btnwin::Widget {
        kind: crate::btnwin::Kind::Action,
        x,
        y,
        a: w,
        b: 0.0,
        c: 0.0,
        d: 0.0,
        key,
        label,
        items: Vec::new(),
        cond: String::new(),
    }
}

/// Select Airport's start buttons (airp_wind, window 1; window GL coordinates):
/// per runway two rows 20 apart from y 391 down while the row is above y 20 — the end's
/// `RWY` button (x 50, 68 wide), "3 nm" (104, 52) and "10 nm" (150, 52) — and the ramp
/// starts two to a row at x 295 / 507 (210 wide), row r at 393 + 20·(scroll − r), shown while
/// 20 ≤ y ≤ 395.
pub fn airp_widgets(apt: &crate::aptdat::Airport, scroll: i32) -> Vec<crate::btnwin::Widget> {
    let mut out = Vec::new();
    let mut y = 391.0;
    for (i, r) in apt.runways.iter().enumerate() {
        let names = [r.end1_name().unwrap_or(""), r.end2_name().unwrap_or("")];
        for (e, name) in names.into_iter().enumerate() {
            if y <= 20.0 {
                return out;
            }
            out.push(action(
                50.0,
                y,
                68.0,
                format!("rwy:{i}:{e}"),
                runway_label(name),
            ));
            out.push(action(
                104.0,
                y,
                52.0,
                format!("rwy:{i}:{e}:3"),
                "3 nm".into(),
            ));
            out.push(action(
                150.0,
                y,
                52.0,
                format!("rwy:{i}:{e}:10"),
                "10 nm".into(),
            ));
            y -= 20.0;
        }
    }
    for (i, name) in ramp_names(apt).into_iter().enumerate() {
        let (col, row) = ((i % 2) as f32, (i / 2) as i32);
        let y = 393.0 + 20.0 * (scroll - row) as f32;
        if (20.0..=395.0).contains(&y) {
            out.push(action(
                295.0 + 212.0 * col,
                y,
                210.0,
                format!("ramp:{i}"),
                name,
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_first_wins_and_order() {
        let (mut v, mut s) = (Vec::new(), HashSet::new());
        parse_headers(
            "I\n1000 Version\n1    433 0 0 KSEA Seattle Tacoma Intl\n100 x\n\
             17 10 0 0 RP16 SEAHAWK LZ\n16 0 0 0 W55 Seattle  Seaplane\n",
            &mut v,
            &mut s,
        );
        parse_headers("1 0 0 0 KSEA Other\n", &mut v, &mut s);
        sort(&mut v);
        let n: Vec<&str> = v.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            n,
            ["SEAHAWK LZ", "Seattle  Seaplane", "Seattle Tacoma Intl"]
        );
        assert_eq!(filter(&v, "ksea"), vec![2]);
        assert_eq!(runway_label("16L"), "RWY 1 6 L");
        assert_eq!(runway_label("34"), "RWY 3 4  ");
    }
}
