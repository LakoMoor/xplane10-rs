//! The 2-D interface: loading screen, menu bar, cockpit panel. Everything here is drawn as
//! screen-space quads by [`crate::overlay`], after the 3-D frame.
//!
//! Positions are in pixels of a 1024×768 screen, the size every oracle frame was taken at. How
//! the original lays the interface out at other sizes is not ported.

use std::path::Path;

use anyhow::{Context, Result};

use crate::overlay::{Overlay, Quad};
use crate::textures::TextureStore;

/// One quad of interface, back to front.
pub struct Layer {
    pub view: wgpu::TextureView,
    /// Left, top, width, height in pixels.
    pub rect: [f32; 4],
    pub tint: [f32; 4],
    /// Part of the texture to show, in texels.
    pub src: Option<[f32; 4]>,
    /// Blend in linear light through the target's sRGB view instead of on the encoded values.
    /// Nothing measured so far needs it; kept for the passes that will.
    pub linear: bool,
}

/// `layers` moved by (dx, dy).
pub fn shifted(layers: Vec<Layer>, dx: f32, dy: f32) -> Vec<Layer> {
    layers
        .into_iter()
        .map(|mut l| {
            l.rect[0] += dx;
            l.rect[1] += dy;
            l
        })
        .collect()
}

/// Copies of `layers` moved by (dx, dy).
pub fn shifted_ref(layers: &[Layer], dx: f32, dy: f32) -> Vec<Layer> {
    layers
        .iter()
        .map(|l| Layer {
            view: l.view.clone(),
            rect: [l.rect[0] + dx, l.rect[1] + dy, l.rect[2], l.rect[3]],
            tint: l.tint,
            src: l.src,
            linear: l.linear,
        })
        .collect()
}

impl Layer {
    /// A whole bitmap, one texel per pixel, at `top_left`.
    pub fn bitmap(view: wgpu::TextureView, top_left: [f32; 2], tint: [f32; 4]) -> Layer {
        let size = view.texture().size();
        Layer {
            view,
            rect: [
                top_left[0],
                top_left[1],
                size.width as f32,
                size.height as f32,
            ],
            tint,
            src: None,
            linear: false,
        }
    }
}

/// Interface font: `DejaVuSans.ttf` at 10.5 px through FreeType's light hinter.
///
/// The file and the calls are the original's (`Resources/fonts/DejaVuSans.ttf` is named in the
/// binary; `xp_ft` documents the calls). The SIZE is fitted: of every shipped font at 9…13 px,
/// only this one at 10.5 reproduces the oracle's "About" (mean 0.25 of 255 per channel, 86 %
/// of samples identical); 10.2 and 10.8 are twenty times worse.
const UI_FONT: &str = "Resources/fonts/DejaVuSans.ttf";
const UI_FONT_PX: f64 = 10.5;

/// The original's glyph-coverage table, `s_font_gamma`: every byte FreeType produces goes through
/// it in `OGL_font::require_char` before it reaches the font texture. Its initializer (oracle
/// `0x1000a71f4`…) computes `(u8)(pow((float)i / 255.0f, 0.8f) * 255.0)` — a truncation, with
/// the exponent held as the f32 `0.8`.
///
/// This is why anti-aliased text is heavier than its coverage: 33 % coverage draws at 41 %.
/// An earlier reading explained the menu titles by linear-light blending instead; that fitted
/// white-on-grey by coincidence and predicted the loading screen's dark text the wrong way.
fn font_gamma(coverage: u8) -> u8 {
    let x = f64::from(f32::from(coverage) / 255.0);
    (x.powf(f64::from(0.8_f32)) * 255.0) as u8
}

/// Menu layout. `mbar_draw_menu_bar` (oracle `0x100df9ab0`) draws each title at its item's
/// x + 10. The baseline row (13) and the spacing are measured on the oracle frame: first ink of
/// the nine titles is at x = 10, 72, 119, 188, 262, 358, 431, 486, 555, which, less the 10 and
/// each first glyph's left bearing, puts the items at 0, 61, 109, 177, 251, 348, 421, 476, 544 —
/// every step is the title's hinted advance plus exactly 30.
const MENU_BASELINE_Y: f32 = 13.0;
const MENU_TEXT_INSET: f32 = 10.0;
const MENU_BAR_HEIGHT: f32 = 18.0;

