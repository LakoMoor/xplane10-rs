//! The GPS airport database the original builds at aircraft load.
//!
//! The oracle logs `I/GPS: Added 19186 airports to the GPS database`. That number is not a
//! property of the scenery alone: it is produced by `GarminUnits::GarminUnits` (oracle
//! `0x100292720`, called from `veh_class::load_veh3` at `0x100224ee5`), which walks the
//! global airport table and offers each airport to the Garmin nav database. Recovered from
//! the disassembly of the pinned build (`reference.lock.json`, sha `6b0ce1e5…`):
//!
//! 1. **Airport type.** `(apt.type & 0x7f) != 1` is skipped when the aircraft's
//!    `acf/_apt_req_plane` is non-zero. Type 1 is the `1` (land airport) header; `16`
//!    seaplane bases and `17` heliports are the other two.
//! 2. **Runways.** A runway is kept when `!(min_len > length_m)`, where `length_m` is the
//!    cached f32 from `vec_rwy_struct::build_cache` (oracle `0x1006b8c70`, ported as
//!    [`runway_length_m`]) and `min_len` is the aircraft's `acf/_min_rwy_len` in metres.
//!    With `acf/_rwy_req_pave` non-zero, only surface codes 1, 2 and 8 survive (the jump
//!    table at `0x100293f8c`).
//! 3. **Airport.** An airport with at least one kept runway is passed to
//!    `AIRAC<Airport>::addExternalDataPoint` (oracle `0x100024760`), which **refuses** it
//!    when its identifier, or `"K"` + its identifier, is already indexed from
//!    `Resources/GNS430/navdata/Airports.txt`. Only accepted airports are counted.
//!
//! So the logged figure is "airports the shipped navdata did not already have". Evidence
//! for the two pieces the disassembly does not pin by itself:
//!
//! - `acf/_min_rwy_len 700.0` is in **feet**. The comparison is against metres
//!   (`get_longest_rwy_mtrs`), and over the shipped install the rule yields 19 186 only at
//!   `700 ft = 213.36 m`; at 700 m it yields 12 922. The conversion site itself was not
//!   located, so the factor is [`FT_TO_M`] by inference.
//! - Water runways (`101`) count as runways. Without them the same rule gives 19 185.
//!
//! What is **not** covered: `acf/_rwy_req_pave = 1` and `acf/_apt_req_plane = 0` are ported
//! from the disassembly but no oracle run exercises them (the oracle only ever loads the
//! default Cessna, see `docs/COMPATIBILITY.md`).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::acf::Acf;

/// Metres per degree of latitude on the original's Earth (`UTL_geoid::set_geoid_to_planet_earth`,
/// oracle `0x10031dfe0`, stores the double `0x40fb210000000000` at `+0x20`).
pub const METERS_PER_DEGREE: f64 = 111_120.0;

/// Degrees → radians as the original spells it (double at oracle `0x1010b31a0`). It is the
/// f32 value of π/180 widened to double, **not** `std::f64::consts::PI / 180.0`.
pub const DEG_TO_RAD: f64 = 0.017_453_294_247_388_84;

/// Feet → metres for `acf/_min_rwy_len`. Inferred, see the module docs.
pub const FT_TO_M: f32 = 0.3048;

/// Runway length exactly as `vec_rwy_struct::build_cache` computes it.
///
/// The widths are the original's: the latitude difference is narrowed to f32 first, the
/// cosine is `cosf` of an f32 angle, the east-west leg is narrowed after the double
/// multiply, and the hypotenuse is f32 before it is scaled in double. On a flat-earth
/// f64 formula eight shipped runways that are surveyed at exactly 700 ft land on the other
/// side of the Cessna's threshold.
pub fn runway_length_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f32 {
    let center_lat = (lat1 + lat2) * 0.5;
    let dlat = (lat2 - lat1) as f32;
    let cos_lat = ((DEG_TO_RAD * center_lat) as f32).cos();
    let dx = ((lon2 - lon1) * f64::from(cos_lat)) as f32;
    let hyp = (dx * dx + dlat * dlat).sqrt();
    (f64::from(hyp) * METERS_PER_DEGREE) as f32
}

/// The three airport header record types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AirportKind {
    /// `1`
    Land,
    /// `16`
    Seaplane,
    /// `17`
    Heliport,
}

/// One runway as the GPS filter sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunwaySummary {
    pub length_m: f32,
    /// apt.dat surface code. The original stores it unchanged in the low four bits of the
    /// runway's flag word (`vec_rwy_struct::encode`, `rwy_surface_mask = 0xf`).
    pub surface: u32,
    /// `true` for a `101` water runway. Its surface code is not in the record; the
    /// original's value for it is unmeasured, and [`GpsFilter::require_paved`] treats it
    /// as unpaved.
    pub water: bool,
}

/// One airport reduced to what the GPS database needs.
#[derive(Debug, Clone, PartialEq)]
pub struct AirportSummary {
    pub kind: AirportKind,
    pub id: String,
    pub runways: Vec<RunwaySummary>,
}

