//! What the menu bar's items and the key bindings do: `APP_handlemenu` and the part of
//! `cmnd_class` the port can run.
//!
//! Every item of the bar does something. Items that open one of the original's windows open a
//! window with the original's frame and title; the windows' own controls are not ported yet and
//! the window says so. Commands with a visible effect run (views, camera moves, zoom, screenshot,
//! quit, check-mark toggles); the effects of toggles whose subsystem is not ported (smoke, replay,
//! A.I., movie recording) are only their check marks, and the notice line says so.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use xp_core::menu::{Action, Item, Rect};
use xp_core::view::Mode;

use crate::{chooser, Renderer};

/// One open window.
pub struct Win {
    /// The menu item that opened it; choosing the item again closes it.
    pub item: i32,
    pub title: String,
    pub lines: Vec<String>,
    pub rect: Rect,
    /// The window's widgets, for windows whose `*_wind()` has been extracted.
    pub def: Option<xp_core::btnwin::Window>,
    pub state: crate::btnui::State,
    /// Rows the widgets sit below the frame's split (see `content_drop`).
    pub drop: f32,
    /// The airport list of Select Airport / QuickFlight (`airp_wind`).
    pub apt: Option<crate::special::AptList>,
    /// QuickFlight's aircraft tree (`opensave_perframe` in mode 0x16).
    pub tree: Option<crate::chooser::Chooser>,
    /// Where `Ui::window` draws the frame (see `frame_rect`).
    pub frame: Rect,
}

/// `wins_class::start_win`: the window's GL origin is ((screen − size)·0.5 + 0.5) truncated,
/// at least 0, measured from the bottom-left corner.
fn centred(size: (f32, f32), screen: (f32, f32)) -> Rect {
    let o = |s: f32, w: f32| (((s - w) * 0.5 + 0.5) as i32).max(0) as f32;
    let l = o(screen.0, size.0);
    // Measured on the oracle frames (even and odd leftovers, full-screen and small windows):
    // the top edge sits ⌊(H − h − 1)/2⌋ rows down — one row above the bottom-origin split when
    // H − h is even.
    let t = ((screen.1 - size.1 - 1.0) * 0.5).floor();
    Rect {
        l,
        t,
        r: l + size.0,
        b: t + size.1,
    }
}

/// The rectangle `Ui::window` frames: `start_win`'s GL origin o = trunc((H − h)·0.5 + 0.5) puts
/// the window's top row at H − o − h, and `BTN_frame2` 0xe grows its quad 4 rows above that
/// (`Ui::window` grows 3, so the rect starts a row higher) and down to the window's bottom.
/// Against [`centred`] this moves the frame's bottom a row down when H − h is even and its
/// top a row up when odd — QuickFlight's and Select Airport's oracle frames.
fn frame_rect(size: (f32, f32), screen: (f32, f32)) -> Rect {
    let r = centred(size, screen);
    let o = (((screen.1 - size.1) * 0.5 + 0.5) as i32).max(0) as f32;
    let t = screen.1 - o - size.1;
    Rect {
        l: r.l,
        t: t - 1.0,
        r: r.r,
        b: t + size.1,
    }
}

/// Select Airport's widgets for the selected airport: its runway and ramp-start buttons
/// replace the previous airport's, and the list carries the map caption and ramp count.
fn airp_refresh(
    install: &std::path::Path,
    index: &[xp_core::apt_index::AptName],
    list: &mut crate::special::AptList,
    widgets: &mut Vec<xp_core::btnwin::Widget>,
) {
    widgets.retain(|w| !(w.key.starts_with("rwy:") || w.key.starts_with("ramp:")));
    list.title.clear();
    list.ramps = 0;
    let Some(a) = list.selected(index) else {
        return;
    };
    match xp_core::airport::find_in_install(install, &a.icao) {
        Ok((_, apt)) => {
            widgets.extend(xp_core::apt_index::airp_widgets(&apt, 0));
            list.title = xp_core::apt_index::map_title(&apt);
            list.ramps = xp_core::apt_index::ramp_names(&apt).len();
        }
        Err(e) => log::warn!("{e:#}"),
    }
}

