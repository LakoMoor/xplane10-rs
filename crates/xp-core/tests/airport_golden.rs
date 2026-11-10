//! Tests for the local frame and the runway layout ported from the oracle's
//! `UTL_geoid::set_center` and `build_rwy_layout`.
//!
//! No oracle output exists for these yet (the original prints no runway corner anywhere
//! we can capture), so what is pinned here is the arithmetic's own invariants and its
//! agreement with a *second* routine of the original, `vec_rwy_struct::build_cache`,
//! ported independently in `gpsdb`. That is consistency, not parity.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use xp_core::airport::{self, StartPose};
use xp_core::aptdat::AptDat;
use xp_core::geoid::{runway_layout, LocalFrame, EARTH_RADIUS_M};
use xp_core::gpsdb;
use xp_core::scene::{Topology, VERTEX_STRIDE};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/fixtures/ksea_prefix.aptdat")
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn ksea() -> Result<(xp_core::aptdat::Airport, Vec<airport::Runway>)> {
    let apt = AptDat::load(fixture())?
        .airports
        .into_iter()
        .next()
        .context("fixture has no airport")?;
    let runways = airport::runways(&apt)?;
    Ok((apt, runways))
}

#[test]
fn frame_axes_are_east_up_south() {
    let frame = LocalFrame::earth(-122.5, 47.5);
    let origin = frame.to_local(47.5, -122.5, 0.0);
    assert!(dist(origin, [0.0; 3]) < 1e-3, "{origin:?}");

    let north = frame.to_local(47.51, -122.5, 0.0);
    assert!(north[2] < -1000.0 && north[0].abs() < 1e-3, "{north:?}");
    let east = frame.to_local(47.5, -122.49, 0.0);
    assert!(east[0] > 700.0 && east[2].abs() < 1.0, "{east:?}");
    let up = frame.to_local(47.5, -122.5, 100.0);
    assert!((up[1] - 100.0).abs() < 1e-3 && up[0].abs() < 1e-3, "{up:?}");

    // 0.01 degree of latitude on a sphere of the original's radius.
    let want = (EARTH_RADIUS_M * 0.01_f64.to_radians()) as f32;
    assert!((-north[2] - want).abs() < 0.01, "{} vs {want}", -north[2]);
    // Nothing is flattened: the surface drops below the tangent plane with distance.
    assert!(north[1] < 0.0 && north[1] > -0.2, "{north:?}");
}

#[test]
fn ksea_runway_fields_decode() -> Result<()> {
    let (apt, runways) = ksea()?;
    assert_eq!(runways.len(), 3);
    let r = &runways[0];
    assert_eq!((r.name1.as_str(), r.name2.as_str()), ("16L", "34R"));
    assert_eq!(r.surface, 2);
    assert_eq!(r.extent.width_m, 46.02);
    assert_eq!((r.extent.extend1_m, r.extent.extend2_m), (46.0, 122.0));
    assert_eq!((r.displaced1_m, r.displaced2_m), (0.0, 0.0));
    assert_eq!(
        airport::pavement_texture(r.surface),
        Some("concrete/rway_pave.dds")
    );
    assert_eq!(
        airport::pavement_texture(13),
        None,
        "water has no pavement directory"
    );
    // `1    433 1 0 KSEA …`: 433 ft
    assert!((airport::elevation_m(&apt)? - 131.9784).abs() < 1e-3);
    Ok(())
}

#[test]
fn layout_is_a_rectangle_of_the_declared_size() -> Result<()> {
    let (_, runways) = ksea()?;
    let frame = LocalFrame::earth(-122.5, 47.5);
    for r in &runways {
        let bare = runway_layout(&frame, &r.extent, false, 0.0);
        let full = runway_layout(&frame, &r.extent, true, 0.0);
        // both short edges are the declared width
        for q in [&bare, &full] {
            assert!(
                (dist(q[0], q[1]) - r.extent.width_m).abs() < 2e-2,
                "{}",
                r.name1
            );
            assert!(
                (dist(q[2], q[3]) - r.extent.width_m).abs() < 2e-2,
                "{}",
                r.name1
            );
        }
        // the extensions lengthen it by exactly their sum
        let grew = dist(full[0], full[2]) - dist(bare[0], bare[2]);
        let want = r.extent.extend1_m + r.extent.extend2_m;
        assert!(
            (grew - want).abs() < 2e-2,
            "{}: grew {grew}, want {want}",
            r.name1
        );
        // and move each end outward, not inward
        assert!(dist(full[0], bare[2]) > dist(bare[0], bare[2]));
        assert!(dist(full[2], bare[0]) > dist(bare[0], bare[2]));
    }
    Ok(())
}

