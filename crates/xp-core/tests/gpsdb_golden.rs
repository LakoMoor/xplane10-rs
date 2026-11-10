//! Differential test for the GPS airport database: the port must add exactly as many
//! airports as the oracle logged, and the intermediate numbers must match an independent
//! measurement of the shipped content (`tests/golden/gpsdb/c172sp_summary.txt`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use xp_core::acf::Acf;
use xp_core::gpsdb::{self, AirportKind, AirportSummary, GpsFilter, RunwaySummary, FT_TO_M};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden")
}

fn read_golden() -> Result<BTreeMap<String, String>> {
    let path = golden_dir().join("gpsdb/c172sp_summary.txt");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading golden {}", path.display()))?;
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once(" = ") else {
            bail!("{}: bad golden line {line:?}", path.display());
        };
        out.insert(k.to_string(), v.to_string());
    }
    Ok(out)
}

fn want<'a>(g: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str> {
    g.get(key)
        .map(|s| s.as_str())
        .with_context(|| format!("golden has no {key}"))
}

fn want_usize(g: &BTreeMap<String, String>, key: &str) -> Result<usize> {
    want(g, key)?
        .parse::<usize>()
        .with_context(|| format!("golden {key} is not a count"))
}

fn want_bits(g: &BTreeMap<String, String>, key: &str) -> Result<u32> {
    let raw = want(g, key)?;
    let hex = raw
        .strip_prefix("0x")
        .with_context(|| format!("golden {key} is not hex"))?;
    u32::from_str_radix(hex, 16).with_context(|| format!("golden {key} is not hex"))
}

/// The number the original itself printed.
fn oracle_added() -> Result<usize> {
    let path = golden_dir().join("original/boot_trace.txt");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    for line in text.lines() {
        if let Some(rest) = line.split("I/GPS: Added ").nth(1) {
            let n = rest
                .split(' ')
                .next()
                .and_then(|t| t.parse::<usize>().ok())
                .with_context(|| format!("bad I/GPS line {line:?}"))?;
            return Ok(n);
        }
    }
    bail!("{} has no I/GPS line", path.display())
}

#[test]
fn golden_agrees_with_the_oracle_log() -> Result<()> {
    let g = read_golden()?;
    assert_eq!(want_usize(&g, "gps.added")?, oracle_added()?);
    Ok(())
}

/// Coordinates are the shipped `100` records of four strips surveyed at exactly 700 ft.
#[test]
fn runway_length_keeps_the_originals_float_widths() -> Result<()> {
    let g = read_golden()?;
    let cases = [
        ("1IN1", 39.48933946, -86.65310175, 39.48742054, -86.65301493),
        ("3PN9", 40.44115790, -79.55104412, 40.44239210, -79.54911148),
        (
            "Z55",
            62.29449460,
            -146.58054656,
            62.29288428,
            -146.57829734,
        ),
        ("TS67", 29.62603182, -98.27866929, 29.62442150, -98.27746627),
    ];
    for (id, lat1, lon1, lat2, lon2) in cases {
        let got = gpsdb::runway_length_m(lat1, lon1, lat2, lon2);
        let want = want_bits(&g, &format!("length.{id}.bits"))?;
        assert_eq!(got.to_bits(), want, "{id}: got {got}");
        // direction must not matter to the length
        let back = gpsdb::runway_length_m(lat2, lon2, lat1, lon1);
        assert_eq!(back.to_bits(), want, "{id} reversed: got {back}");
    }
    Ok(())
}

#[test]
fn seven_hundred_feet_is_the_cessnas_threshold() -> Result<()> {
    let g = read_golden()?;
    let threshold = 700.0_f32 * FT_TO_M;
    assert_eq!(
        threshold.to_bits(),
        want_bits(&g, "filter.min_rwy_len_m.bits")?
    );
    let filter = GpsFilter {
        min_rwy_len_m: threshold,
        require_land_airport: true,
        require_paved: false,
    };
    let rwy = |bits: u32| RunwaySummary {
        length_m: f32::from_bits(bits),
        surface: 3,
        water: false,
    };
    // 1IN1 is 0.0003 m over, 3PN9 is 0.0011 m under: same survey, opposite outcome.
    assert!(filter.keeps_runway(&rwy(want_bits(&g, "length.1IN1.bits")?)));
    assert!(!filter.keeps_runway(&rwy(want_bits(&g, "length.3PN9.bits")?)));
    // equality is kept: the original skips only when min > length
    assert!(filter.keeps_runway(&rwy(threshold.to_bits())));
    Ok(())
}

#[test]
fn filter_follows_the_disassembled_branches() {
    let long = 1000.0_f32;
    let rwy = |surface: u32, water: bool| RunwaySummary {
        length_m: long,
        surface,
        water,
    };
    let apt = |kind, runways: Vec<RunwaySummary>| AirportSummary {
        kind,
        id: "TEST".to_string(),
        runways,
    };
    let open = GpsFilter {
        min_rwy_len_m: 500.0,
        require_land_airport: false,
        require_paved: false,
    };
    let land_only = GpsFilter {
        require_land_airport: true,
        ..open
    };
    let paved = GpsFilter {
        require_paved: true,
        ..open
    };
    assert!(open.keeps_airport(&apt(AirportKind::Heliport, vec![rwy(3, false)])));
    assert!(!land_only.keeps_airport(&apt(AirportKind::Heliport, vec![rwy(1, false)])));
    assert!(!land_only.keeps_airport(&apt(AirportKind::Seaplane, vec![rwy(0, true)])));
    assert!(!open.keeps_airport(&apt(AirportKind::Land, vec![])));
    // jump table at oracle 0x100293f8c: 1, 2 and 8 skip the paved test, everything else takes it
    for surface in 0..16_u32 {
        let kept = paved.keeps_runway(&rwy(surface, false));
        assert_eq!(kept, matches!(surface, 1 | 2 | 8), "surface {surface}");
    }
    assert!(!paved.keeps_runway(&rwy(0, true)));
}