/// The size `start_win` calls the screen (`_DAT_102f2b23c`): the interface's 1024×768, not the
/// window's. An oracle frame at 1920×1052 keeps "full-screen" windows 1024×768, centred.
pub(crate) const UI_SCREEN: (f32, f32) = (1024.0, 768.0);

/// Oracle frames: in windows 766 rows or taller the widgets sit 2 rows lower than the frame's
/// centring gives (Rendering Options 1020×766, every 1024×768 window); smaller windows match it.
/// Measured, not yet traced in start_win.
fn content_drop(size: (f32, f32)) -> f32 {
    if size.1 >= UI_SCREEN.1 - 2.0 {
        2.0
    } else {
        0.0
    }
}

impl Win {
    pub(crate) fn place(&self) -> crate::btnui::Place {
        crate::btnui::Place {
            ox: self.rect.l,
            oy: self.rect.t,
            // The window's bottom row is the frame's (H − o), its GL origin; the full-screen
            // windows' widgets sit `drop` rows below the centred split instead (measured).
            h: if self.drop > 0.0 {
                self.rect.h() + self.drop
            } else {
                self.frame.b - self.rect.t
            },
            tab_dy: 2.0 * self.drop,
        }
    }
}

impl Win {
    /// The two close boxes of the title strip (`Ui::window`).
    /// `BTN_frame2` 0xe draws the close boxes for start_win ids above 1 except 0x15 (Flight
    /// Plan) and 0x16 (QuickFlight) and the modal file windows (999, 1000, 0x3e2…0x3ee).
    pub(crate) fn closeable(&self) -> bool {
        match self.def.as_ref().map(|d| d.id) {
            None => true,
            Some(id) => id > 1 && id != 0x15 && id != 0x16 && !(999..=0x3ee).contains(&id),
        }
    }

    fn close_hit(&self, x: f32, y: f32) -> bool {
        if !self.closeable() {
            return false;
        }
        let t = self.rect.t;
        [self.rect.l + 6.0, self.rect.r - 24.0]
            .iter()
            .any(|&l| x >= l && x < l + 18.0 && y >= t && y < t + 18.0)
    }
}

/// Toggle commands that only change state the port shows as the item's check mark.
const MARK_TOGGLES: &[(&str, i32)] = &[
    ("sim/operation/video_record_toggle", 312),
    ("sim/operation/toggle_ai_flies", 606),
    ("sim/view/ai_controls_views", 607),
    ("sim/flight_controls/smoke_toggle", 609),
    ("sim/view/3d_path_toggle", 610),
    ("sim/replay/replay_toggle", 613),
    ("sim/view/cinema_verite", 1831),
    ("sim/view/sunglasses", 1832),
    ("sim/view/night_vision", 1833),
    ("sim/view/flashlight_red", 1834),
    ("sim/view/show_physics_model", 2103),
];

/// Toggle commands that open and close one of the original's windows.
const WINDOW_TOGGLES: &[(&str, i32)] = &[
    ("sim/operation/configure_video_recording", 313),
    ("sim/operation/toggle_logbook", 315),
    ("sim/operation/text_file_toggle", 2107),
    ("sim/operation/checklist_toggle", 2110),
    ("sim/operation/toggle_controllers_window", 2112),
    ("sim/general/toggle_artificial_stab_win", 2115),
    ("sim/general/toggle_autopilot_constants_win", 2116),
    ("sim/general/toggle_fadec_win", 2117),
    ("sim/general/toggle_control_deflections_win", 2118),
    ("sim/general/toggle_bouncers_win", 2119),
    ("sim/general/toggle_weapon_guidance_win", 2120),
    ("sim/general/toggle_projection_win", 2121),
    ("sim/operation/toggle_sky_colors_win", 2122),
    ("sim/operation/dev_console", 2123),
];