/// Loading screen geometry, measured on `tests/golden/original/frames/loading.png` (the
/// original's window during startup, box-filtered from a 2× capture):
///
/// - `intro.png` (1024×1024, picture in rows 256…1023) one texel per pixel at (0, −256);
/// - rows 538…716: white at 70 % (picture (160, 215, 252) becomes (226, 243, 254));
/// - rows 599…658 of that: black at 20 % on top ((229, 244, 255) becomes (183, 195, 204));
/// - rows 538…539 and 717…718: black at 60 % (a 2-pixel rule);
/// - rows 719…767: the picture, untouched;
/// - text colour (51, 51, 51), left edge x = 10, lines 10 pixels apart.
///
/// These overlays blend on the encoded values: 70 % white over 160 is 226 in the frame, where
/// linear-light blending would give 233. The draw code of the
/// loading screen has not been read; the numbers are from the pixels.
const INTRO_OFFSET_PX: [f32; 2] = [0.0, -256.0];
const LOAD_TEXT_RGB: [u8; 3] = [51, 51, 51];
const LOAD_TEXT_X: f32 = 10.0;
/// Baselines (the row under the capitals) of the text lines, fitted to the oracle frame one
/// row at a time: a line placed one row off drops its band from 99.9 % to 93 % within 8.
const LOAD_VERSION_BASELINE: f32 = 528.0;
const LOAD_NOTICE_BASELINE: f32 = 558.0;
const LOAD_MESSAGE_BASELINE: f32 = 618.0;
const LOAD_TIP_BASELINE: f32 = 678.0;
const LOAD_STATUS_BASELINE: f32 = 737.0;
const LOAD_LINE_PITCH: f32 = 10.0;

/// The four fixed lines of the home-use notice; the strings are in the binary.
pub const HOME_USE_NOTICE: [&str; 4] = [
    "This is X-Plane, HOME-USE VERSION, NOT APPROVED FOR COMMERCIAL USE.",
    "Do NOT use this version for commercial applications.",
    "For commercial use, see www.X-Plane.com store.",
    "Report violations to austin@X-Plane.com.",
];

/// What the loading screen shows at one moment.
pub struct LoadingState<'a> {
    pub version_line: &'a str,
    /// `MESSAGE:` lines of a `TYPE: LOADING` block in
    /// `Output/preferences/server_message_1050.txt`.
    pub message: &'a [String],
    /// One entry of `Resources/hint_messages.txt`.
    pub tip: &'a [String],
    pub status: &'a str,
    /// 0…1.
    pub progress: f32,
}

/// The `MESSAGE:` lines of the install's cached server message, if it has a loading one.
pub fn server_message(install: &Path) -> Vec<String> {
    let path = install.join("Output/preferences/server_message_1050.txt");
    let Ok(text) = xp_core::read_text_lossy(&path) else {
        return Vec::new();
    };
    if !text.lines().any(|l| l.starts_with("TYPE: LOADING")) {
        return Vec::new();
    }
    text.lines()
        .filter_map(|l| l.strip_prefix("MESSAGE:"))
        .map(|l| l.trim().to_string())
        .collect()
}

/// Entry `index` (wrapping) of `Resources/hint_messages.txt`: blank-line separated blocks of
/// `N.  text` lines. The original picks one at random; the caller supplies the choice.
pub fn hint(install: &Path, index: usize) -> Vec<String> {
    let path = install.join("Resources/hint_messages.txt");
    let Ok(text) = xp_core::read_text_lossy(&path) else {
        return Vec::new();
    };
    let mut blocks: Vec<Vec<String>> = vec![Vec::new()];
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            if blocks.last().is_some_and(|b| !b.is_empty()) {
                blocks.push(Vec::new());
            }
            continue;
        }
        let body = match line.split_once('.') {
            Some((n, rest)) if n.chars().all(|c| c.is_ascii_digit()) => rest.trim(),
            _ => line,
        };
        if let Some(b) = blocks.last_mut() {
            b.push(body.to_string());
        }
    }
    blocks.retain(|b| !b.is_empty());
    if blocks.is_empty() {
        return Vec::new();
    }
    blocks.swap_remove(index % blocks.len())
}

