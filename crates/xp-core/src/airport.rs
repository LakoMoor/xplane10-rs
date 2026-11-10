//! Airport geometry: apt.dat runways → renderable buffers.
//!
//! The corner positions are the original's ([`crate::geoid::runway_layout`], ported from
//! `build_rwy_layout`). Everything about how the surface is *textured* is not: the
//! original cuts a runway into threshold, touchdown-zone, fixed-distance and centreline
//! segments from `Resources/bitmaps/runways/<surface>/rway_*.dds`, and that layout code is
//! not recovered. This module draws each runway as one quad of plain pavement and says so
//! in its draw list; it is real shipped geometry under a placeholder material.

use anyhow::{bail, Context, Result};

use crate::aptdat::{Airport, Record};
use crate::geoid::{runway_layout, LocalFrame, RunwayExtent};
use crate::obj8::TextureKind;
use crate::scene::{DrawCall, Mesh, TextureSlot, Topology, DEFAULT_DIFFUSE, VERTEX_STRIDE};

/// Feet → metres for the airport elevation in the `1` header.
pub const FT_TO_M: f32 = 0.3048;

/// One `100` land runway.
#[derive(Debug, Clone, PartialEq)]
pub struct Runway {
    pub name1: String,
    pub name2: String,
    pub extent: RunwayExtent,
    /// apt.dat surface code (field 2).
    pub surface: u32,
    /// Displaced thresholds, metres (fields 11 and 20). Parsed, not yet used.
    pub displaced1_m: f32,
    pub displaced2_m: f32,
}

fn f64_field(r: &Record, i: usize, what: &str) -> Result<f64> {
    r.f64(i)
        .with_context(|| format!("runway record has no {what} (field {i}): {:?}", r.raw))
}

/// Decode the `100` records of one airport.
///
/// Field positions are the ones `aptdat::Runway` already verified against KSEA's datum
/// (designators at 8/17, positions at 9–10/18–19). The two lengths after each position
/// are the displaced threshold and the blast pad; **the blast pad is taken to be the
/// `+0x44`/`+0x48` extension `build_rwy_layout` adds beyond each end**, because that is the
/// only per-end length in the record that lies outside the surveyed ends. That pairing is
/// an inference from the record layout, not a traced store.
pub fn runways(apt: &Airport) -> Result<Vec<Runway>> {
    let mut out = Vec::new();
    for rw in &apt.runways {
        let r = &rw.record;
        let surface = r
            .field(2)
            .and_then(|t| t.parse::<u32>().ok())
            .with_context(|| format!("runway record has no surface code: {:?}", r.raw))?;
        out.push(Runway {
            name1: r.field(8).unwrap_or("").to_string(),
            name2: r.field(17).unwrap_or("").to_string(),
            extent: RunwayExtent {
                lat1: f64_field(r, 9, "end 1 latitude")?,
                lon1: f64_field(r, 10, "end 1 longitude")?,
                lat2: f64_field(r, 18, "end 2 latitude")?,
                lon2: f64_field(r, 19, "end 2 longitude")?,
                width_m: f64_field(r, 1, "width")? as f32,
                extend1_m: f64_field(r, 12, "end 1 blast pad")? as f32,
                extend2_m: f64_field(r, 21, "end 2 blast pad")? as f32,
            },
            surface,
            displaced1_m: f64_field(r, 11, "end 1 displaced threshold")? as f32,
            displaced2_m: f64_field(r, 20, "end 2 displaced threshold")? as f32,
        });
    }
    Ok(out)
}

/// Airport elevation in metres, from field 1 of the `1` header (feet MSL).
pub fn elevation_m(apt: &Airport) -> Result<f32> {
    let ft = apt
        .header
        .f64(1)
        .with_context(|| format!("airport header has no elevation: {:?}", apt.header.raw))?;
    Ok(ft as f32 * FT_TO_M)
}

/// The pavement texture for a surface code, relative to `Resources/bitmaps/runways/`.
///
/// INFERRED from the directory names, which spell the apt.dat surface names; only codes 1
/// and 2 are exercised by a frame. A code with no directory is `None`, not a guess.
pub fn pavement_texture(surface: u32) -> Option<&'static str> {
    Some(match surface {
        1 => "asphalt/rway_pave.dds",
        2 => "concrete/rway_pave.dds",
        3 => "goodgrass/rway_pave.dds",
        4 => "roughdirt/rway_pave.dds",
        5 => "gravel/rway_pave.dds",
        12 => "drylakebed/rway_pave.dds",
        14 => "snow/rway_pave.dds",
        _ => return None,
    })
}