/// Items whose action the dispatcher table does not carry (no `win`/`cmd` row): the command the
/// item's text and check-mark command name.
fn implied_command(id: i32) -> Option<&'static str> {
    Some(match id {
        1819 => "sim/general/up",
        1832 => "sim/view/sunglasses",
        _ => return None,
    })
}

/// The `X-Plane Keys.prf` key text (as [`xp_core::menu::key_bindings`] spells it) → command.
pub fn key_commands(install: &Path) -> BTreeMap<String, String> {
    let prf = xp_core::read_text_lossy(install.join("Output/preferences/X-Plane Keys.prf"))
        .unwrap_or_default();
    let mut out = BTreeMap::new();
    for (cmd, key) in xp_core::menu::key_bindings_all(&prf) {
        out.entry(key).or_insert(cmd);
    }
    out
}

/// The prf name of a physical key, first letter lower-cased like the menu shows it.
pub fn key_name(code: winit::keyboard::KeyCode) -> Option<String> {
    use winit::keyboard::KeyCode as K;
    let s = match code {
        K::Space => "space".to_string(),
        K::Enter => "return".to_string(),
        K::Tab => "tab".to_string(),
        K::ArrowLeft => "left".to_string(),
        K::ArrowRight => "right".to_string(),
        K::ArrowUp => "up".to_string(),
        K::ArrowDown => "down".to_string(),
        K::Comma => ",".to_string(),
        K::Period => ".".to_string(),
        K::Equal => "=".to_string(),
        K::Minus => "-".to_string(),
        K::Slash => "/".to_string(),
        K::Semicolon => ";".to_string(),
        K::Quote => "'".to_string(),
        K::BracketLeft => "[".to_string(),
        K::BracketRight => "]".to_string(),
        K::Backquote => "`".to_string(),
        other => {
            let dbg = format!("{other:?}");
            if let Some(c) = dbg.strip_prefix("Key") {
                c.to_lowercase()
            } else if let Some(d) = dbg.strip_prefix("Digit") {
                d.to_string()
            } else if let Some(n) = dbg.strip_prefix("Numpad") {
                format!("numpad-{n}")
            } else if dbg.starts_with('F') && dbg[1..].chars().all(|c| c.is_ascii_digit()) {
                format!("f{}", &dbg[1..])
            } else {
                return None;
            }
        }
    };
    Some(s)
}

/// The key text with the original's modifier spelling (`SHIFT`, `OPT`, `CTRL`, joined by `+`).
/// The Mac's Command key is the original's CTRL.
pub fn key_text(name: &str, m: winit::keyboard::ModifiersState) -> String {
    let mut mods = Vec::new();
    if m.shift_key() {
        mods.push("SHIFT");
    }
    if m.alt_key() {
        mods.push("OPT");
    }
    if m.control_key() || m.super_key() {
        mods.push("CTRL");
    }
    if mods.is_empty() {
        name.to_string()
    } else {
        format!("{}-{name}", mods.join("+"))
    }
}

impl Renderer {
    /// `APP_handlemenu`.
    pub(crate) fn run_menu_item(&mut self, item: &Item) {
        match item.id {
            0 => self.toggle_window(
                item.id,
                "About X-Plane",
                vec![
                    crate::VERSION_LINE.to_string(),
                    String::new(),
                    "xplane10-rs: a Rust + wgpu reimplementation of X-Plane 10.51r2 that runs"
                        .into(),
                    "on your own X-Plane 10 installation. Not affiliated with Laminar Research."
                        .into(),
                ],
            ),
            600 => {
                self.chooser = Some(chooser::Chooser::new(
                    &self.install,
                    &self.aircraft[self.current],
                ));
                self.chooser_layers = None;
            }
            904 => self
                .notice("Get Me Lost: random relocation needs the flight model; not ported yet."),
            _ => {
                if let Some(c) = implied_command(item.id) {
                    self.run_command(c, Some(item));
                    return;
                }
                match &item.action {
                    Action::Command(c) => {
                        let c = c.clone();
                        self.run_command(&c, Some(item));
                    }
                    Action::Window(n) => {
                        let title = window_title(&item.text);
                        let n = *n;
                        self.open_window(item.id, &title, Some(n));
                    }
                    Action::Special(_) | Action::None => {
                        let title = window_title(&item.text);
                        self.toggle_window(item.id, &title, not_ported(&title));
                    }
                }
            }
        }
    }