impl AirportSummary {
    /// `vec_apt_struct::get_longest_rwy_mtrs` (oracle `0x1000e1260`): 0 with no runways.
    pub fn longest_runway_m(&self) -> f32 {
        self.runways.iter().fold(0.0_f32, |m, r| m.max(r.length_m))
    }
}

fn field<'a>(fields: &[&'a [u8]], i: usize, line_no: usize, what: &str) -> Result<&'a [u8]> {
    match fields.get(i) {
        Some(f) => Ok(f),
        None => bail!("line {line_no}: record is missing field {i} ({what})"),
    }
}

fn parse_f64(fields: &[&[u8]], i: usize, line_no: usize, what: &str) -> Result<f64> {
    let raw = field(fields, i, line_no, what)?;
    let text =
        std::str::from_utf8(raw).with_context(|| format!("line {line_no}: {what} is not ASCII"))?;
    text.parse::<f64>()
        .with_context(|| format!("line {line_no}: bad {what} {text:?}"))
}

/// Read the airports of one apt.dat without retaining its records.
///
/// The default `apt.dat` is 178 MB and 2.6 million records; [`crate::aptdat::AptDat`]
/// keeps every line, which is right for inspecting one airport and wrong for a table of
/// 34 745. Only the `1`/`16`/`17` headers and the `100`/`101` runway records are decoded.
pub fn scan_apt_dat(path: impl AsRef<Path>) -> Result<Vec<AirportSummary>> {
    let path = path.as_ref();
    let bytes =
        std::fs::read(path).with_context(|| format!("reading apt.dat {}", path.display()))?;
    scan_apt_bytes(&bytes).with_context(|| format!("scanning apt.dat {}", path.display()))
}

/// [`scan_apt_dat`] over bytes already in memory.
pub fn scan_apt_bytes(bytes: &[u8]) -> Result<Vec<AirportSummary>> {
    let mut out: Vec<AirportSummary> = Vec::new();
    let mut fields: Vec<&[u8]> = Vec::with_capacity(32);
    // Lines 1 and 2 are the marker and the version/generator line.
    for (n, line) in bytes.split(|b| *b == b'\n').enumerate().skip(2) {
        let line_no = n + 1;
        fields.clear();
        fields.extend(
            line.split(|b| b.is_ascii_whitespace())
                .filter(|f| !f.is_empty()),
        );
        let Some(kind) = fields.first().copied() else {
            continue;
        };
        let header = match kind {
            b"1" => Some(AirportKind::Land),
            b"16" => Some(AirportKind::Seaplane),
            b"17" => Some(AirportKind::Heliport),
            _ => None,
        };
        if let Some(kind) = header {
            let id = field(&fields, 4, line_no, "airport identifier")?;
            out.push(AirportSummary {
                kind,
                id: String::from_utf8_lossy(id).into_owned(),
                runways: Vec::new(),
            });
            continue;
        }
        let (water, ends) = match kind {
            // 100 <width> <surface> … <name> <lat> <lon> … <name> <lat> <lon> …
            b"100" => (false, [9, 10, 18, 19]),
            // 101 <width> <buoys> <name> <lat> <lon> <name> <lat> <lon>
            b"101" => (true, [4, 5, 7, 8]),
            _ => continue,
        };
        let Some(airport) = out.last_mut() else {
            bail!("line {line_no}: runway record before any airport header");
        };
        let lat1 = parse_f64(&fields, ends[0], line_no, "runway end 1 latitude")?;
        let lon1 = parse_f64(&fields, ends[1], line_no, "runway end 1 longitude")?;
        let lat2 = parse_f64(&fields, ends[2], line_no, "runway end 2 latitude")?;
        let lon2 = parse_f64(&fields, ends[3], line_no, "runway end 2 longitude")?;
        let surface = if water {
            0
        } else {
            let raw = field(&fields, 2, line_no, "runway surface code")?;
            std::str::from_utf8(raw)
                .ok()
                .and_then(|t| t.parse::<u32>().ok())
                .with_context(|| format!("line {line_no}: bad runway surface code"))?
        };
        airport.runways.push(RunwaySummary {
            length_m: runway_length_m(lat1, lon1, lat2, lon2),
            surface,
            water,
        });
    }
    Ok(out)
}

