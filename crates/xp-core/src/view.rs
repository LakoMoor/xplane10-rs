//! The View menu's camera modes and the `sim/general/*` camera commands.
//!
//! Everything is in the aircraft's OBJ8 model space (x right, y up, nose toward −z), the space
//! the port draws the scene in. A view is an aircraft→eye matrix in OpenGL column-major order,
//! like the oracle's `g_acf_mv`.
//!
//! Status: the two views captured from the oracle (Forwards with Panel and Chase) reproduce the
//! captured matrices exactly at their default state. The other modes' eye points and the step
//! sizes of the camera commands are PLACEHOLDERS chosen to behave like the original (orbit,
//! spot, tower, runway, free camera), not values read from it.

pub type Mat = [f32; 16];

pub const IDENTITY: Mat = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// `a · b` for column-major matrices.
pub fn mul(a: &Mat, b: &Mat) -> Mat {
    let mut out = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            out[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
        }
    }
    out
}

pub fn translate(t: [f32; 3]) -> Mat {
    let mut m = IDENTITY;
    m[12] = t[0];
    m[13] = t[1];
    m[14] = t[2];
    m
}

/// Rotation about +x by `deg` (right-handed).
pub fn rot_x(deg: f32) -> Mat {
    let (s, c) = deg.to_radians().sin_cos();
    let mut m = IDENTITY;
    m[5] = c;
    m[6] = s;
    m[9] = -s;
    m[10] = c;
    m
}

/// Rotation about +y by `deg` (right-handed).
pub fn rot_y(deg: f32) -> Mat {
    let (s, c) = deg.to_radians().sin_cos();
    let mut m = IDENTITY;
    m[0] = c;
    m[2] = -s;
    m[8] = s;
    m[10] = c;
    m
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}

/// A view looking from `eye` at `target` with `up`, looking along eye −z.
pub fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> Mat {
    let f = norm(sub(target, eye));
    let r = norm(cross(f, up));
    let u = cross(r, f);
    [
        r[0],
        u[0],
        -f[0],
        0.0,
        r[1],
        u[1],
        -f[1],
        0.0,
        r[2],
        u[2],
        -f[2],
        0.0,
        -dot(r, eye),
        -dot(u, eye),
        dot(f, eye),
        1.0,
    ]
}

/// The eye point of a rigid view matrix, in model space.
pub fn eye_of(m: &Mat) -> [f32; 3] {
    let t = [m[12], m[13], m[14]];
    [
        -(m[0] * t[0] + m[1] * t[1] + m[2] * t[2]),
        -(m[4] * t[0] + m[5] * t[1] + m[6] * t[2]),
        -(m[8] * t[0] + m[9] * t[1] + m[10] * t[2]),
    ]
}

/// The View menu's modes, in menu order (items 1800–1815).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    ForwardPanel,
    ForwardHud,
    ForwardNothing,
    Cockpit3d,
    Ridealong,
    Chase,
    Circle,
    StillSpot,
    LinearSpot,
    Tower,
    Runway,
    TrackWeapon,
    FreeCamera,
}

impl Mode {
    /// The `sim/view/*` command that selects the mode.
    pub fn from_command(cmd: &str) -> Option<Mode> {
        Some(match cmd {
            "sim/view/forward_with_panel" => Mode::ForwardPanel,
            "sim/view/forward_with_hud" => Mode::ForwardHud,
            "sim/view/forward_with_nothing" => Mode::ForwardNothing,
            "sim/view/3d_cockpit_cmnd_look" | "sim/view/3d_cockpit_toggle" => Mode::Cockpit3d,
            "sim/view/ridealong" => Mode::Ridealong,
            "sim/view/chase" => Mode::Chase,
            "sim/view/circle" => Mode::Circle,
            "sim/view/still_spot" => Mode::StillSpot,
            "sim/view/linear_spot" => Mode::LinearSpot,
            "sim/view/tower" => Mode::Tower,
            "sim/view/runway" => Mode::Runway,
            "sim/view/track_weapon" => Mode::TrackWeapon,
            "sim/view/free_camera" => Mode::FreeCamera,
            _ => return None,
        })
    }