/// Where an aircraft stands, in a [`LocalFrame`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StartPose {
    /// The point on the ground under the aircraft's model origin.
    pub position: [f32; 3],
    /// Unit vector the nose points along, in the frame's x/z plane.
    pub forward: [f32; 2],
    /// Model-space y of the ground plane (the lowest point of the gear).
    pub ground_y: f32,
}

impl StartPose {
    /// On a runway end, pointing down the runway toward the other end.
    ///
    /// PLACEHOLDER for the original's start position. The oracle logs
    /// `Init dat_p0 type:loc_tak apt:KSEA rwy:34R`; the routine that turns that into a
    /// position (`start_loc_spec::update_loc_spec_for_type_start`, oracle `0x1002003a0`,
    /// 29 KB) is not ported, so the aircraft is put on the surveyed end itself.
    pub fn on_runway_end(
        frame: &LocalFrame,
        rwy: &Runway,
        end_name: &str,
        alt_m: f32,
        ground_y: f32,
    ) -> Result<StartPose> {
        let e = &rwy.extent;
        let p1 = frame.to_local(e.lat1, e.lon1, alt_m);
        let p2 = frame.to_local(e.lat2, e.lon2, alt_m);
        let (from, to) = if rwy.name1 == end_name {
            (p1, p2)
        } else if rwy.name2 == end_name {
            (p2, p1)
        } else {
            bail!("runway {}/{} has no end {end_name:?}", rwy.name1, rwy.name2);
        };
        let dx = to[0] - from[0];
        let dz = to[2] - from[2];
        let len = (dx * dx + dz * dz).sqrt();
        if len <= 0.0 || !len.is_finite() {
            bail!("runway {}/{} has zero length", rwy.name1, rwy.name2);
        }
        Ok(StartPose {
            position: from,
            forward: [dx / len, dz / len],
            ground_y,
        })
    }

    /// A frame point expressed in the aircraft's OBJ8 model space.
    ///
    /// Model space is lateral x, vertical y, longitudinal z with the nose toward −z
    /// (`docs/COMPATIBILITY.md` row 40). Taking +x as the aircraft's right is the
    /// right-handed completion of that; which side is which is not measured, and the
    /// shipped Cessna is symmetric enough that a frame cannot tell.
    pub fn to_model(&self, p: [f32; 3]) -> [f32; 3] {
        let [fx, fz] = self.forward;
        let d = [
            p[0] - self.position[0],
            p[1] - self.position[1],
            p[2] - self.position[2],
        ];
        // model x axis = up × (−forward) = (−fz, 0, fx); model z axis = −forward.
        let x = d[0] * -fz + d[2] * fx;
        let z = -(d[0] * fx + d[2] * fz);
        [x, d[1] + self.ground_y, z]
    }
}

fn push_vertex(bytes: &mut Vec<u8>, pos: [f32; 3], normal: [f32; 3], uv: [f32; 2]) {
    for v in pos
        .iter()
        .chain(normal.iter())
        .chain(uv.iter())
        .chain(DEFAULT_DIFFUSE.iter())
    {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
}

/// One quad per runway, in apt.dat order, carried into the aircraft's model space by
/// `to_model` ([`StartPose::to_model`], or an observed pose from [`crate::oracle::Probe`]).
/// `up` is the frame's vertical in model space and becomes the pavement's normal.
///
/// The texture slots are relative to `Resources/bitmaps/runways/`. UVs span the width
/// once and repeat every width along the length — a placeholder that keeps the pavement
/// texels square; the original's segment layout is not ported.
pub fn build_runway_mesh(
    frame: &LocalFrame,
    runways: &[Runway],
    alt_m: f32,
    to_model: &dyn Fn([f32; 3]) -> [f32; 3],
    up: [f32; 3],
) -> Result<Mesh> {
    let mut vertex_bytes: Vec<u8> = Vec::with_capacity(runways.len() * 4 * VERTEX_STRIDE);
    let mut indices: Vec<u32> = Vec::with_capacity(runways.len() * 6);
    let mut draws: Vec<DrawCall> = Vec::with_capacity(runways.len());
    let mut textures: Vec<TextureSlot> = Vec::new();
    for (n, rwy) in runways.iter().enumerate() {
        let corners = runway_layout(frame, &rwy.extent, true, alt_m);
        let along = {
            let dx = corners[2][0] - corners[0][0];
            let dz = corners[2][2] - corners[0][2];
            (dx * dx + dz * dz).sqrt()
        };
        if rwy.extent.width_m <= 0.0 || !along.is_finite() {
            bail!("runway {}/{} has no area", rwy.name1, rwy.name2);
        }
        let repeats = along / rwy.extent.width_m;
        let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, repeats], [1.0, repeats]];

        let texture = pavement_texture(rwy.surface).map(|path| {
            textures
                .iter()
                .position(|t| t.path == path)
                .unwrap_or_else(|| {
                    textures.push(TextureSlot {
                        kind: TextureKind::Diffuse,
                        path: path.to_string(),
                    });
                    textures.len() - 1
                })
        });

        let vertex_offset = vertex_bytes.len() / VERTEX_STRIDE;
        for (corner, uv) in corners.iter().zip(uvs) {
            push_vertex(&mut vertex_bytes, to_model(*corner), up, uv);
        }
        let index_offset = indices.len();
        let base = u32::try_from(vertex_offset).context("runway vertex index overflows u32")?;
        indices.extend([base, base + 1, base + 2, base + 2, base + 1, base + 3]);
        draws.push(DrawCall {
            declared_index_offset: index_offset,
            index_count: 6,
            index_offset,
            vertex_offset,
            vertex_count: 4,
            topology: Topology::Triangles,
            texture,
            color: DEFAULT_DIFFUSE,
            tinted: false,
            // There is no OBJ8 source line; the runway's ordinal in the airport stands in.
            source_line: n,
            guards: vec![format!("runway {}/{}", rwy.name1, rwy.name2)],
        });
    }

    Ok(Mesh {
        vertex_bytes,
        indices,
        draws,
        textures,
    })
}