    /// `cmnd_class::single_press` for the commands the port knows. Returns false if the command
    /// is unknown to the port.
    pub(crate) fn run_command(&mut self, cmd: &str, item: Option<&Item>) -> bool {
        if let Some(mode) = Mode::from_command(cmd) {
            self.view.select(mode);
            return true;
        }
        if self.view.command(cmd) {
            return true;
        }
        if let Some(&(_, id)) = MARK_TOGGLES.iter().find(|(c, _)| *c == cmd) {
            let on = self.toggles.insert(id);
            if !on {
                self.toggles.remove(&id);
            }
            let name = item
                .map(|i| i.text.trim().to_string())
                .unwrap_or(cmd.to_string());
            let effect = match id {
                1832..=1834 => "",
                _ => " (its effect is not ported yet)",
            };
            self.notice(&format!(
                "{name}: {}{effect}",
                if on { "on" } else { "off" }
            ));
            return true;
        }
        if let Some(&(_, id)) = WINDOW_TOGGLES.iter().find(|(c, _)| *c == cmd) {
            let text = item
                .map(|i| i.text.clone())
                .or_else(|| self.item_text(id))
                .unwrap_or_else(|| cmd.to_string());
            let title = window_title(&text);
            self.toggle_window(id, &title, not_ported(&title));
            return true;
        }
        match cmd {
            "sim/operation/quit" => self.quit = true,
            "sim/operation/screenshot" => self.screenshot = true,
            "sim/view/3d_path_reset" => self.notice("3-D flight path reset."),
            "sim/operation/stab_deriv_pitch"
            | "sim/operation/stab_deriv_heading"
            | "sim/operation/cycle_dump" => self.notice(&format!(
                "{}: needs the running flight model; not ported yet.",
                item.map(|i| i.text.trim()).unwrap_or(cmd)
            )),
            _ => return false,
        }
        true
    }

    fn item_text(&self, id: i32) -> Option<String> {
        self.menus
            .iter()
            .flat_map(|m| m.items.iter())
            .find(|i| i.id == id)
            .map(|i| i.text.clone())
    }

    /// Whether an item shows its check mark.
    pub(crate) fn checked(&self, id: i32) -> bool {
        id == self.view.mode.menu_item()
            || self.toggles.contains(&id)
            || self.windows.iter().any(|w| w.item == id)
    }

    pub(crate) fn notice(&mut self, text: &str) {
        log::info!("{text}");
        self.notice = Some((text.to_string(), 240));
    }

