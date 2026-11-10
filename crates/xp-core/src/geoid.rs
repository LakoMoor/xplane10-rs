//! The original's local coordinate frame: latitude/longitude → scene metres.
//!
//! Recovered from the pinned oracle build by disassembly (addresses in
//! `tests/original_patch/re/README.md`):
//!
//! - `UTL_geoid::set_geoid_to_planet_earth` (`0x10031dfe0`) stores the planet radius
//!   `6 378 145.0` at `+0x08`.
//! - `UTL_geoid::set_center(float lon, float lat)` (`0x10031e4c0`) stores the centre in
//!   radians at `+0x50` (longitude) and `+0x40` (latitude), and `sin`/`cos` of the centre
//!   latitude at `+0x60`/`+0x70`. The centre arrives as **f32** and is widened before the
//!   degree→radian multiply.
//! - `build_rwy_layout` (`0x1000e13d0`) is the consumer read here: it turns each runway end
//!   into `(x, y, z)` with the arithmetic ported in [`LocalFrame::to_local`].
//!
//! The frame is the tangent frame at the centre: **x east, y up, z south**. A point on the
//! sphere of radius `R + alt` is rotated so the centre sits at the origin with y along the
//! local vertical; nothing is flattened, so y falls away from the tangent plane with
//! distance (about 0.08 m at 1 km, 7.8 m at 10 km).
//!
//! How the original *chooses* the centre (`UTL_geoid::exec_shift`, scenery shifts) is not
//! recovered. Callers pick one and say so.

/// Planet radius in metres (`UTL_geoid + 0x08` on Earth).
pub const EARTH_RADIUS_M: f64 = 6_378_145.0;

/// Degrees → radians as the double at oracle `0x1010b3b80`. One ulp-scale step away from
/// `PI / 180`; kept as the original's literal.
pub const DEG_TO_RAD: f64 = 0.017_453_292_519_943_278;

/// A tangent frame centred on one latitude/longitude.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalFrame {
    pub radius_m: f64,
    /// Centre longitude, radians.
    pub lon0: f64,
    /// Centre latitude, radians.
    pub lat0: f64,
    pub sin_lat0: f64,
    pub cos_lat0: f64,
}

impl LocalFrame {
    /// `set_geoid_to_planet_earth` followed by `set_center(lon, lat)`.
    pub fn earth(center_lon_deg: f32, center_lat_deg: f32) -> LocalFrame {
        let lat0 = f64::from(center_lat_deg) * DEG_TO_RAD;
        let lon0 = f64::from(center_lon_deg) * DEG_TO_RAD;
        LocalFrame {
            radius_m: EARTH_RADIUS_M,
            lon0,
            lat0,
            sin_lat0: lat0.sin(),
            cos_lat0: lat0.cos(),
        }
    }

    /// One point, exactly as `build_rwy_layout` computes a runway end: doubles throughout,
    /// narrowed to f32 only at the end, with the original's grouping of the products.
    pub fn to_local(&self, lat_deg: f64, lon_deg: f64, alt_m: f32) -> [f32; 3] {
        let r = self.radius_m;
        let rr = f64::from(alt_m) + r;
        let dlon = DEG_TO_RAD * lon_deg - self.lon0;
        let lat = DEG_TO_RAD * lat_deg;
        let cos_lat = lat.cos();
        // Offsets from the centre in the meridian plane of the centre, before the tilt.
        let toward = cos_lat * dlon.cos() * rr - r * self.cos_lat0;
        let axial = r * self.sin_lat0 - lat.sin() * rr;
        let x = dlon.sin() * cos_lat * rr;
        let y = self.cos_lat0 * toward - self.sin_lat0 * axial;
        let z = axial * self.cos_lat0 + toward * self.sin_lat0;
        [x as f32, y as f32, z as f32]
    }
}

/// What `build_rwy_layout` reads from a runway.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunwayExtent {
    pub lat1: f64,
    pub lon1: f64,
    pub lat2: f64,
    pub lon2: f64,
    /// Full width, metres (`vec_rwy_struct + 0x38`).
    pub width_m: f32,
    /// Extension beyond end 1, metres (`+0x44`).
    pub extend1_m: f32,
    /// Extension beyond end 2, metres (`+0x48`).
    pub extend2_m: f32,
}

/// The four corners of a runway, as `build_rwy_layout` writes them.
///
/// Order is the original's: `[end1 − n, end1 + n, end2 − n, end2 + n]`, where `n` is half
/// the width along `(−dz, dx)` of the unit direction from end 1 to end 2. With
/// `with_extensions` the two ends are pushed outward by `extend1_m`/`extend2_m`; without it
/// they sit on the surveyed ends. All of the corner arithmetic is f32, as in the original.
pub fn runway_layout(
    frame: &LocalFrame,
    rwy: &RunwayExtent,
    with_extensions: bool,
    alt_m: f32,
) -> [[f32; 3]; 4] {
    let [x1, y1, z1] = frame.to_local(rwy.lat1, rwy.lon1, alt_m);
    let [x2, y2, z2] = frame.to_local(rwy.lat2, rwy.lon2, alt_m);
    let dx = x2 - x1;
    let dz = z2 - z1;
    let inv = 1.0_f32 / (dx * dx + dz * dz).sqrt();
    let ux = dx * inv;
    let uz = dz * inv;
    let half = 0.5_f32 * rwy.width_m;
    let (e1, e2) = if with_extensions {
        (rwy.extend1_m, rwy.extend2_m)
    } else {
        (0.0, 0.0)
    };
    let nx = -uz * half;
    let nz = half * ux;
    [
        [x1 - nx - e1 * ux, y1, z1 - nz - e1 * uz],
        [x1 + nx - e1 * ux, y1, z1 + nz - e1 * uz],
        [x2 - nx + e2 * ux, y2, z2 - nz + e2 * uz],
        [x2 + nx + e2 * ux, y2, z2 + nz + e2 * uz],
    ]
}

/// The frame centre the original uses for a position: `(lon, lat)` in degrees.
///
/// MEASURED for one start and generalised by inference. With the aircraft at KSEA
/// (47.431 N, 122.308 W) the oracle's `g_wrl_mv` puts the frame origin 14 455 m west and
/// 48 017–48 034 m south of it, which is `(−122.5, 47.0)`; with that centre the ported
/// transform reproduces the oracle's aircraft position to under a metre. The same run loads
/// DSF tiles for longitudes −124…−121 and latitudes 46…48, a 3 × 2 block, and `(−122.5, 47.0)`
/// is its middle. The rule written here — the middle of the aircraft's longitude tile and the
/// nearest whole latitude — is the simplest one that gives that block; it is not read out of
/// `UTL_geoid::exec_shift`.
pub fn region_center(lat_deg: f64, lon_deg: f64) -> (f32, f32) {
    ((lon_deg.floor() + 0.5) as f32, lat_deg.round() as f32)
}