/// Interface resources and the two overlay pipelines.
pub struct Ui {
    face: xp_ft::Face,
    white: wgpu::TextureView,
    atlas: wgpu::TextureView,
    intro: wgpu::TextureView,
    encoded: Overlay,
    linear: Overlay,
    target_format: wgpu::TextureFormat,
}

fn upload_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> wgpu::TextureView {
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * width),
            rows_per_image: None,
        },
        size,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

impl Ui {
    /// `target_format` is the non-sRGB format of the texture the interface is drawn into; that
    /// texture must list the sRGB spelling in its `view_formats`.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        store: &mut TextureStore,
        install: &Path,
        target_format: wgpu::TextureFormat,
    ) -> Result<Ui> {
        let bitmaps = install.join("Resources/bitmaps");
        Ok(Ui {
            face: xp_ft::Face::open(&install.join(UI_FONT), UI_FONT_PX)?,
            white: upload_rgba(device, queue, "white", 1, 1, &[255; 4]),
            atlas: store.get_ui(device, queue, &bitmaps.join("interface.png"))?,
            intro: store.get_ui(device, queue, &bitmaps.join("intro.png"))?,
            encoded: Overlay::new(device, target_format),
            linear: Overlay::new(device, target_format.add_srgb_suffix()),
            target_format,
        })
    }

    /// A line of text as a texture, with the row of its baseline and its advance.
    fn text(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        text: &str,
        rgb: [u8; 3],
    ) -> Result<(wgpu::TextureView, f32, f64)> {
        let bmp = xp_ft::render_line(&self.face, text, xp_ft::LOAD_TARGET_LIGHT)
            .with_context(|| format!("rasterizing {text:?}"))?;
        let rgba: Vec<u8> = bmp
            .coverage
            .iter()
            .flat_map(|c| [rgb[0], rgb[1], rgb[2], font_gamma(*c)])
            .collect();
        let view = upload_rgba(
            device,
            queue,
            text,
            bmp.width as u32,
            bmp.height as u32,
            &rgba,
        );
        Ok((view, bmp.baseline as f32, bmp.advance))
    }

    fn solid(&self, rect: [f32; 4], rgba: [f32; 4]) -> Layer {
        Layer {
            view: self.white.clone(),
            rect,
            tint: rgba,
            src: None,
            linear: false,
        }
    }

    /// A CPU image as a texture.
    pub fn solid_image(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        img: &(u32, u32, Vec<u8>),
    ) -> wgpu::TextureView {
        upload_rgba(device, queue, "image", img.0, img.1, &img.2)
    }

    /// An `interface.png` cell `src` (l, t, r, b texels) drawn into `dst` (l, t, w, h).
    pub fn atlas_cell(&self, src: [f32; 4], dst: [f32; 4]) -> Layer {
        Layer {
            view: self.atlas.clone(),
            rect: dst,
            tint: [1.0; 4],
            src: Some(src),
            linear: false,
        }
    }

    /// A flat rectangle in an 8-bit colour.
    pub fn fill(&self, rect: [f32; 4], rgba: [u8; 4]) -> Layer {
        self.solid(rect, rgba.map(|c| f32::from(c) / 255.0))
    }

    /// Text with its baseline's left end at `at`.
    pub fn text_at(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        text: &str,
        at: [f32; 2],
        rgb: [u8; 3],
    ) -> Result<Layer> {
        let (view, baseline, _) = self.text(device, queue, text, rgb)?;
        Ok(Layer::bitmap(view, [at[0], at[1] - baseline], [1.0; 4]))
    }

    /// Text centred on `at[0]`, baseline at `at[1]`.
    pub fn text_centred(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        text: &str,
        at: [f32; 2],
        rgb: [u8; 3],
    ) -> Result<Layer> {
        let (view, baseline, advance) = self.text(device, queue, text, rgb)?;
        Ok(Layer::bitmap(
            view,
            [(at[0] - advance as f32 * 0.5).round(), at[1] - baseline],
            [1.0; 4],
        ))
    }

    /// Text ending at `at[0]`, baseline at `at[1]`.
    pub fn text_right(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        text: &str,
        at: [f32; 2],
        rgb: [u8; 3],
    ) -> Result<Layer> {
        let (view, baseline, advance) = self.text(device, queue, text, rgb)?;
        Ok(Layer::bitmap(
            view,
            [(at[0] - advance as f32).round(), at[1] - baseline],
            [1.0; 4],
        ))
    }

    /// `OGL_measure_string`: the hinted advance of `text` in the interface font.
    pub fn measure(&self, text: &str) -> f32 {
        xp_ft::render_line(&self.face, text, xp_ft::LOAD_TARGET_LIGHT)
            .map(|b| b.advance as f32)
            .unwrap_or(0.0)
    }

    /// A filled rectangle in a palette colour at an opacity (`OGL_fillrect`).
    pub fn palette_fill(&self, r: xp_core::menu::Rect, index: usize, alpha: f32) -> Layer {
        let c = xp_core::menu::palette(index);
        self.solid(
            [r.l, r.t, r.w(), r.h()],
            [
                f32::from(c[0]) / 255.0,
                f32::from(c[1]) / 255.0,
                f32::from(c[2]) / 255.0,
                alpha,
            ],
        )
    }

    /// A one-pixel horizontal line from `x0` to `x1` (inclusive) on row `y` (`OGL_line`).
    pub fn hline(&self, x0: f32, x1: f32, y: f32, index: usize) -> Layer {
        self.palette_fill(
            xp_core::menu::Rect {
                l: x0,
                t: y,
                r: x1 + 1.0,
                b: y + 1.0,
            },
            index,
            1.0,
        )
    }

    /// Nine-slice an atlas cell over `dst`. `cell` is the cell's texels (l, t, r, b) and
    /// `border` its fixed left, top, right and bottom widths; `grow` pushes the drawn quad out
    /// past `dst` by that much on each side (a cell whose drop shadow lies outside the box).
    pub fn nine_slice(
        &self,
        dst: xp_core::menu::Rect,
        cell: [f32; 4],
        border: [f32; 4],
        grow: [f32; 4],
    ) -> Vec<Layer> {
        let (l, t, r, b) = (
            dst.l - grow[0],
            dst.t - grow[1],
            dst.r + grow[2],
            dst.b + grow[3],
        );
        let xs = [l, l + border[0], r - border[2], r];
        let ys = [t, t + border[1], b - border[3], b];
        let us = [cell[0], cell[0] + border[0], cell[2] - border[2], cell[2]];
        let vs = [cell[1], cell[1] + border[1], cell[3] - border[3], cell[3]];
        let mut out = Vec::with_capacity(9);
        for j in 0..3 {
            for i in 0..3 {
                if xs[i + 1] <= xs[i] || ys[j + 1] <= ys[j] {
                    continue;
                }
                out.push(Layer {
                    view: self.atlas.clone(),
                    rect: [xs[i], ys[j], xs[i + 1] - xs[i], ys[j + 1] - ys[j]],
                    tint: [1.0; 4],
                    src: Some([us[i], vs[j], us[i + 1], vs[j + 1]]),
                    linear: false,
                });
            }
        }
        out
    }

    /// [`Ui::nine_slice`] with the middle row and column repeated at one texel per pixel instead
    /// of stretched, for cells with a texture in them (the windows' 1-pixel pinstripe).
    pub fn nine_slice_tiled(
        &self,
        dst: xp_core::menu::Rect,
        cell: [f32; 4],
        border: [f32; 4],
        grow: [f32; 4],
    ) -> Vec<Layer> {
        let mut out = Vec::new();
        for q in self.nine_slice(dst, cell, border, grow) {
            let [x, y, w, h] = q.rect;
            let [u0, v0, u1, v1] = q.src.unwrap_or([0.0; 4]);
            let (sw, sh) = (u1 - u0, v1 - v0);
            if sw <= 0.0 || sh <= 0.0 {
                continue;
            }
            let mut ty = 0.0;
            while ty < h {
                let th = sh.min(h - ty);
                let mut tx = 0.0;
                while tx < w {
                    let tw = sw.min(w - tx);
                    out.push(Layer {
                        view: q.view.clone(),
                        rect: [x + tx, y + ty, tw, th],
                        tint: q.tint,
                        src: Some([u0, v0, u0 + tw, v0 + th]),
                        linear: false,
                    });
                    tx += sw;
                }
                ty += sh;
            }
        }
        out
    }

    /// An open drop-down, as `mbar_draw_menus` draws it: the frame (`BTN_frame2` type 0x14), the
    /// open title and the hovered item filled with palette 0x2d at 25 %, separators as two
    /// lines, item text at (left + 18, bottom − 2) in palette 0x2f, and `[key]` right-aligned 10
    /// pixels inside the right edge.
    pub fn drop_down(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        menu: &xp_core::menu::Menu,
        hover: Option<usize>,
        checked: &dyn Fn(i32) -> bool,
    ) -> Result<Vec<Layer>> {
        use xp_core::menu::{KEY_INSET, TEXT_INSET};
        let mut l = Vec::new();
        if menu.items.is_empty() {
            return Ok(l);
        }
        // `BTN_frame2` type 0x14 is the cell at texels x 78…103, y 157…183 of `interface.png`
        // with its drop shadow around it (x 71…110, y 156…194). Located by matching the
        // frame's rows and columns in the oracle frame `menu_1_about.png` against the atlas;
        // the slice widths are where the cell's edge profiles become constant.
        l.extend(self.nine_slice(
            menu.frame,
            [71.0, 156.0, 111.0, 195.0],
            [11.0, 6.0, 11.0, 20.0],
            [7.0, 1.0, 7.0, 11.0],
        ));
        l.push(self.palette_fill(menu.title_rect, 0x2d, 0.25));
        if let Some(h) = hover.and_then(|h| menu.items.get(h)) {
            l.push(self.palette_fill(h.rect, 0x2d, 0.25));
        }
        let text_rgb = xp_core::menu::palette(0x2f);
        for it in &menu.items {
            if it.separator {
                l.push(self.hline(it.rect.l, it.rect.r - 1.0, it.rect.t, 0x2c));
                l.push(self.hline(it.rect.l, it.rect.r - 1.0, it.rect.b - 1.0, 0x2e));
                continue;
            }
            let baseline = it.rect.b - 2.0;
            if it.checkable {
                // `BTN_util_element(l + 8, b + 6, 109, 187|206, 9, 9)`: an 18x18 atlas element
                // centred on texel (109, v) in GL texture space, i.e. image rows 512 − v ± 9.
                // v = 187 is the unchecked mark, 206 the checked one.
                let v = if checked(it.id) { 206.0 } else { 187.0 };
                l.push(Layer {
                    view: self.atlas.clone(),
                    rect: [it.rect.l + 8.0 - 9.0, it.rect.b - 6.0 - 9.0, 18.0, 18.0],
                    tint: [1.0; 4],
                    src: Some([100.0, 512.0 - v - 9.0, 118.0, 512.0 - v + 9.0]),
                    linear: false,
                });
            }
            l.push(self.text_at(
                device,
                queue,
                &it.text,
                [it.rect.l + TEXT_INSET, baseline],
                text_rgb,
            )?);
            if !it.key.is_empty() {
                l.push(self.text_right(
                    device,
                    queue,
                    &format!("[{}]", it.key),
                    [it.rect.r - KEY_INSET, baseline],
                    text_rgb,
                )?);
            }
        }
        Ok(l)
    }

    /// A movable window: `BTN_frame2` type 0xe (atlas cell x 1…101, y 54…154: title strip and
    /// pinstripe body with its drop shadow), title centred in white on baseline `t + 14`, and the
    /// close box (atlas x 120…138, y 316…334) at both ends of the title strip.
    ///
    /// Offsets fitted on the user's screenshot of the Open dialog (window at a known place): the
    /// cell is drawn from `t − 3`, so atlas row 57 (the strip's dark top line) lands on row `t`.
    /// The oracle's full-screen QuickFlight window fits the same rules with its `t` at −1. The
    /// close boxes' places are from the user's Rendering Options screenshot (centres 15 px
    /// inside each end).
    pub fn window(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rect: xp_core::menu::Rect,
        title: &str,
        close: bool,
    ) -> Result<Vec<Layer>> {
        let mut l = self.nine_slice_tiled(
            rect,
            [1.0, 54.0, 101.0, 154.0],
            [30.0, 35.0, 30.0, 35.0],
            [9.0, 3.0, 9.0, 14.0],
        );
        if close {
            for x in [rect.l + 6.0, rect.r - 24.0] {
                l.push(Layer {
                    view: self.atlas.clone(),
                    rect: [x, rect.t, 18.0, 18.0],
                    tint: [1.0; 4],
                    src: Some([120.0, 316.0, 138.0, 334.0]),
                    linear: false,
                });
            }
        }
        l.push(self.text_centred(
            device,
            queue,
            title,
            [((rect.l + rect.r) * 0.5).floor(), rect.t + 14.0],
            xp_core::menu::palette(0x2f),
        )?);
        Ok(l)
    }

    /// A sub-pane inside a window: `BTN_frame2` type 0x10 (atlas cell x 203…303, y 54…154).
    pub fn pane(&self, rect: xp_core::menu::Rect) -> Vec<Layer> {
        self.frame2(0x10, rect)
    }

    /// `BTN_frame2` for the window-family types 0xe…0x12: atlas cells at x 1, 102, 203, 304, 405
    /// (101 apart), rows 54…154. 0x12 is the dark translucent info pane (α 0.70 over the
    /// pinstripe gives the oracle's 91/255 in Rendering Options).
    pub fn frame2(&self, ty: i32, rect: xp_core::menu::Rect) -> Vec<Layer> {
        let u = 1.0 + (ty.clamp(0xe, 0x12) - 0xe) as f32 * 101.0;
        self.nine_slice(
            rect,
            [u, 54.0, u + 100.0, 154.0],
            [30.0, 30.0, 30.0, 30.0],
            [9.0, 4.0, 9.0, 14.0],
        )
    }

    /// The menu bar across the top of a screen `width` pixels wide.
    ///
    /// Background: `mbar_draw_menu_bar` draws it with `BTN_frame1(0x48, …)`; the pixels are the
    /// cell at texels x 1…100, y 0…17 of `interface.png`, untinted — the oracle frame carries
    /// that cell's values exactly, as 3-texel caps around its uniform middle column. Titles:
    /// white, through the font gamma table.
    pub fn menu_bar(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: f32,
        menus: &[xp_core::menu::Menu],
    ) -> Result<Vec<Layer>> {
        const CAP: f32 = 3.0;
        let cell = |x: f32, w: f32, l: f32, r: f32| Layer {
            view: self.atlas.clone(),
            rect: [x, 0.0, w, MENU_BAR_HEIGHT],
            tint: [1.0; 4],
            src: Some([l, 0.0, r, MENU_BAR_HEIGHT]),
            linear: false,
        };
        let mut layers = vec![
            cell(0.0, CAP, 1.0, 4.0),
            cell(CAP, width - 2.0 * CAP, 50.0, 51.0),
            cell(width - CAP, CAP, 98.0, 101.0),
        ];
        for m in menus {
            let (view, baseline, _) = self.text(device, queue, &m.title, [255; 3])?;
            layers.push(Layer::bitmap(
                view,
                [m.title_rect.l + MENU_TEXT_INSET, MENU_BASELINE_Y - baseline],
                [1.0; 4],
            ));
        }
        Ok(layers)
    }

    /// The loading screen for a 1024×768 screen.
    pub fn loading(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        state: &LoadingState<'_>,
    ) -> Result<Vec<Layer>> {
        const W: f32 = 1024.0;
        let mut layers = vec![
            Layer::bitmap(self.intro.clone(), INTRO_OFFSET_PX, [1.0; 4]),
            self.solid([0.0, 538.0, W, 179.0], [1.0, 1.0, 1.0, 0.7]),
            self.solid([0.0, 599.0, W, 60.0], [0.0, 0.0, 0.0, 0.2]),
            self.solid([0.0, 538.0, W, 2.0], [0.0, 0.0, 0.0, 0.6]),
            self.solid([0.0, 717.0, W, 2.0], [0.0, 0.0, 0.0, 0.6]),
        ];
        let put = |text: &str, baseline_y: f32, layers: &mut Vec<Layer>| -> Result<()> {
            if text.is_empty() {
                return Ok(());
            }
            let (view, baseline, _) = self.text(device, queue, text, LOAD_TEXT_RGB)?;
            layers.push(Layer::bitmap(
                view,
                [LOAD_TEXT_X, baseline_y - baseline],
                [1.0; 4],
            ));
            Ok(())
        };
        put(state.version_line, LOAD_VERSION_BASELINE, &mut layers)?;
        for (i, line) in HOME_USE_NOTICE.iter().enumerate() {
            put(
                line,
                LOAD_NOTICE_BASELINE + i as f32 * LOAD_LINE_PITCH,
                &mut layers,
            )?;
        }
        for (i, line) in state.message.iter().enumerate() {
            put(
                line,
                LOAD_MESSAGE_BASELINE + i as f32 * LOAD_LINE_PITCH,
                &mut layers,
            )?;
        }
        for (i, line) in state.tip.iter().enumerate() {
            put(
                line,
                LOAD_TIP_BASELINE + i as f32 * LOAD_LINE_PITCH,
                &mut layers,
            )?;
        }
        put(state.status, LOAD_STATUS_BASELINE, &mut layers)?;

        // Progress bar: two cells of `interface.png`, rows 337…347, found by compositing every
        // atlas column over the picture and matching the oracle's columns (residual 7…11 summed
        // over 33 samples, i.e. exact to rounding). The track is texels x 114…137 with 4-texel
        // caps, on screen x 10…1013; the fill is texels x 86…108 with a 2-texel left cap and a
        // 3-texel right cap, starting one pixel inside the track. In the oracle frame the fill
        // is 137 of the 1002 pixels it can occupy.
        let cell = |x: f32, w: f32, l: f32, r: f32| Layer {
            view: self.atlas.clone(),
            rect: [x, 747.0, w, 11.0],
            tint: [1.0; 4],
            src: Some([l, 337.0, r, 348.0]),
            linear: false,
        };
        let (x0, x1) = (10.0f32, 1014.0f32);
        layers.push(cell(x0, 4.0, 114.0, 118.0));
        layers.push(cell(x0 + 4.0, x1 - x0 - 8.0, 120.0, 121.0));
        layers.push(cell(x1 - 4.0, 4.0, 134.0, 138.0));
        let fill = ((x1 - x0 - 2.0) * state.progress.clamp(0.0, 1.0)).round();
        if fill >= 5.0 {
            let f0 = x0 + 1.0;
            layers.push(cell(f0, 2.0, 86.0, 88.0));
            layers.push(cell(f0 + 2.0, fill - 5.0, 95.0, 96.0));
            layers.push(cell(f0 + fill - 3.0, 3.0, 106.0, 109.0));
        }
        Ok(layers)
    }

    /// Draw `layers` over `target`, in order, switching between the encoded and the sRGB view
    /// as each layer asks.
    pub fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::Texture,
        extent: (u32, u32),
        layers: &[Layer],
    ) {
        let encoded = target.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.target_format),
            ..Default::default()
        });
        let linear = target.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.target_format.add_srgb_suffix()),
            ..Default::default()
        });
        let mut i = 0;
        while i < layers.len() {
            let mode = layers[i].linear;
            let mut j = i;
            while j < layers.len() && layers[j].linear == mode {
                j += 1;
            }
            let quads: Vec<Quad<'_>> = layers[i..j]
                .iter()
                .map(|l| Quad {
                    texture: &l.view,
                    rect: l.rect,
                    tint: l.tint,
                    src: l.src,
                })
                .collect();
            if mode {
                self.linear.draw(device, queue, &linear, extent, &quads);
            } else {
                self.encoded.draw(device, queue, &encoded, extent, &quads);
            }
            i = j;
        }
    }
}
