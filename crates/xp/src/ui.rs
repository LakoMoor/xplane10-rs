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

/// The menu bar's titles, left to right. Read off the oracle frame `cockpit_menubar.png`; the
/// original builds the list from its own tables (`mbar_class`), which are not ported.
pub const MENU_TITLES: [&str; 9] = [
    "About",
    "File",
    "Aircraft",
    "Location",
    "Environment",
    "Settings",
    "View",
    "Special",
    "Plugins",
];

/// Menu layout. `mbar_draw_menu_bar` (oracle `0x100df9ab0`) draws each title at its item's
/// x + 10. The baseline row (13) and the spacing are measured on the oracle frame: first ink of
/// the nine titles is at x = 10, 72, 119, 188, 262, 358, 431, 486, 555, which, less the 10 and
/// each first glyph's left bearing, puts the items at 0, 61, 109, 177, 251, 348, 421, 476, 544 —
/// every step is the title's hinted advance plus exactly 30.
const MENU_BASELINE_Y: f32 = 13.0;
const MENU_TEXT_INSET: f32 = 10.0;
const MENU_ITEM_GAP: f64 = 30.0;
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

    /// A flat rectangle in an 8-bit colour.
    pub fn fill(&self, rect: [f32; 4], rgba: [u8; 4]) -> Layer {
        self.solid(rect, rgba.map(|c| f32::from(c) / 255.0))
    }

    /// The 18-pixel title strip (the menu-bar cell of `interface.png`), `width` wide.
    pub fn title_strip(&self, top_left: [f32; 2], width: f32) -> Layer {
        Layer {
            view: self.atlas.clone(),
            rect: [top_left[0], top_left[1], width, MENU_BAR_HEIGHT],
            tint: [1.0; 4],
            src: Some([50.0, 0.0, 51.0, MENU_BAR_HEIGHT]),
            linear: false,
        }
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
        let mut item_x = 0.0f64;
        for title in MENU_TITLES {
            let (view, baseline, advance) = self.text(device, queue, title, [255; 3])?;
            let layer = Layer::bitmap(
                view,
                [item_x as f32 + MENU_TEXT_INSET, MENU_BASELINE_Y - baseline],
                [1.0; 4],
            );
            layers.push(layer);
            item_x += advance + MENU_ITEM_GAP;
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