#[test]
fn scan_reads_headers_and_both_runway_records() -> Result<()> {
    let text = "I\n1050 test\n\n\
1 21 0 0 1IN1 Shearer\n\
100 18.29 3 0 0.00 0 0 0 18 39.48933946 -086.65310175 0 0 0 0 0 0 36 39.48742054 -086.65301493 0 0 0 0 0 0\n\
16 0 0 0 W01 Water Base\n\
101 30.48 0 07 47.0 -122.0 25 47.0 -121.99\n\
17 5 0 0 H01 Pad\n\
102 H1 47.0 -122.0 0 20 20 1 0 0 0.25 0\n\
99\n";
    let apts = gpsdb::scan_apt_bytes(text.as_bytes())?;
    assert_eq!(apts.len(), 3);
    assert_eq!(apts[0].id, "1IN1");
    assert_eq!(apts[0].kind, AirportKind::Land);
    assert_eq!(apts[0].runways.len(), 1);
    assert_eq!(apts[0].runways[0].surface, 3);
    assert_eq!(apts[0].longest_runway_m().to_bits(), 0x4355_5c3f);
    assert_eq!(apts[1].kind, AirportKind::Seaplane);
    assert!(apts[1].runways[0].water);
    assert_eq!(apts[2].kind, AirportKind::Heliport);
    assert!(apts[2].runways.is_empty(), "a 102 helipad is not a runway");
    assert_eq!(apts[2].longest_runway_m(), 0.0);
    Ok(())
}

/// The differential: the port's count against the number in the oracle's own Log.txt.
#[test]
fn gps_database_matches_the_oracle_count() -> Result<()> {
    let Some(root) = xp_core::install_root() else {
        eprintln!(
            "SKIP gps_database_matches_the_oracle_count: XP_INSTALL unset (no coverage claimed)"
        );
        return Ok(());
    };
    let g = read_golden()?;

    let acf = Acf::load(root.join("Aircraft/General Aviation/Cessna 172SP/Cessna_172SP.acf"))?;
    let filter = GpsFilter::from_acf(&acf)?;
    assert_eq!(
        filter.min_rwy_len_m.to_bits(),
        want_bits(&g, "filter.min_rwy_len_m.bits")?
    );
    assert_eq!(
        usize::from(filter.require_land_airport),
        want_usize(&g, "filter.require_land_airport")?
    );
    assert_eq!(
        usize::from(filter.require_paved),
        want_usize(&g, "filter.require_paved")?
    );

    assert_eq!(
        gpsdb::earth_apt_files(&root)?.len(),
        want_usize(&g, "world.apt_files")?
    );
    let world = gpsdb::world_airports(&root)?;
    let count = |k: AirportKind| world.iter().filter(|a| a.kind == k).count();
    assert_eq!(world.len(), want_usize(&g, "world.airports")?);
    assert_eq!(count(AirportKind::Land), want_usize(&g, "world.land")?);
    assert_eq!(
        count(AirportKind::Seaplane),
        want_usize(&g, "world.seaplane")?
    );
    assert_eq!(
        count(AirportKind::Heliport),
        want_usize(&g, "world.heliport")?
    );

    let ids = gpsdb::navdata_airport_ids(root.join("Resources/GNS430/navdata/Airports.txt"))?;
    assert_eq!(ids.len(), want_usize(&g, "navdata.airports")?);

    let kept: Vec<&AirportSummary> = world.iter().filter(|a| filter.keeps_airport(a)).collect();
    assert_eq!(kept.len(), want_usize(&g, "gps.kept_by_filter")?);
    let by_id = kept.iter().filter(|a| ids.contains(&a.id)).count();
    let by_k = kept
        .iter()
        .filter(|a| !ids.contains(&a.id) && ids.contains(&format!("K{}", a.id)))
        .count();
    assert_eq!(by_id, want_usize(&g, "gps.refused.id_in_navdata")?);
    assert_eq!(by_k, want_usize(&g, "gps.refused.k_plus_id_in_navdata")?);

    let added = gpsdb::gps_added_airports(&world, &ids, &filter);
    assert_eq!(added.len(), oracle_added()?, "I/GPS: Added N airports");
    assert_eq!(added.len(), kept.len() - by_id - by_k);
    Ok(())
}

/// The pack list is the oracle's own "prioritized in this order" list.
#[test]
fn scenery_pack_order_matches_the_oracle_log() -> Result<()> {
    let Some(root) = xp_core::install_root() else {
        eprintln!("SKIP scenery_pack_order_matches_the_oracle_log: XP_INSTALL unset (no coverage claimed)");
        return Ok(());
    };
    let path = golden_dir().join("original/boot_trace.txt");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut want: Vec<String> = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with("I found the following scenery packages") {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        let t = line.trim();
        let Some((index, pack)) = t.split_once(' ') else {
            break;
        };
        if index.parse::<usize>().ok() != Some(want.len()) {
            break;
        }
        want.push(pack.trim().to_string());
    }
    assert_eq!(want.len(), 24, "oracle listed 24 packs");
    assert_eq!(gpsdb::scenery_packs(&root)?, want);
    Ok(())
}