/// The original has two notions of runway length and they do not agree: the layout is on a
/// sphere of radius 6 378 145 m (111 319.5 m per degree) while `build_cache` scales a
/// flat-degree hypotenuse by 111 120 m per degree. The ratio is a constant 1.001795.
#[test]
fn layout_length_and_cached_length_differ_by_the_originals_own_ratio() -> Result<()> {
    let (_, runways) = ksea()?;
    let frame = LocalFrame::earth(-122.5, 47.5);
    let want = (EARTH_RADIUS_M * 1.0_f64.to_radians() / gpsdb::METERS_PER_DEGREE) as f32;
    assert!((want - 1.001_795).abs() < 1e-5, "{want}");
    for r in &runways {
        let e = &r.extent;
        let q = runway_layout(&frame, e, false, 0.0);
        let mid1 = [
            (q[0][0] + q[1][0]) * 0.5,
            q[0][1],
            (q[0][2] + q[1][2]) * 0.5,
        ];
        let mid2 = [
            (q[2][0] + q[3][0]) * 0.5,
            q[2][1],
            (q[2][2] + q[3][2]) * 0.5,
        ];
        let cached = gpsdb::runway_length_m(e.lat1, e.lon1, e.lat2, e.lon2);
        let ratio = dist(mid1, mid2) / cached;
        assert!((ratio - want).abs() < 2e-4, "{}: ratio {ratio}", r.name1);
    }
    Ok(())
}

#[test]
fn start_pose_puts_the_far_end_ahead_of_the_nose() -> Result<()> {
    let (apt, runways) = ksea()?;
    let frame = LocalFrame::earth(-122.5, 47.5);
    let alt = airport::elevation_m(&apt)?;
    let r = &runways[0];
    let pose = StartPose::on_runway_end(&frame, r, "34R", alt, -1.35)?;
    // 34R is the south end; taking off on it means heading north-ish, i.e. toward −z.
    assert!(pose.forward[1] < -0.99, "{:?}", pose.forward);

    let e = &r.extent;
    let here = pose.to_model(frame.to_local(e.lat2, e.lon2, alt));
    assert!(dist(here, [0.0, -1.35, 0.0]) < 1e-3, "{here:?}");
    let far = pose.to_model(frame.to_local(e.lat1, e.lon1, alt));
    assert!(far[0].abs() < 0.05, "far end is on the centreline: {far:?}");
    assert!(
        far[2] < -3600.0 && far[2] > -3660.0,
        "nose points along −z: {far:?}"
    );

    // a point to the east of a north-facing aircraft is on its +x side
    let east = pose.to_model(frame.to_local(e.lat2, e.lon2 + 0.001, alt));
    assert!(east[0] > 70.0, "{east:?}");

    assert!(StartPose::on_runway_end(&frame, r, "09", alt, 0.0).is_err());
    Ok(())
}