    /// The menu item that carries the mode's check mark.
    pub fn menu_item(self) -> i32 {
        match self {
            Mode::ForwardPanel => 1800,
            Mode::ForwardHud => 1801,
            Mode::ForwardNothing => 1802,
            Mode::Cockpit3d => 1804,
            Mode::Ridealong => 1806,
            Mode::Chase => 1807,
            Mode::Circle => 1808,
            Mode::StillSpot => 1809,
            Mode::LinearSpot => 1810,
            Mode::Tower => 1812,
            Mode::Runway => 1813,
            Mode::TrackWeapon => 1814,
            Mode::FreeCamera => 1815,
        }
    }

    /// The 2-D forward views: the aircraft's own exterior is not drawn.
    pub fn is_forward(self) -> bool {
        matches!(
            self,
            Mode::ForwardPanel | Mode::ForwardHud | Mode::ForwardNothing
        )
    }

    fn orbits(self) -> bool {
        matches!(self, Mode::Chase | Mode::Circle | Mode::TrackWeapon)
    }
}

/// The camera state the View menu and the `sim/general/*` commands act on.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub mode: Mode,
    /// Oracle cockpit view (`g_acf_mv` of the Forwards-with-Panel capture).
    cockpit: Mat,
    /// Oracle chase distance, from the chase capture's translation.
    chase_dist: f32,
    /// Look direction offsets, degrees: heading right-positive, pitch up-positive.
    pub pan: [f32; 2],
    /// Orbit angles of the chase/circle views, degrees.
    pub orbit: [f32; 2],
    pub dist: f32,
    /// Head offset in the cockpit views and eye of the free camera, model metres.
    pub offset: [f32; 3],
    /// Projection zoom factor; 1 is the original's default field of view.
    pub zoom: f32,
}

/// Step of a translate command in the cockpit (m), outside (m), and of a rotate command (°).
const HEAD_STEP: f32 = 0.05;
const MOVE_STEP: f32 = 2.0;
const TURN_STEP: f32 = 5.0;
const ZOOM_STEP: f32 = 1.1;

impl View {
    /// `cockpit` and `chase` are the oracle's `g_acf_mv` for the two captured views.
    pub fn new(cockpit: Mat, chase: &Mat) -> View {
        View {
            mode: Mode::ForwardPanel,
            cockpit,
            chase_dist: -chase[14],
            pan: [0.0; 2],
            orbit: [0.0; 2],
            dist: -chase[14],
            offset: [0.0; 3],
            zoom: 1.0,
        }
    }

    /// Switch mode, resetting the per-mode state as selecting a view in the original does.
    pub fn select(&mut self, mode: Mode) {
        let from = self.matrix();
        self.mode = mode;
        self.pan = [0.0; 2];
        self.zoom = 1.0;
        self.offset = [0.0; 3];
        match mode {
            Mode::Chase | Mode::TrackWeapon => {
                self.orbit = [0.0; 2];
                self.dist = self.chase_dist;
            }
            Mode::Circle => {
                self.orbit = [-35.0, 12.0];
                self.dist = 25.0;
            }
            // The free camera starts where the previous view's eye was.
            Mode::FreeCamera => self.offset = eye_of(&from),
            _ => {}
        }
    }

    /// Apply a `sim/general/*` command. Returns false for other commands.
    pub fn command(&mut self, cmd: &str) -> bool {
        let Some(what) = cmd.strip_prefix("sim/general/") else {
            return false;
        };
        let orbit = self.mode.orbits();
        let step = if self.mode.is_forward() || self.mode == Mode::Cockpit3d {
            HEAD_STEP
        } else {
            MOVE_STEP
        };
        // Translations are along the eye's own axes in the free camera, the model's otherwise.
        let (h, p) = (self.pan[0].to_radians(), self.pan[1].to_radians());
        let fwd = [h.sin() * p.cos(), p.sin(), -h.cos() * p.cos()];
        let right = [h.cos(), 0.0, h.sin()];
        let mut mv = |d: [f32; 3], k: f32| {
            for (o, v) in self.offset.iter_mut().zip(d) {
                *o += v * k;
            }
        };
        match what {
            "left" if orbit => self.orbit[0] -= TURN_STEP,
            "right" if orbit => self.orbit[0] += TURN_STEP,
            "up" if orbit => self.orbit[1] = (self.orbit[1] + TURN_STEP).min(89.0),
            "down" if orbit => self.orbit[1] = (self.orbit[1] - TURN_STEP).max(-89.0),
            "forward" if orbit => self.dist = (self.dist - MOVE_STEP).max(3.0),
            "backward" if orbit => self.dist += MOVE_STEP,
            "left" => mv(right, -step),
            "right" => mv(right, step),
            "up" => mv([0.0, 1.0, 0.0], step),
            "down" => mv([0.0, 1.0, 0.0], -step),
            "forward" => mv(fwd, step),
            "backward" => mv(fwd, -step),
            "rot_left" => self.pan[0] -= TURN_STEP,
            "rot_right" => self.pan[0] += TURN_STEP,
            "rot_up" => self.pan[1] = (self.pan[1] + TURN_STEP).min(89.0),
            "rot_down" => self.pan[1] = (self.pan[1] - TURN_STEP).max(-89.0),
            "zoom_in" => self.zoom = (self.zoom * ZOOM_STEP).min(20.0),
            "zoom_out" => self.zoom = (self.zoom / ZOOM_STEP).max(0.25),
            _ => return false,
        }
        true
    }

