//! Weight & Balance & Fuel: the aircraft's loading as the original sets it up when the aircraft
//! is loaded (`veh_class::load_veh3`, `fuel_class::fuel_plane`) and the rows `flt_class::
//! wegt_wind` draws for it.
//!
//! * acf fields (`veh_class::IO_pmap`): 0x2238 `_m_fuel_max_tot`, 0x223c `_m_empty`, 0x2240
//!   `_m_jett`, 0x2244 `_m_displaced`, 0x2248 `_m_max`, 0x1e30 `_m_shift`, 0x2300/04/08
//!   `_jato_thrust` / `_jato_dur` / `_jato_sfc`, 0x2220/28 `_cgZ_fwd` / `_cgZ_aft`, 0x3480
//!   `_tank_rat[9]`, 0x4110 the 24 weapon files.
//! * Fuel: each tank `_m_fuel_max_tot × _tank_rat[i] × 0.5` (`veh_class::veh_class` sets the
//!   ratio at +0x42e8 to 0.5).
//! * Slung load (+0x728): `_m_jett × 0.5`; shifted weight (+0x72c): `_m_shift`.
//! * Payload (+0x724): half of what is left under `_m_max` after the fuel capacity, slung and
//!   shifted loads, the empty weight and the weapons — or, when `_m_displaced` exceeds
//!   `_m_max` × 0.5, under `_m_displaced`; never below 0.

use crate::acf::Acf;
use crate::btnwin::{Kind, Widget};

#[derive(Debug, Clone, PartialEq)]
pub struct Loading {
    pub empty: f32,
    pub max: f32,
    pub fuel_max: f32,
    pub displaced: f32,
    pub tank_rat: [f32; 9],
    /// Tank x (lateral) positions: left of the centre line < 0.
    pub tank_x: [f32; 9],
    pub m_shift: f32,
    pub m_jett: f32,
    pub jato: bool,
    pub weapons: bool,
    pub cg_fwd: f32,
    pub cg_aft: f32,
    pub cg_z: f32,
    /// `_average_mac_acf` (+0x22b4) as `veh_class::load_veh2` computes it.
    pub mac: f32,
    pub jett_is_slung: bool,
    pub payload: f32,
    pub slung: f32,
    pub shifted: f32,
    pub tanks: [f32; 9],
}

impl Loading {
    pub fn from_acf(acf: &Acf) -> Loading {
        let f = |k: &str| acf.f32_or_int(&format!("acf/{k}")).unwrap_or(0.0);
        let mut tank_rat = [0.0; 9];
        let mut tank_x = [0.0; 9];
        for i in 0..9 {
            tank_rat[i] = f(&format!("_tank_rat/{i}"));
            tank_x[i] = f(&format!("_tank_xyz/{i},0"));
        }
        let weapons = (0..24).any(|i| {
            acf.text(&format!("_wpna/{i}/_v10_att_file_stl"))
                .is_some_and(|s| !s.trim().is_empty())
        });
        let (empty, max, fuel_max, displaced) =
            (f("_m_empty"), f("_m_max"), f("_m_fuel_max_tot"), f("_m_displaced"));
        let (m_shift, m_jett) = (f("_m_shift"), f("_m_jett"));
        let jato = f("_jato_thrust") * f("_jato_dur") * f("_jato_sfc") > 0.0;
        let slung = m_jett * 0.5;
        let shifted = m_shift;
        // Weapons' mass is not modelled yet: aircraft with ordnance get 0 here.
        let wpn = 0.0;
        let rest = |base: f32| base - fuel_max - shifted - slung - empty - wpn;
        let payload = if f64::from(displaced) <= f64::from(max) * 0.5 {
            rest(max) * 0.5
        } else {
            rest(displaced)
        };
        let mut tanks = [0.0; 9];
        for i in 0..9 {
            tanks[i] = fuel_max * tank_rat[i] * 0.5;
        }
        Loading {
            empty,
            max,
            fuel_max,
            displaced,
            tank_rat,
            tank_x,
            m_shift,
            m_jett,
            jato,
            weapons,
            cg_fwd: f("_cgZ_fwd"),
            cg_aft: f("_cgZ_aft"),
            cg_z: f("_cgZ"),
            mac: mean_chord(acf),
            jett_is_slung: f("_jett_is_slung") != 0.0,
            payload: payload.max(0.0),
            slung,
            shifted,
            tanks,
        }
    }

    /// `fuel_class::veh_fuel_now_tot`: 4 all tanks, 1 left of the centre line, 3 right of it.
    pub fn fuel(&self, side: i32) -> f32 {
        (0..9)
            .filter(|&i| match side {
                1 => self.tank_x[i] < 0.0,
                3 => self.tank_x[i] > 0.0,
                _ => true,
            })
            .map(|i| self.tanks[i])
            .sum()
    }

    /// +0x730: fuel, slung and shifted loads, empty weight, payload and weapons.
    pub fn total(&self) -> f32 {
        self.fuel(4) + self.slung + self.empty + self.shifted + self.payload
    }

