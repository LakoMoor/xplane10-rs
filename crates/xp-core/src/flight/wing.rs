//! Finite-wing element build-up: how a wing of the ACF is cut into the strips the blade-element
//! model integrates over.
//!
//! Verified against the "Finite-Wing & Element Build-Up" sections of the oracle's
//! `Cycle Dump.txt` for the Cessna 172SP: every `Element #` line (54 of them over 8 wings,
//! including the propeller blade) and every `Wing area for this wing=` line is reproduced as
//! printed. The rule was derived from those lines and the ACF, then checked on all of them:
//!
//! - a wing has `_els` equal strips along `_semilen_SEG`;
//! - the chord runs linearly from `_Croot` to `_Ctip`; a strip's `MAC` is the mean aerodynamic
//!   chord of its trapezoid, `2/3 · (c1² + c1·c2 + c2²) / (c1 + c2)`;
//! - a strip's area is its mean chord times its span **projected through the sweep**:
//!   `(c1 + c2)/2 · (semilen / els) · cos(sweep_design)` — the fin, swept 38°, prints 0.372 m²
//!   for a strip whose unswept area would be 0.472;
//! - a strip's incidence is `_incidence/<strip>`;
//! - ACF lengths are feet.
//!
//! NOT ported yet, and visible in the same report: wing joining ("I AM ADDING LEFT WING 2 TO THE
//! TIP OF LEFT WING 1"), the joined semi-length / sweep / aspect ratio / taper / washout lines,
//! and Oswald's efficiency. The joined semi-length and sweep are understood (root offset plus
//! spans; quarter-chord line), the printed aspect ratio and the efficiency formula are not, so
//! none of that block is claimed.

use anyhow::{Context, Result};

use crate::acf::Acf;

/// Feet → metres.
pub const FT_TO_M: f32 = 0.3048;

/// One strip of a wing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Element {
    /// Planform area, m².
    pub area_m2: f32,
    /// Mean aerodynamic chord of the strip, m.
    pub mac_m: f32,
    /// Incidence, degrees.
    pub incidence_deg: f32,
}

/// One `_wing/<index>` of an ACF.
#[derive(Debug, Clone, PartialEq)]
pub struct Wing {
    pub index: usize,
    /// Span of this segment, m.
    pub semilen_m: f32,
    pub root_chord_m: f32,
    pub tip_chord_m: f32,
    pub sweep_deg: f32,
    pub dihedral_deg: f32,
    /// The four airfoil files: root low/high Reynolds, tip low/high Reynolds.
    pub airfoils: [String; 4],
    pub elements: Vec<Element>,
}

impl Wing {
    /// Read wing `index`. `Ok(None)` when the ACF has no such wing or it has no span.
    pub fn from_acf(acf: &Acf, index: usize) -> Result<Option<Wing>> {
        let key = |leaf: &str| format!("_wing/{index}/{leaf}");
        let Some(semilen_ft) = acf.f32_or_int(&key("_semilen_SEG")) else {
            return Ok(None);
        };
        if semilen_ft <= 0.0 {
            return Ok(None);
        }
        let num = |leaf: &str| {
            acf.f32_or_int(&key(leaf))
                .with_context(|| format!("ACF has no {}", key(leaf)))
        };
        let els = acf
            .i32(&key("_els"))
            .with_context(|| format!("ACF has no {}", key("_els")))?;
        let els = usize::try_from(els).with_context(|| format!("{} is negative", key("_els")))?;
        let (croot_ft, ctip_ft) = (num("_Croot")?, num("_Ctip")?);
        let sweep_deg = num("_sweep_design")?;
        let afl = |leaf: &str| acf.text(&key(leaf)).unwrap_or("").trim().to_string();

        let strip_span_m = semilen_ft * FT_TO_M / els as f32 * sweep_deg.to_radians().cos();
        let mut elements = Vec::with_capacity(els);
        for i in 0..els {
            let chord_at = |t: f32| (croot_ft + (ctip_ft - croot_ft) * t) * FT_TO_M;
            let c1 = chord_at(i as f32 / els as f32);
            let c2 = chord_at((i as f32 + 1.0) / els as f32);
            // Mean aerodynamic chord of the trapezoidal strip, not its mid-span chord: the
            // tailplane's strip 4 runs from 0.975 m to 0.914 m and prints 0.95, where the
            // mid-span chord 0.9449 would print 0.94.
            let mac_m = 2.0 / 3.0 * (c1 * c1 + c1 * c2 + c2 * c2) / (c1 + c2);
            elements.push(Element {
                area_m2: (c1 + c2) * 0.5 * strip_span_m,
                mac_m,
                incidence_deg: num(&format!("_incidence/{i}"))?,
            });
        }
        Ok(Some(Wing {
            index,
            semilen_m: semilen_ft * FT_TO_M,
            root_chord_m: croot_ft * FT_TO_M,
            tip_chord_m: ctip_ft * FT_TO_M,
            sweep_deg,
            dihedral_deg: num("_dihed_design")?,
            airfoils: [
                afl("_afl_file_R0"),
                afl("_afl_file_R1"),
                afl("_afl_file_T0"),
                afl("_afl_file_T1"),
            ],
            elements,
        }))
    }

    /// Sum of the strips' areas, m² — the report's "Wing area for this wing".
    pub fn area_m2(&self) -> f32 {
        self.elements.iter().map(|e| e.area_m2).sum()
    }

    /// The report's lines for this wing, from the airfoils to the area, as the original prints
    /// them.
    pub fn report_lines(&self) -> Vec<String> {
        let mut out = vec![
            format!("\troot lo Re:  {}.", self.airfoils[0]),
            format!("\troot hi Re:  {}.", self.airfoils[1]),
            format!("\ttip  lo Re:  {}.", self.airfoils[2]),
            format!("\ttip  hi Re:  {}.", self.airfoils[3]),
        ];
        for (i, e) in self.elements.iter().enumerate() {
            out.push(format!(
                "\tElement #{i:2}: S={:8.3} sqr mtrs, MAC={:7.2} mtrs, incidence={:7.2} deg.",
                e.area_m2, e.mac_m, e.incidence_deg
            ));
        }
        out.push(format!("\tWing area for this wing={:.2}.", self.area_m2()));
        out
    }
}