    /// `wins_class::start_win(n)`: the window's extracted widgets when the port has them,
    /// otherwise the framed placeholder.
    fn open_window(&mut self, item: i32, title: &str, start_win: Option<i32>) {
        let screen = (self.screen.0 as f32, self.screen.1 as f32);
        let def = match start_win.map(|n| xp_core::btnwin::load(n, title, UI_SCREEN)) {
            Some(Ok(d)) => d,
            Some(Err(e)) => {
                log::error!("{e:#}");
                None
            }
            None => None,
        };
        match def {
            None => self.toggle_window(item, title, not_ported(title)),
            Some(mut def) => {
                if start_win == Some(7) {
                    // The NAVAIDs tab lists the navaids around the start airport (KSEA).
                    let all = xp_core::navaid::load(&self.install);
                    if let Some((lat, lon)) =
                        xp_core::navaid::airport_position(&self.install, crate::START_AIRPORT)
                    {
                        let list = xp_core::navaid::fail_list(&all, lat, lon);
                        def.widgets
                            .extend(xp_core::navaid::fail_widgets(&list, def.size.1));
                    }
                }
                if start_win == Some(6) {
                    // Weight & Balance & Fuel: the loading the original sets up for the aircraft.
                    match xp_core::acf::Acf::load(self.install.join(&self.aircraft[self.current])) {
                        Ok(acf) => {
                            let load = xp_core::weights::Loading::from_acf(&acf);
                            load.apply(&mut def.widgets);
                            for (k, v) in load.values() {
                                let _ = self.prefs.set(&k, v);
                            }
                            let prefs = std::cell::RefCell::new(&mut self.prefs);
                            let get = |k: &str| prefs.borrow().get(k);
                            let mut set = |k: &str, v: f64| {
                                let _ = prefs.borrow_mut().set(k, v);
                            };
                            xp_core::weights::sync(&get, &mut set, "");
                        }
                        Err(e) => log::warn!("{e:#}"),
                    }
                }
                let drop = if def.id == 22 {
                    0.0
                } else {
                    content_drop(def.size)
                };
                self.windows_cache = None;
                if let Some(i) = self.windows.iter().position(|w| w.item == item) {
                    self.windows.remove(i);
                    return;
                }
                let apt = matches!(start_win, Some(1 | 22)).then(|| {
                    if self.apt_index.is_empty() {
                        self.apt_index = xp_core::apt_index::load(&self.install);
                    }
                    crate::special::AptList::new(&self.apt_index, crate::START_AIRPORT)
                });
                let mut apt = apt;
                if let (Some(1), Some(a)) = (start_win, apt.as_mut()) {
                    airp_refresh(&self.install, &self.apt_index, a, &mut def.widgets);
                }
                let tree = (start_win == Some(22)).then(|| {
                    crate::chooser::Chooser::new(&self.install, &self.aircraft[self.current])
                });
                self.windows.push(Win {
                    item,
                    title: def.title.clone(),
                    lines: Vec::new(),
                    rect: centred(def.size, screen),
                    frame: frame_rect(def.size, screen),
                    def: Some(def),
                    state: Default::default(),
                    drop,
                    apt,
                    tree,
                });
            }
        }
    }

    fn toggle_window(&mut self, item: i32, title: &str, lines: Vec<String>) {
        self.windows_cache = None;
        if let Some(i) = self.windows.iter().position(|w| w.item == item) {
            self.windows.remove(i);
            return;
        }
        let (w, h) = (640.0, 300.0);
        let k = self.windows.len() as f32 * 24.0;
        let l = ((self.screen.0 as f32 - w) * 0.5).floor() + k;
        let t = ((self.screen.1 as f32 - h) * 0.5).floor() + k;
        self.windows.push(Win {
            item,
            title: title.to_string(),
            lines,
            rect: Rect {
                l,
                t,
                r: l + w,
                b: t + h,
            },
            def: None,
            state: Default::default(),
            drop: 0.0,
            apt: None,
            tree: None,
            frame: Rect {
                l,
                t,
                r: l + w,
                b: t + h,
            },
        });
    }

    /// Re-centre the open windows on a new screen size, keeping their stacking offsets.
    pub(crate) fn place_windows(&mut self) {
        let (sw, sh) = (self.screen.0 as f32, self.screen.1 as f32);
        self.windows_cache = None;
        for win in self.windows.iter_mut() {
            if let Some(def) = &win.def {
                win.rect = centred(def.size, (sw, sh));
                win.frame = frame_rect(def.size, (sw, sh));
            }
        }
        for (k, win) in self
            .windows
            .iter_mut()
            .enumerate()
            .filter(|(_, w)| w.def.is_none())
        {
            let (w, h) = (win.rect.w(), win.rect.h());
            let l = ((sw - w) * 0.5).floor() + k as f32 * 24.0;
            let t = ((sh - h) * 0.5).floor().max(18.0) + k as f32 * 24.0;
            win.rect = Rect {
                l,
                t,
                r: l + w,
                b: t + h,
            };
            win.frame = win.rect;
        }
    }