#[test]
fn runway_mesh_is_one_textured_quad_per_runway() -> Result<()> {
    let (apt, runways) = ksea()?;
    // Centred on the start point, as the frame run does: with the centre 16 km away the
    // tangent plane is tilted 0.0025 rad against the local horizontal and the far end of the
    // runway sits 3 m *above* the near one.
    let e = runways[0].extent;
    let frame = LocalFrame::earth(e.lon2 as f32, e.lat2 as f32);
    let alt = airport::elevation_m(&apt)?;
    let pose = StartPose::on_runway_end(&frame, &runways[0], "34R", alt, -1.35)?;
    let mesh = airport::build_runway_mesh(
        &frame,
        &runways,
        alt,
        &|p| pose.to_model(p),
        [0.0, 1.0, 0.0],
    )?;

    assert_eq!(mesh.vertex_bytes.len(), 3 * 4 * VERTEX_STRIDE);
    assert_eq!(mesh.indices.len(), 18);
    assert_eq!(mesh.triangle_count(), 6);
    assert_eq!(mesh.draws.len(), 3);
    // all three KSEA runways are concrete, so they share one slot
    assert_eq!(mesh.textures.len(), 1);
    assert_eq!(mesh.textures[0].path, "concrete/rway_pave.dds");
    for (n, d) in mesh.draws.iter().enumerate() {
        assert_eq!(d.topology, Topology::Triangles);
        assert_eq!(d.texture, Some(0));
        assert_eq!(d.upload_range(), (n as u32 * 6)..(n as u32 * 6 + 6));
        assert!(mesh.indices[d.index_offset..d.index_offset + 6]
            .iter()
            .all(|i| (*i as usize) >= d.vertex_offset && (*i as usize) < d.vertex_offset + 4));
    }
    // every vertex lies on the ground plane under the gear, to within the earth's curvature
    // over a 3.7 km runway (about 1 m)
    for v in mesh.vertex_bytes.chunks_exact(VERTEX_STRIDE) {
        let y = f32::from_le_bytes([v[4], v[5], v[6], v[7]]);
        assert!(y <= -1.35 + 1e-3 && y > -1.35 - 2.5, "y = {y}");
        let ny = f32::from_le_bytes([v[16], v[17], v[18], v[19]]);
        assert!((ny - 1.0).abs() < 1e-6, "normal y {ny}");
    }
    Ok(())
}

#[test]
fn one_airport_can_be_cut_out_of_a_larger_file() -> Result<()> {
    let text = "I\n1050 test\n\n\
1 10 0 0 AAAA First\n\
100 30.00 1 0 0.00 0 0 0 09 10.0 20.0 0 0 0 0 0 0 27 10.0 20.01 0 0 0 0 0 0\n\
1 20 0 0 BBBB Second\n\
100 45.00 2 0 0.00 0 0 0 18 11.0 21.0 0 10 0 0 0 0 36 10.99 21.0 0 20 0 0 0 0\n\
110 1 0.25 0.0 pavement\n\
17 5 0 0 HHHH Pad\n\
102 H1 12.0 22.0 0 20 20 1 0 0 0.25 0\n\
99\n";
    let cut = airport::extract_airport_text(text.as_bytes(), "BBBB").context("BBBB not found")?;
    assert!(
        cut.starts_with("I\n1050 test\n1 20 0 0 BBBB Second\n"),
        "{cut:?}"
    );
    assert!(!cut.contains("HHHH") && !cut.contains("AAAA") && !cut.contains("\n99"));
    let apt = AptDat::parse(&cut)?
        .airports
        .into_iter()
        .next()
        .context("no airport")?;
    let runways = airport::runways(&apt)?;
    assert_eq!(runways.len(), 1);
    assert_eq!(runways[0].extent.extend2_m, 20.0);
    // the last airport ends at the closing 99, and an absent one is None
    let last = airport::extract_airport_text(text.as_bytes(), "HHHH").context("HHHH not found")?;
    assert!(
        last.ends_with("102 H1 12.0 22.0 0 20 20 1 0 0 0.25 0\n"),
        "{last:?}"
    );
    assert!(airport::extract_airport_text(text.as_bytes(), "ZZZZ").is_none());
    Ok(())
}

/// Which pack `KSEA` comes from. `Global Airports` is first in the oracle's pack list but does
/// not contain KSEA at all, so the lookup falls through to the demo pack.
#[test]
fn ksea_resolves_through_the_pack_priority_order() -> Result<()> {
    let Some(root) = xp_core::install_root() else {
        eprintln!("SKIP ksea_resolves_through_the_pack_priority_order: XP_INSTALL unset (no coverage claimed)");
        return Ok(());
    };
    let (file, apt) = airport::find_in_install(&root, "KSEA")?;
    assert!(
        file.ends_with("Custom Scenery/KSEA Demo Area/Earth nav data/apt.dat"),
        "{}",
        file.display()
    );
    let names: Vec<String> = airport::runways(&apt)?
        .iter()
        .map(|r| format!("{}/{}", r.name1, r.name2))
        .collect();
    assert_eq!(names, ["16L/34R", "16C/34C", "16R/34L"]);
    assert!(airport::find_in_install(&root, "ZZZZ").is_err());
    Ok(())
}