    /// Patch the rows `gen_window` recorded for `wegt_wind` (start_win 6) with this aircraft's
    /// loading: slider ranges (0 … 1.1 × `_m_max`; the CG `_cgZ_fwd` − 0.25 × MAC … `_cgZ_aft` +
    /// 0.25 × MAC, both relative to `_cgZ`), palettes (0x49 green; 0x50 red for a CG outside
    /// fwd … aft or a total above `_m_max`; 0x51 for the maximum), the read-only rows (empty,
    /// total, maximum), the rows the original draws only for some aircraft (JATO, shifted and
    /// slung weight, the slung-load size with `_jett_is_slung`, the Ordnance tab with weapons)
    /// and one 200-wide row per tank with `_tank_rat` > 0 at x 120 + 310·(i / 3),
    /// y 391 − 36·(i % 3).
    pub fn apply(&self, widgets: &mut Vec<Widget>) {
        let top = self.max * 1.1;
        let tab0 = widgets
            .iter()
            .find(|w| w.cond.ends_with("wgtW_tab=0"))
            .map(|w| w.cond.clone())
            .unwrap_or_default();
        let (fwd, aft) = (self.cg_fwd - self.cg_z, self.cg_aft - self.cg_z);
        let cg_pal = if fwd <= 0.0 && 0.0 <= aft { "73" } else { "80" };
        let mut out = Vec::with_capacity(widgets.len() + 9);
        for mut w in widgets.drain(..) {
            let ordnance = (w.kind == Kind::Tab && w.a == 1.0) || w.cond.ends_with("wgtW_tab=1");
            if ordnance && !self.weapons {
                continue;
            }
            if w.label.starts_with("slung-load size") && !self.jett_is_slung {
                continue;
            }
            if w.kind != Kind::Slider || !w.cond.ends_with("wgtW_tab=0") {
                out.push(w);
                continue;
            }
            let name = w.label.trim_start_matches('-');
            let (key, pal, edit, fmt, keep) = match name.split('|').next().unwrap_or("") {
                "center of gravity" => ("m:wgt_cg", cg_pal, true, "cg", true),
                "empty weight" => ("m:wgt_empty", "73", false, "int", true),
                "payload weight" => ("m:wgt_payload", "73", true, "int", true),
                "fuel TOTAL" => ("m:wgt_fuel", "73", true, "int", true),
                "fuel LEFT" => ("m:wgt_left", "73", true, "int", true),
                "fuel RIGHT" => ("m:wgt_right", "73", true, "int", true),
                "JATO fuel weight" => ("m:wgt_jato", "73", true, "int", self.jato),
                "shifted weight" => ("m:wgt_shift", "73", true, "int", self.m_shift > 0.0),
                "slung load weight" => ("m:wgt_slung", "73", true, "int", self.m_jett > 0.0),
                "total weight" => ("m:wgt_total", "m:wgt_total_pal", false, "int", true),
                "maximum allowable weight" => ("m:wgt_max", "81", false, "int", true),
                _ => ("", "73", false, "int", true),
            };
            if !keep {
                continue;
            }
            let after = key == "m:wgt_right";
            w.key = key.to_string();
            (w.b, w.c) = if key == "m:wgt_cg" {
                (fwd - 0.25 * self.mac, aft + 0.25 * self.mac)
            } else {
                (0.0, top)
            };
            w.items = vec!["horz".into(), pal.into(), u8::from(edit).to_string(), fmt.into()];
            out.push(w);
            if after {
                for i in (0..9).filter(|&i| self.tank_rat[i] > 0.0) {
                    out.push(Widget {
                        kind: Kind::Slider,
                        x: 120.0 + 310.0 * (i / 3) as f32,
                        y: 391.0 - 36.0 * (i % 3) as f32,
                        a: 200.0,
                        b: 0.0,
                        c: top,
                        d: 0.0,
                        key: format!("m:wgt_tank{i}"),
                        label: "-fuel TANK|(lb)".into(),
                        items: vec!["horz".into(), "73".into(), "1".into(), "int".into()],
                        cond: tab0.clone(),
                    });
                }
            }
        }
        *widgets = out;
    }

    /// The values behind [`Loading::apply`]'s keys and what [`sync`] needs; call [`sync`] after
    /// setting them for the side, total-fuel and total-weight rows.
    pub fn values(&self) -> Vec<(String, f64)> {
        let mut v = vec![
            ("m:wgt_cg".to_string(), 0.0),
            ("m:wgt_empty".into(), f64::from(self.empty)),
            ("m:wgt_payload".into(), f64::from(self.payload)),
            ("m:wgt_jato".into(), 0.0),
            ("m:wgt_shift".into(), f64::from(self.shifted)),
            ("m:wgt_slung".into(), f64::from(self.slung)),
            ("m:wgt_max".into(), f64::from(self.max)),
            ("m:acf_weapons".into(), if self.weapons { 1.0 } else { 0.0 }),
        ];
        for i in 0..9 {
            v.push((format!("m:wgt_tank{i}"), f64::from(self.tanks[i])));
            v.push((format!("m:wgt_rat{i}"), f64::from(self.tank_rat[i])));
            v.push((format!("m:wgt_tx{i}"), f64::from(self.tank_x[i])));
        }
        v
    }
}