    /// A click on the open windows. True when a window took it.
    pub(crate) fn click_windows(&mut self, x: f32, y: f32) -> bool {
        let Some(top) = self.windows.iter().rposition(|w| w.rect.contains(x, y)) else {
            return false;
        };
        if self.windows[top].close_hit(x, y) {
            self.windows.remove(top);
        } else if self.windows[top].def.is_some() {
            let w = &mut self.windows[top];
            let place = w.place();
            let mut id = 0;
            if let Some(def) = &w.def {
                id = def.id;
                crate::btnui::click(
                    def,
                    &mut self.prefs,
                    &self.presets,
                    &mut w.state,
                    place,
                    &self.ui,
                    x,
                    y,
                );
            }
            // Window GL coordinates of the press.
            let (gx, gy) = (x - place.ox, place.oy + place.h - y);
            if let Some(a) = &mut w.apt {
                let ly = if id == 22 { 605.0 } else { 595.0 };
                if crate::special::apt_click(a, 20.0, ly, gx, gy) && id == 1 {
                    if let Some(def) = &mut w.def {
                        airp_refresh(&self.install, &self.apt_index, a, &mut def.widgets);
                    }
                }
            }
            if id == 22 {
                if let Some((k, v)) = crate::special::qflt_click(w.tree.as_mut(), gx, gy) {
                    if let Err(e) = self.prefs.set(k, v) {
                        log::error!("{e:#}");
                    }
                }
            }
            self.apply_prefs();
        } else {
            // Bring it to the front.
            let w = self.windows.remove(top);
            self.windows.push(w);
        }
        self.windows_cache = None;
        true
    }

    /// Pointer motion with the button held: sliders follow it.
    pub(crate) fn drag_windows(&mut self, x: f32) {
        if let Some(w) = self.windows.last_mut() {
            let place = w.place();
            if let Some(def) = &w.def {
                if crate::btnui::drag(def, &mut self.prefs, &w.state, place, &self.ui, x) {
                    self.windows_cache = None;
                }
            }
        }
    }

    pub(crate) fn release(&mut self) {
        for w in &mut self.windows {
            w.state.drag = None;
        }
    }

    /// Settings with an effect the port has: the default forward view (`_default_view`, radio
    /// buttons of Rendering Options: 0 panel, 1 scenery only, 2 3-D cockpit, 3 HUD).
    pub(crate) fn apply_prefs(&mut self) {
        use xp_core::view::Mode;
        let want = match self.prefs.get("_default_view") as i32 {
            1 => Mode::ForwardNothing,
            2 => Mode::Cockpit3d,
            3 => Mode::ForwardHud,
            _ => Mode::ForwardPanel,
        };
        if (self.view.mode.is_forward() || self.view.mode == Mode::Cockpit3d)
            && self.view.mode != want
        {
            self.view.select(want);
        }
    }

    /// Escape: close the open menu, else the front window.
    pub(crate) fn escape(&mut self) {
        if self.open_menu.take().is_some() {
            self.menu_cache = None;
        } else if self.windows.pop().is_some() {
            self.windows_cache = None;
        } else if self.chooser.is_some() && self.session.is_some() {
            self.chooser = None;
        }
    }

    /// A key press: the binding from `X-Plane Keys.prf`, when it names a command the port runs.
    pub(crate) fn key(&mut self, text: &str) -> bool {
        let Some(cmd) = self.key_commands.get(text).cloned() else {
            return false;
        };
        let item = self
            .menus
            .iter()
            .flat_map(|m| m.items.iter())
            .find(|i| matches!(&i.action, Action::Command(c) if *c == cmd))
            .cloned();
        self.run_command(&cmd, item.as_ref())
    }

