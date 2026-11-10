//! State read out of the running original by the in-process probe
//! (`tests/original_patch/stub/probe.c`).
//!
//! The probe samples three globals that `ZOOM_publish_matrices` (oracle `0x10094e1d0`)
//! refreshes every frame: `g_proj`, `g_wrl_mv` (world → eye) and `g_acf_mv` (aircraft → eye),
//! each sixteen f32 in OpenGL column-major order. A probe file taken beside a screenshot is
//! therefore that frame's exact camera and the aircraft's exact pose — observations of the
//! original, used here as test inputs, never as constants of the port.

use anyhow::{bail, Context, Result};

/// One probe sample.
#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    pub proj: [f32; 16],
    pub wrl_mv: [f32; 16],
    pub acf_mv: [f32; 16],
    /// The `_skyc` block (oracle `0x102b7c540`, 96 f32), when the probe recorded it.
    pub skyc: Option<Vec<f32>>,
}

fn parse_matrix(text: &str, name: &str) -> Result<[f32; 16]> {
    let key = format!("{name}.bits");
    let line = text
        .lines()
        .find(|l| l.split_whitespace().next() == Some(key.as_str()))
        .with_context(|| format!("probe has no {key} line"))?;
    let mut out = [0.0_f32; 16];
    let mut n = 0;
    for tok in line.split_whitespace().skip(1) {
        if n == 16 {
            bail!("{key}: more than 16 values");
        }
        let bits =
            u32::from_str_radix(tok, 16).with_context(|| format!("{key}: bad word {tok:?}"))?;
        out[n] = f32::from_bits(bits);
        n += 1;
    }
    if n != 16 {
        bail!("{key}: {n} values, want 16");
    }
    Ok(out)
}

fn parse_floats(text: &str, name: &str) -> Result<Option<Vec<f32>>> {
    let Some(line) = text
        .lines()
        .find(|l| l.split_whitespace().next() == Some(name))
    else {
        return Ok(None);
    };
    let mut out = Vec::new();
    for tok in line.split_whitespace().skip(1) {
        out.push(
            tok.parse::<f32>()
                .with_context(|| format!("{name}: bad value {tok:?}"))?,
        );
    }
    Ok(Some(out))
}

/// The sun as the original's sky model holds it for one frame.
///
/// Field positions are the binary's own: its static initialiser registers `skyc/sun_dir_r`
/// at `_skyc + 0x4c`, `skyc/sun_amb_r` at `+0x2c` (g and b follow), and the public datarefs
/// `sim/graphics/scenery/sun_heading_degrees` / `sun_pitch_degrees` read `+0x168` / `+0x170`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sun {
    /// `skyc/sun_dir_*`: colour of the direct light.
    pub direct_rgb: [f32; 3],
    /// `skyc/sun_amb_*`: colour of the ambient light.
    pub ambient_rgb: [f32; 3],
    pub heading_deg: f32,
    pub pitch_deg: f32,
}

impl Sun {
    /// Unit vector toward the sun in the local frame (x east, y up, z south).
    ///
    /// The frame's axes are measured; reading the heading as degrees clockwise from north is
    /// the dataref's documented meaning and has not been cross-checked against a shadow yet.
    pub fn direction_world(&self) -> [f32; 3] {
        let h = self.heading_deg.to_radians();
        let p = self.pitch_deg.to_radians();
        [h.sin() * p.cos(), p.sin(), -h.cos() * p.cos()]
    }
}

impl Probe {
    /// The sun of this sample, if the probe recorded the sky block.
    pub fn sun(&self) -> Option<Sun> {
        let s = self.skyc.as_ref()?;
        if s.len() < 93 {
            return None;
        }
        Some(Sun {
            direct_rgb: [s[19], s[20], s[21]],
            ambient_rgb: [s[11], s[12], s[13]],
            heading_deg: s[90],
            pitch_deg: s[92],
        })
    }