/// DIFFERENTIAL against the oracle: its own matrices say where the aircraft is, and the ported
/// transform must put KSEA's 34R end beside it — about 18 m behind it along the runway and under a
/// metre off the centreline — when the frame is centred where `region_center` says.
#[test]
fn ported_frame_agrees_with_the_oracles_aircraft_position() -> Result<()> {
    let probe = xp_core::oracle::Probe::load(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/golden/original/frames/chase_default.probe.txt"),
    )?;
    let (_, runways) = ksea()?;
    let e = runways[0].extent;
    let (clon, clat) = xp_core::geoid::region_center(e.lat2, e.lon2);
    assert_eq!((clon, clat), (-122.5, 47.0));
    let frame = LocalFrame::earth(clon, clat);

    let acf = probe.model_to_world([0.0, 0.0, 0.0]);
    // 50 km from the centre the vertical leans 0.43 degrees, so altitude moves a point
    // sideways (1 m per 130 m). The oracle's aircraft is at y = −99.3, which is about 97 m
    // MSL here, not the 132 m of the airport header: the runway follows the terrain.
    let end = frame.to_local(e.lat2, e.lon2, 97.0);
    let far = frame.to_local(e.lat1, e.lon1, 97.0);
    assert!(
        (acf[1] - f64::from(end[1])).abs() < 2.5,
        "aircraft height above the end"
    );
    let (dx, dz) = (f64::from(far[0] - end[0]), f64::from(far[2] - end[2]));
    let len = (dx * dx + dz * dz).sqrt();
    let (ux, uz) = (dx / len, dz / len);
    let (ax, az) = (acf[0] - f64::from(end[0]), acf[2] - f64::from(end[2]));
    let along = ax * ux + az * uz;
    let across = ax * -uz + az * ux;
    assert!((along - 18.2).abs() < 0.5, "along {along}");
    assert!(across.abs() < 1.0, "across {across}");

    // the chase camera: 60 degrees wide at 4:3, 17.44 m behind the model origin, near 1 m
    assert!((probe.aspect() - 4.0 / 3.0).abs() < 1e-5);
    assert!((probe.fov_y_rad().to_degrees() - 46.826).abs() < 1e-2);
    let eye = probe.eye_in_model();
    assert!(eye[0].abs() < 1e-5 && eye[1].abs() < 1e-5 && (eye[2] - 17.4396).abs() < 1e-3);
    let (near, far_plane) = probe.near_far();
    assert!(
        (near - 1.0).abs() < 1e-3 && far_plane > 90_000.0,
        "{near} {far_plane}"
    );
    // and the round trip through both matrices is the identity
    let back = probe.world_to_model([acf[0] as f32, acf[1] as f32, acf[2] as f32]);
    assert!(back.iter().all(|v| v.abs() < 0.02), "{back:?}");
    Ok(())
}

/// The sun the oracle's sky model held for the committed frame, at the field offsets its own
/// static initialiser names (`skyc/sun_dir_*`, `skyc/sun_amb_*`).
#[test]
fn probe_carries_the_oracles_sun() -> Result<()> {
    let probe = xp_core::oracle::Probe::load(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/golden/original/frames/chase_default.probe.txt"),
    )?;
    let sun = probe.sun().context("probe has no sky block")?;
    assert!((sun.heading_deg - 235.42).abs() < 0.05 && (sun.pitch_deg - 34.77).abs() < 0.05);
    assert!(
        (sun.direct_rgb[0] - 0.7803).abs() < 1e-3 && (sun.ambient_rgb[2] - 0.1961).abs() < 1e-3
    );
    // south-west and up: +x is east, +z is south
    let d = sun.direction_world();
    assert!(d[0] < -0.6 && d[1] > 0.5 && d[2] > 0.4, "{d:?}");
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    assert!((len - 1.0).abs() < 1e-5);
    // carried into model space it stays a unit vector, and stays mostly "up"
    let m = probe.world_dir_to_model(d);
    let mlen = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
    assert!((mlen - 1.0).abs() < 1e-3 && m[1] > 0.5, "{m:?}");
    Ok(())
}