    /// One `--script` step per frame, once the aircraft is loaded:
    /// `close` (the start-up dialog), `item:<id>`, `cmd:<command>`, `key:<key text>`,
    /// `menu:<index>` (open a drop-down), `point:<x>,<y>`, `shot:<path>`, `wait`, `quit`.
    /// A port tool for parity screenshots; the original has no such option.
    pub(crate) fn script_step(&mut self) {
        let Some(step) = self.script.pop_front() else {
            return;
        };
        let verb0 = step.split(':').next().unwrap_or("");
        let early = matches!(verb0, "close" | "shot" | "wait" | "point" | "quit");
        if self.session.is_none() && !early {
            self.script.push_front(step);
            return;
        }
        let (verb, arg) = step.split_once(':').unwrap_or((step.as_str(), ""));
        match verb {
            "close" => self.chooser = None,
            "item" => {
                let item = arg.parse::<i32>().ok().and_then(|id| {
                    self.menus
                        .iter()
                        .flat_map(|m| m.items.iter())
                        .find(|i| i.id == id)
                        .cloned()
                });
                match item {
                    Some(i) => self.run_menu_item(&i),
                    None => log::warn!("script: no menu item {arg}"),
                }
            }
            "cmd" => {
                if !self.run_command(arg, None) {
                    log::warn!("script: the port does not run {arg}");
                }
            }
            "key" => {
                self.key(arg);
            }
            "menu" => {
                self.open_menu = arg.parse().ok();
                self.menu_cache = None;
            }
            "click" => {
                if let Some((x, y)) = arg.split_once(',') {
                    let (x, y) = (x.parse().unwrap_or(0.0), y.parse().unwrap_or(0.0));
                    self.click_windows(x, y);
                    self.release();
                }
            }
            "point" => {
                if let Some((x, y)) = arg.split_once(',') {
                    self.pointer_x = x.parse().unwrap_or(0.0);
                    self.pointer_y = y.parse().ok();
                }
            }
            "shot" => {
                self.shot_to = Some(arg.into());
                self.screenshot = true;
            }
            "wait" => {}
            "quit" => self.quit = true,
            other => log::warn!("script: unknown step {other}"),
        }
    }

    /// Write the frame as the original does: `Output/<aircraft>_<n>.png`, first free `n`
    /// (or where a `--script` `shot:` step says).
    pub(crate) fn save_screenshot(&mut self) {
        if let Some(path) = self.shot_to.take() {
            let r = crate::read_back(&self.gpu.device, &self.gpu.queue, &self.target, self.screen)
                .and_then(|mut rgba| {
                    for px in rgba.chunks_exact_mut(4) {
                        px[3] = 255;
                    }
                    let png = xp_core::png::encode_rgba(self.screen.0, self.screen.1, &rgba);
                    std::fs::write(&path, png).map_err(anyhow::Error::from)
                });
            if let Err(e) = r {
                log::error!("shot {}: {e:#}", path.display());
            }
            return;
        }
        let stem = self.aircraft[self.current]
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "X-Plane".into());
        let dir = self.install.join("Output");
        let path = (1..10_000)
            .map(|n| dir.join(format!("{stem}_{n}.png")))
            .find(|p| !p.exists());
        let Some(path) = path else {
            self.notice("Screenshot: no free file name in Output/");
            return;
        };
        let result = crate::read_back(&self.gpu.device, &self.gpu.queue, &self.target, self.screen)
            .and_then(|mut rgba| {
                for px in rgba.chunks_exact_mut(4) {
                    px[3] = 255;
                }
                let png = xp_core::png::encode_rgba(self.screen.0, self.screen.1, &rgba);
                std::fs::write(&path, png).map_err(anyhow::Error::from)
            });
        match result {
            Ok(()) => {
                let shown = path
                    .strip_prefix(&self.install)
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                self.notice(&format!("Screenshot saved: {shown}"));
            }
            Err(e) => self.notice(&format!("Screenshot failed: {e:#}")),
        }
    }
}

/// The window title the original gives an item's window: the item text, trimmed.
fn window_title(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn not_ported(title: &str) -> Vec<String> {
    vec![
        format!("The \"{title}\" window opens here in the original."),
        "Its controls are not ported yet.".into(),
        String::new(),
        "Close it with either box in the title strip, the menu item again, or Escape.".into(),
    ]
}

/// The set type the renderer keeps toggles in.
pub type Toggles = BTreeSet<i32>;
