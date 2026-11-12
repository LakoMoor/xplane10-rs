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
    }
}