/// Recompute the derived rows after `key` changed (`wegt_wind`): a new fuel TOTAL is spread over
/// the tanks by `_tank_rat` (+0x5088 … = ratio × total); LEFT / RIGHT over that side's tanks in
/// the same proportions (approximation: the original's side split is not decoded yet); then the
/// side and total fuel, the total weight (+0x730) and its palette.
pub fn sync(get: &dyn Fn(&str) -> f64, set: &mut dyn FnMut(&str, f64), key: &str) {
    let rat: Vec<f64> = (0..9).map(|i| get(&format!("m:wgt_rat{i}"))).collect();
    let tx: Vec<f64> = (0..9).map(|i| get(&format!("m:wgt_tx{i}"))).collect();
    let side = |i: usize, s: i32| match s {
        1 => tx[i] < 0.0,
        3 => tx[i] > 0.0,
        _ => true,
    };
    let mut tank: Vec<f64> = (0..9).map(|i| get(&format!("m:wgt_tank{i}"))).collect();
    let s = match key {
        "m:wgt_fuel" => 4,
        "m:wgt_left" => 1,
        "m:wgt_right" => 3,
        _ => 0,
    };
    if s != 0 {
        let v = get(key);
        let sum: f64 = (0..9).filter(|&i| side(i, s)).map(|i| rat[i]).sum();
        for i in (0..9).filter(|&i| side(i, s) && rat[i] > 0.0) {
            tank[i] = if s == 4 { v * rat[i] } else { v * rat[i] / sum.max(1e-9) };
            set(&format!("m:wgt_tank{i}"), tank[i]);
        }
    }
    let fuel = |s: i32| (0..9).filter(|&i| side(i, s)).map(|i| tank[i]).sum::<f64>();
    let all = fuel(4);
    set("m:wgt_fuel", all);
    set("m:wgt_left", fuel(1));
    set("m:wgt_right", fuel(3));
    let total = all + get("m:wgt_empty") + get("m:wgt_payload") + get("m:wgt_shift") + get("m:wgt_slung");
    set("m:wgt_total", total);
    set("m:wgt_total_pal", if total <= get("m:wgt_max") { 73.0 } else { 80.0 });
}

/// `veh_class::load_veh2`'s `_average_mac_acf`: the mean, over every element of the surfaces
/// 8 … 55 (0 … 7 are the propeller discs), of the element's mean aerodynamic chord
/// 2/3 · (c₁ + c₂ − c₁c₂ / (c₁ + c₂)) from the chords at its two ends. The element-end chords
/// run linearly from `_Croot` to `_Ctip` over equal spans (approximation of the element layout:
/// it reproduces the C172's CG-slider thumb at x 447 on the oracle frame).
fn mean_chord(acf: &Acf) -> f32 {
    let (mut sum, mut n) = (0.0f64, 0.0f64);
    for w in 8..56 {
        let f = |k: &str| acf.f32_or_int(&format!("_wing/{w}/{k}")).unwrap_or(0.0) as f64;
        let els = f("_els") as i32;
        if els <= 0 || f("_semilen_SEG") <= 0.0 {
            continue;
        }
        let (cr, ct) = (f("_Croot"), f("_Ctip"));
        for i in 0..els {
            let a = cr + (ct - cr) * f64::from(i) / f64::from(els);
            let b = cr + (ct - cr) * f64::from(i + 1) / f64::from(els);
            if a + b > 0.0 {
                sum += 2.0 * (a + b - a * b / (a + b)) / 3.0;
                n += 1.0;
            }
        }
    }
    if n > 0.0 { (sum / n) as f32 } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c172_default_load() {
        let acf = Acf::parse(
            "I\n1100 version\nACF\nPROPERTIES_BEGIN\nP acf/_m_empty 1721.0\n\
             P acf/_m_max 2558.0\nP acf/_m_fuel_max_tot 350.0\nP acf/_m_displaced 0.0\n\
             P acf/_tank_rat/0 0.5\nP acf/_tank_rat/1 0.5\nP acf/_tank_xyz/0,0 -2.0\n\
             P acf/_tank_xyz/1,0 2.0\nP acf/_cgZ 3.3\nP acf/_cgZ_fwd 3.0\nP acf/_cgZ_aft 4.0\n\
             PROPERTIES_END\n",
        )
        .unwrap();
        let l = Loading::from_acf(&acf);
        assert_eq!(l.payload, 243.5);
        assert_eq!(l.fuel(4), 175.0);
        assert_eq!(l.fuel(1), 87.5);
        assert_eq!(l.total(), 2139.5);
        // The oracle frame (after about a minute of engine running): 243, 174, 87, 87, 2139.
        let mut w = Vec::new();
        l.apply(&mut w);
        assert!(w.is_empty());
    }
}