/// Scenery packs in the original's priority order, as paths relative to the install.
///
/// Reproduces the oracle's "I found the following scenery packages (prioritized in this
/// order)" list for the pinned install: the `SCENERY_PACK` lines of
/// `Custom Scenery/scenery_packs.ini` in file order, then the directories of
/// `Resources/default scenery/` in byte order. How the original orders custom packs that
/// are *absent* from the ini is not measured here; such a pack is an error rather than a
/// guess.
pub fn scenery_packs(install: &Path) -> Result<Vec<String>> {
    let ini = install.join("Custom Scenery/scenery_packs.ini");
    let text =
        crate::read_text_lossy(&ini).with_context(|| format!("reading {}", ini.display()))?;
    let mut packs: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("SCENERY_PACK ") {
            packs.push(rest.trim_end().to_string());
        }
    }
    let custom = install.join("Custom Scenery");
    for entry in
        std::fs::read_dir(&custom).with_context(|| format!("listing {}", custom.display()))?
    {
        let entry = entry.with_context(|| format!("listing {}", custom.display()))?;
        if !entry.path().is_dir() {
            continue;
        }
        let rel = format!("Custom Scenery/{}/", entry.file_name().to_string_lossy());
        if !packs.contains(&rel) {
            bail!("{rel} is not listed in scenery_packs.ini; its priority is unmeasured");
        }
    }
    let default_root = install.join("Resources/default scenery");
    let mut defaults: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&default_root)
        .with_context(|| format!("listing {}", default_root.display()))?
    {
        let entry = entry.with_context(|| format!("listing {}", default_root.display()))?;
        if entry.path().is_dir() {
            defaults.push(format!(
                "Resources/default scenery/{}/",
                entry.file_name().to_string_lossy()
            ));
        }
    }
    defaults.sort();
    packs.extend(defaults);
    Ok(packs)
}

/// The `Earth nav data/apt.dat` of every pack that has one, in pack priority order.
pub fn earth_apt_files(install: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for pack in scenery_packs(install)? {
        let path = install.join(&pack).join("Earth nav data/apt.dat");
        if path.is_file() {
            out.push(path);
        }
    }
    Ok(out)
}

/// The world airport table: every pack's airports, the first occurrence of an identifier
/// winning.
///
/// The first-wins rule is what `IO_apt`'s `std::set<std::string>&` argument is for; the
/// shipped install cannot distinguish it from last-wins by *count* (every custom airport
/// also exists in the default file), so the direction is inferred from pack priority.
pub fn world_airports(install: &Path) -> Result<Vec<AirportSummary>> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<AirportSummary> = Vec::new();
    for file in earth_apt_files(install)? {
        for airport in scan_apt_dat(&file)? {
            if seen.insert(airport.id.clone()) {
                out.push(airport);
            }
        }
    }
    Ok(out)
}

/// Airport identifiers already present in a GNS430 `Airports.txt` (`A,<id>,<name>,…` rows).
pub fn navdata_airport_ids(path: impl AsRef<Path>) -> Result<BTreeSet<String>> {
    let path = path.as_ref();
    let text = crate::read_text_lossy(path)
        .with_context(|| format!("reading navdata {}", path.display()))?;
    let mut ids = BTreeSet::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("A,") {
            if let Some(id) = rest.split(',').next() {
                ids.insert(id.to_string());
            }
        }
    }
    Ok(ids)
}

/// The aircraft-supplied arguments of `GarminUnits::GarminUnits`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpsFilter {
    /// `acf/_min_rwy_len`, converted to metres.
    pub min_rwy_len_m: f32,
    /// `acf/_apt_req_plane`: keep land airports only.
    pub require_land_airport: bool,
    /// `acf/_rwy_req_pave`: keep paved runways only.
    pub require_paved: bool,
}

impl GpsFilter {
    pub fn from_acf(acf: &Acf) -> Result<GpsFilter> {
        let min_ft = acf
            .f32_or_int("acf/_min_rwy_len")
            .context("aircraft has no acf/_min_rwy_len")?;
        let land = acf
            .i32("acf/_apt_req_plane")
            .context("aircraft has no acf/_apt_req_plane")?;
        let paved = acf
            .i32("acf/_rwy_req_pave")
            .context("aircraft has no acf/_rwy_req_pave")?;
        Ok(GpsFilter {
            min_rwy_len_m: min_ft * FT_TO_M,
            require_land_airport: land != 0,
            require_paved: paved != 0,
        })
    }

    /// The per-runway test, in the original's order: length first, then surface.
    pub fn keeps_runway(&self, r: &RunwaySummary) -> bool {
        // `ucomiss min, len; ja skip` — written as a negation so a NaN length is kept,
        // as it is in the original.
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        let long_enough = !(self.min_rwy_len_m > r.length_m);
        if !long_enough {
            return false;
        }
        if !self.require_paved {
            return true;
        }
        !r.water && matches!(r.surface & 0xf, 1 | 2 | 8)
    }

    pub fn keeps_airport(&self, a: &AirportSummary) -> bool {
        if self.require_land_airport && a.kind != AirportKind::Land {
            return false;
        }
        a.runways.iter().any(|r| self.keeps_runway(r))
    }
}

/// The airports the original adds to the GPS database, in table order.
///
/// `navdata_ids` plays the role of the Garmin database's file index, which is the only map
/// the original consults before accepting. `airports` is expected to be identifier-unique
/// already, as [`world_airports`] makes it.
pub fn gps_added_airports<'a>(
    airports: &'a [AirportSummary],
    navdata_ids: &BTreeSet<String>,
    filter: &GpsFilter,
) -> Vec<&'a AirportSummary> {
    let mut added: Vec<&AirportSummary> = Vec::new();
    for a in airports {
        if !filter.keeps_airport(a) {
            continue;
        }
        if navdata_ids.contains(&a.id) || navdata_ids.contains(&format!("K{}", a.id)) {
            continue;
        }
        added.push(a);
    }
    added
}
