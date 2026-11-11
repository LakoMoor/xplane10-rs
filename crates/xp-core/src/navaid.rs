//! Navaids for the NAVAIDs tab of System Failures.
//!
//! `fail_class::fail_wind` (0x10078c780), g1_tab 5: asks the geoid index for the navaids
//! around the aircraft, bubble-sorts them by name (memcmp, shorter first on a tie of the
//! common part; a stable sort, so equal names keep query order) and draws one check box
//! per navaid labelled `name (TYPE)`. The type suffix by `__vec_nav` type: 2 NDB, 3 VOR,
//! 4/5 LOC, 6 GLS, 7/8/9 MAR, 12/13 DME.
//!
//! The area: the oracle frame at KSEA lists exactly the navaids of earth_nav.dat in
//! 46…48° N × 124…122° W, i.e. the 2×2 one-degree tiles from floor(lat)−1, floor(lon)−1,
//! plus the eight boat navaids `IO_nav` puts first in `__vec_nav` (USS Nimitz LOC, GLS,
//! DME, VOR; USS Perry the same), which ride on the carrier and frigate near the aircraft.
//! The query's own order for equal names (ASTORIA VOR/DME: DME before VOR in the oracle,
//! file order says VOR first) is not modelled: equal names keep file order.

use std::path::Path;

/// One listed navaid.
#[derive(Debug, Clone, PartialEq)]
pub struct Navaid {
    pub kind: u32,
    pub lat: f64,
    pub lon: f64,
    /// Everything after the eighth field, spacing kept ("KAST 26  GS").
    pub name: String,
}

impl Navaid {
    /// The label fail_wind draws.
    pub fn label(&self) -> String {
        let t = match self.kind {
            2 => " (NDB)",
            3 => " (VOR)",
            4 | 5 => " (LOC)",
            6 => " (GLS)",
            7..=9 => " (MAR)",
            12 | 13 => " (DME)",
            _ => "",
        };
        format!("{}{t}", self.name)
    }
}

/// The text after `n` whitespace-separated fields, its inner spacing kept.
fn tail(line: &str, n: usize) -> Option<&str> {
    let mut rest = line.trim_start();
    for _ in 0..n {
        let end = rest.find(char::is_whitespace)?;
        rest = rest[end..].trim_start();
    }
    Some(rest.trim_end())
}

/// Parse earth_nav.dat (810 format) text: the listed kinds only.
pub fn parse(text: &str) -> Vec<Navaid> {
    let mut out = Vec::new();
    for l in text.lines() {
        let mut f = l.split_whitespace();
        let (Some(k), Some(lat), Some(lon)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        let (Ok(kind), Ok(lat), Ok(lon)) = (k.parse::<u32>(), lat.parse(), lon.parse()) else {
            continue;
        };
        if !matches!(kind, 2..=9 | 12 | 13) {
            continue;
        }
        if let Some(name) = tail(l, 8) {
            out.push(Navaid {
                kind,
                lat,
                lon,
                name: name.to_string(),
            });
        }
    }
    out
}

/// The NAVAIDs-tab list around (lat, lon), sorted as fail_wind sorts it.
pub fn fail_list(all: &[Navaid], lat: f64, lon: f64) -> Vec<Navaid> {
    let (la, lo) = (lat.floor() - 1.0, lon.floor() - 1.0);
    let mut v: Vec<Navaid> = ["USS Nimitz", "USS Perry"]
        .iter()
        .flat_map(|n| {
            [4, 6, 12, 3].map(|kind| Navaid {
                kind,
                lat,
                lon,
                name: n.to_string(),
            })
        })
        .collect();
    v.extend(
        all.iter()
            .filter(|n| n.lat >= la && n.lat < la + 2.0 && n.lon >= lo && n.lon < lo + 2.0)
            .cloned(),
    );
    v.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    v
}

/// earth_nav.dat of the install (Custom Data first, as the original prefers it).
pub fn load(install: &Path) -> Vec<Navaid> {
    for p in [
        "Custom Data/earth_nav.dat",
        "Resources/default data/earth_nav.dat",
    ] {
        if let Ok(t) = crate::read_text_lossy(install.join(p)) {
            return parse(&t);
        }
    }
    Vec::new()
}

/// Where the aircraft starts: the first runway end of airport `id` (the port always starts
/// on a runway of its start airport).
pub fn airport_position(install: &Path, id: &str) -> Option<(f64, f64)> {
    let (_, apt) = crate::airport::find_in_install(install, id).ok()?;
    let r = apt.runways.first()?;
    Some((r.end1_lat()?, r.end1_lon()?))
}

/// The NAVAIDs tab rows: a check box per navaid from (20, H − 73), 15 down, a new column
/// 200 right once the next row would fall below y = 25 (46 per column at H = 768). Each
/// box is a session value `m:navfail<i>` (the navaid's fail flag, `__vec_nav` +0x30).
pub fn fail_widgets(list: &[Navaid], h: f32) -> Vec<crate::btnwin::Widget> {
    let (mut x, mut y) = (20.0, h - 58.0);
    list.iter()
        .enumerate()
        .map(|(i, n)| {
            if y < 25.0 {
                y = h - 58.0;
                x += 200.0;
            }
            y -= 15.0;
            crate::btnwin::Widget {
                kind: crate::btnwin::Kind::Bool,
                x,
                y,
                a: 0.0,
                b: 0.0,
                c: 0.0,
                d: 0.0,
                key: format!("m:navfail{i}"),
                label: format!("-|-{}", n.label()),
                items: Vec::new(),
                cond: "fail_class::fail_wind()::g1_tab=5".into(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_area() {
        let t = "I\n810 Version\n\
2  47.41058333 -122.83886111      0   274  50    0.0 CAN  CARNEY NDB\n\
6  46.15775000 -123.87405300     15 10950  10  300275.500 IAST KAST 26  GS\n\
2  47.63248700 -121.38949000    142   362  10    0.0 BF   EAST NDB\n\
99\n";
        let all = parse(t);
        assert_eq!(all.len(), 3);
        let l: Vec<String> = fail_list(&all, 47.46, -122.31)
            .iter()
            .map(Navaid::label)
            .collect();
        assert_eq!(l[0], "CARNEY NDB (NDB)");
        assert_eq!(l[1], "KAST 26  GS (GLS)");
        assert_eq!(l[2], "USS Nimitz (LOC)");
        assert_eq!(l.len(), 10);
    }
}