    /// A direction of the local frame, in aircraft model space.
    ///
    /// Rotations only. Going through [`Probe::world_to_model`] and subtracting two points would
    /// difference numbers near 50 000 in f32 and lose the third decimal of a unit vector.
    pub fn world_dir_to_model(&self, d: [f32; 3]) -> [f32; 3] {
        let (w, a) = (&self.wrl_mv, &self.acf_mv);
        let d = [f64::from(d[0]), f64::from(d[1]), f64::from(d[2])];
        let f = |m: &[f32; 16], i: usize| f64::from(m[i]);
        let eye = [
            f(w, 0) * d[0] + f(w, 4) * d[1] + f(w, 8) * d[2],
            f(w, 1) * d[0] + f(w, 5) * d[1] + f(w, 9) * d[2],
            f(w, 2) * d[0] + f(w, 6) * d[1] + f(w, 10) * d[2],
        ];
        [
            (f(a, 0) * eye[0] + f(a, 1) * eye[1] + f(a, 2) * eye[2]) as f32,
            (f(a, 4) * eye[0] + f(a, 5) * eye[1] + f(a, 6) * eye[2]) as f32,
            (f(a, 8) * eye[0] + f(a, 9) * eye[1] + f(a, 10) * eye[2]) as f32,
        ]
    }

    /// Parse a probe file. The `.bits` lines are read, so values round-trip exactly.
    pub fn parse(text: &str) -> Result<Probe> {
        Ok(Probe {
            proj: parse_matrix(text, "g_proj")?,
            wrl_mv: parse_matrix(text, "g_wrl_mv")?,
            acf_mv: parse_matrix(text, "g_acf_mv")?,
            skyc: parse_floats(text, "skyc")?,
        })
    }

    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Probe> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading probe {}", path.display()))?;
        Probe::parse(&text).with_context(|| format!("parsing probe {}", path.display()))
    }

    /// `(near, far)` of the GL frustum in `g_proj`.
    pub fn near_far(&self) -> (f32, f32) {
        let a = f64::from(self.proj[10]);
        let b = f64::from(self.proj[14]);
        ((b / (a - 1.0)) as f32, (b / (a + 1.0)) as f32)
    }

    /// Vertical field of view in radians (`proj[5]` is `1 / tan(fov_y / 2)`).
    pub fn fov_y_rad(&self) -> f32 {
        2.0 * (1.0 / self.proj[5]).atan()
    }

    /// Width over height of the frustum.
    pub fn aspect(&self) -> f32 {
        self.proj[5] / self.proj[0]
    }

    /// The eye in aircraft model space, assuming `g_acf_mv` is a rigid transform.
    pub fn eye_in_model(&self) -> [f32; 3] {
        let m = &self.acf_mv;
        let t = [m[12], m[13], m[14]];
        // p_eye = R p + t  ⇒  eye = −Rᵀ t; column k of R is m[4k..4k+3].
        [
            -(m[0] * t[0] + m[1] * t[1] + m[2] * t[2]),
            -(m[4] * t[0] + m[5] * t[1] + m[6] * t[2]),
            -(m[8] * t[0] + m[9] * t[1] + m[10] * t[2]),
        ]
    }

    fn apply(m: &[f32; 16], p: [f64; 3]) -> [f64; 3] {
        let f = |i: usize| f64::from(m[i]);
        [
            f(0) * p[0] + f(4) * p[1] + f(8) * p[2] + f(12),
            f(1) * p[0] + f(5) * p[1] + f(9) * p[2] + f(13),
            f(2) * p[0] + f(6) * p[1] + f(10) * p[2] + f(14),
        ]
    }

    fn unapply(m: &[f32; 16], e: [f64; 3]) -> [f64; 3] {
        let f = |i: usize| f64::from(m[i]);
        let d = [e[0] - f(12), e[1] - f(13), e[2] - f(14)];
        [
            f(0) * d[0] + f(1) * d[1] + f(2) * d[2],
            f(4) * d[0] + f(5) * d[1] + f(6) * d[2],
            f(8) * d[0] + f(9) * d[1] + f(10) * d[2],
        ]
    }

    /// A point of the original's local frame, in aircraft model space.
    ///
    /// World coordinates are tens of kilometres from the frame centre, so the two transforms
    /// are composed in f64 and narrowed once.
    pub fn world_to_model(&self, p: [f32; 3]) -> [f32; 3] {
        let eye = Self::apply(
            &self.wrl_mv,
            [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])],
        );
        let m = Self::unapply(&self.acf_mv, eye);
        [m[0] as f32, m[1] as f32, m[2] as f32]
    }

    /// A point of aircraft model space, in the original's local frame.
    pub fn model_to_world(&self, p: [f32; 3]) -> [f64; 3] {
        let eye = Self::apply(
            &self.acf_mv,
            [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])],
        );
        Self::unapply(&self.wrl_mv, eye)
    }
}