fn header_id(line: &[u8]) -> Option<&[u8]> {
    let mut fields = line
        .split(|b| b.is_ascii_whitespace())
        .filter(|f| !f.is_empty());
    match fields.next()? {
        b"1" | b"16" | b"17" => fields.nth(3),
        _ => None,
    }
}

/// Cut one airport's records out of an apt.dat, keeping the two file header lines.
///
/// The default `apt.dat` is 178 MB; parsing all of it to look at one airport costs gigabytes
/// in [`crate::aptdat::AptDat`]. The block runs from the airport's `1`/`16`/`17` header to
/// the next header or the closing `99`.
pub fn extract_airport_text(bytes: &[u8], id: &str) -> Option<String> {
    let mut lines = bytes.split(|b| *b == b'\n');
    let marker = lines.next()?;
    let version = lines.next()?;
    let mut block: Vec<u8> = Vec::new();
    let mut inside = false;
    for line in lines {
        let first = line
            .split(|b| b.is_ascii_whitespace())
            .find(|f| !f.is_empty());
        if let Some(found) = header_id(line) {
            if inside {
                break;
            }
            inside = found == id.as_bytes();
        } else if inside && first == Some(b"99".as_slice()) {
            break;
        }
        if inside {
            block.extend_from_slice(line);
            block.push(b'\n');
        }
    }
    if block.is_empty() {
        return None;
    }
    let mut text: Vec<u8> = Vec::with_capacity(block.len() + 64);
    text.extend_from_slice(marker);
    text.push(b'\n');
    text.extend_from_slice(version);
    text.push(b'\n');
    text.extend_from_slice(&block);
    Some(String::from_utf8_lossy(&text).into_owned())
}

/// Find an airport by identifier in the install, in scenery-pack priority order.
///
/// Returns the file it came from as well. On the pinned install `KSEA` resolves to the
/// `KSEA Demo Area` pack: `Global Airports` has higher priority but does not contain it. The
/// first-pack-wins direction is the same inference [`crate::gpsdb::world_airports`] makes.
pub fn find_in_install(
    install: &std::path::Path,
    id: &str,
) -> Result<(std::path::PathBuf, Airport)> {
    for file in crate::gpsdb::earth_apt_files(install)? {
        let bytes =
            std::fs::read(&file).with_context(|| format!("reading apt.dat {}", file.display()))?;
        let Some(text) = extract_airport_text(&bytes, id) else {
            continue;
        };
        let parsed = crate::aptdat::AptDat::parse(&text)
            .with_context(|| format!("parsing airport {id} from {}", file.display()))?;
        let Some(apt) = parsed.airports.into_iter().next() else {
            bail!(
                "{}: airport {id} is not a land airport (`1` header); seaplane bases and heliports are not modelled",
                file.display()
            );
        };
        return Ok((file, apt));
    }
    bail!("no scenery pack has an airport {id:?}")
}