    /// The aircraft→eye matrix of the current state.
    pub fn matrix(&self) -> Mat {
        let up = [0.0, 1.0, 0.0];
        // Look offsets act in eye space, after the mode's own placement.
        let pan = mul(&rot_x(-self.pan[1]), &rot_y(self.pan[0]));
        let base = match self.mode {
            Mode::ForwardPanel | Mode::ForwardHud | Mode::ForwardNothing | Mode::Cockpit3d => {
                mul(&self.cockpit, &translate(neg(self.offset)))
            }
            Mode::Chase | Mode::Circle | Mode::TrackWeapon => mul(
                &translate([0.0, 0.0, -self.dist]),
                &mul(&rot_x(self.orbit[1]), &rot_y(-self.orbit[0])),
            ),
            Mode::Ridealong => translate(neg(add([-7.0, 0.8, 1.5], self.offset))),
            Mode::StillSpot => look_at(add([-15.0, 3.0, -40.0], self.offset), [0.0; 3], up),
            Mode::LinearSpot => look_at(add([30.0, 6.0, -80.0], self.offset), [0.0; 3], up),
            Mode::Tower => look_at(add([-120.0, 30.0, -200.0], self.offset), [0.0; 3], up),
            Mode::Runway => look_at(add([0.0, 0.3, -80.0], self.offset), [0.0; 3], up),
            Mode::FreeCamera => return mul(&pan, &translate(neg(self.offset))),
        };
        mul(&pan, &base)
    }

    /// A GL projection captured on the oracle's 1024×768 screen, for a screen of `aspect`
    /// (width / height), scaled by the zoom factor. The horizontal field of view is kept, as the
    /// original's "lateral field of view" setting is: a wider screen sees less vertically.
    pub fn projection(&self, proj: &Mat, aspect: f32) -> Mat {
        let mut p = *proj;
        let k = aspect / (4.0 / 3.0);
        p[1] *= k;
        p[5] *= k;
        for i in [0, 1, 4, 5] {
            p[i] *= self.zoom;
        }
        p
    }
}

fn neg(a: [f32; 3]) -> [f32; 3] {
    [-a[0], -a[1], -a[2]]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &Mat, b: &Mat) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn captured_views_reproduce_the_oracle() {
        let cockpit = mul(&translate([0.1, -0.5, 0.2]), &rot_x(-3.0));
        let chase = translate([0.0, 0.0, -17.439_566]);
        let mut v = View::new(cockpit, &chase);
        assert!(close(&v.matrix(), &cockpit));
        v.select(Mode::Chase);
        assert!(close(&v.matrix(), &chase));
    }

    #[test]
    fn camera_commands_move_the_eye() {
        let chase = translate([0.0, 0.0, -17.0]);
        let mut v = View::new(IDENTITY, &chase);
        v.select(Mode::Chase);
        v.select(Mode::FreeCamera);
        assert!(close(&v.matrix(), &chase));
        assert!(v.command("sim/general/forward"));
        assert!((eye_of(&v.matrix())[2] - 15.0).abs() < 1e-4);
        v.select(Mode::Chase);
        v.command("sim/general/right");
        // Orbiting keeps the distance to the aircraft.
        let e = eye_of(&v.matrix());
        assert!((dot(e, e).sqrt() - 17.0).abs() < 1e-4);
        assert!(!v.command("sim/operation/quit"));
    }
}
