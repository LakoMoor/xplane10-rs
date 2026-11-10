//! Glyph rasterization through FreeType, bound rather than rewritten.
//!
//! The original embeds FreeType and rasterizes its interface text with
//! `FT_Set_Char_Size(face, 0, size·64, 72, …)` (`OGL_face_info::load`, oracle `0x1000a5dc0`) and
//! `FT_Load_Char(face, ch, 0x10000)` (`OGL_font::require_char`, `0x1000a3a80`); `0x10000` is
//! `FT_LOAD_TARGET_LIGHT`, the light auto-hinter. No Rust rasterizer in reach reproduces that
//! hinter: measured on the oracle's menu title "About", an unhinted Rust rasterizer is 40 % of the
//! ink away, FreeType with these exact calls is 0.25 of 255 per channel away. So this crate is the
//! one place in the port that links a C library and uses `unsafe`; see `docs/PORT.md`.
//!
//! The system FreeType is newer than the one the original embeds, and the light hinter has
//! changed between versions, so the result is close, not guaranteed identical.

use std::ffi::CString;
use std::os::raw::{c_char, c_double, c_int, c_uchar, c_ulong, c_void};
use std::path::Path;

use anyhow::{bail, Context, Result};

/// `FT_LOAD_TARGET_LIGHT`, the flag the original passes to `FT_Load_Char`.
pub const LOAD_TARGET_LIGHT: i32 = 0x10000;

#[repr(C)]
#[derive(Default)]
struct RawGlyph {
    width: c_int,
    rows: c_int,
    left: c_int,
    top: c_int,
    advance: c_double,
}

extern "C" {
    fn xpft_open(path: *const c_char, size: c_double) -> *mut c_void;
    fn xpft_close(h: *mut c_void);
    fn xpft_glyph_render(
        h: *mut c_void,
        ch: c_ulong,
        flags: c_int,
        out: *mut RawGlyph,
        buf: *mut c_uchar,
        cap: c_int,
    ) -> c_int;
    fn xpft_version(maj: *mut c_int, min: *mut c_int, pat: *mut c_int);
}

/// One rasterized glyph: 8-bit coverage, row-major, top row first.
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    pub width: usize,
    pub rows: usize,
    /// Pixels from the pen position to the bitmap's left edge.
    pub left: i32,
    /// Pixels from the baseline up to the bitmap's top edge.
    pub top: i32,
    /// Pen advance in pixels.
    pub advance: f64,
    pub coverage: Vec<u8>,
}

/// A font file at one size.
pub struct Face {
    handle: *mut c_void,
}

impl Face {
    /// Open `path` at `size` pixels (points at 72 dpi).
    pub fn open(path: &Path, size: f64) -> Result<Face> {
        let c = CString::new(path.to_string_lossy().as_bytes())
            .with_context(|| format!("font path {} contains a NUL", path.display()))?;
        // SAFETY: `c` is a valid NUL-terminated string for the duration of the call; the shim
        // returns either NULL or a pointer it owns until `xpft_close`.
        let handle = unsafe { xpft_open(c.as_ptr(), size) };
        if handle.is_null() {
            bail!("FreeType could not open {} at size {size}", path.display());
        }
        Ok(Face { handle })
    }

    /// Rasterize one character with the given `FT_LOAD_*` flags.
    pub fn glyph(&self, ch: char, flags: i32) -> Result<Glyph> {
        let mut raw = RawGlyph::default();
        let mut buf = vec![0u8; 128 * 128];
        // SAFETY: `handle` came from `xpft_open` and is live; `raw` and `buf` are valid for
        // writes of the sizes passed, and the shim writes at most `cap` bytes into `buf`.
        let rc = unsafe {
            xpft_glyph_render(
                self.handle,
                ch as c_ulong,
                flags,
                &mut raw,
                buf.as_mut_ptr(),
                buf.len() as c_int,
            )
        };
        match rc {
            0 => {}
            2 => bail!("glyph {ch:?} is larger than 128x128"),
            _ => bail!("FreeType could not render {ch:?}"),
        }
        let (width, rows) = (raw.width as usize, raw.rows as usize);
        buf.truncate(width * rows);
        Ok(Glyph {
            width,
            rows,
            left: raw.left,
            top: raw.top,
            advance: raw.advance,
            coverage: buf,
        })
    }
}

impl Drop for Face {
    fn drop(&mut self) {
        // SAFETY: `handle` came from `xpft_open` and is closed exactly once.
        unsafe { xpft_close(self.handle) }
    }
}

/// The linked FreeType's version.
pub fn version() -> (i32, i32, i32) {
    let (mut a, mut b, mut c) = (0, 0, 0);
    // SAFETY: three valid out-pointers.
    unsafe { xpft_version(&mut a, &mut b, &mut c) };
    (a, b, c)
}

/// A line of text as one coverage bitmap.
#[derive(Debug, Clone, PartialEq)]
pub struct TextBitmap {
    pub width: usize,
    pub height: usize,
    /// Row of the baseline inside the bitmap.
    pub baseline: usize,
    /// Sum of the glyph advances, in pixels.
    pub advance: f64,
    pub coverage: Vec<u8>,
}

/// Lay `text` out left to right from pen 0, each glyph at the pen rounded to a whole pixel, as
/// the original's textured-quad text does. Coverage of overlapping glyphs adds and saturates.
pub fn render_line(face: &Face, text: &str, flags: i32) -> Result<TextBitmap> {
    let glyphs: Vec<Glyph> = text
        .chars()
        .map(|c| face.glyph(c, flags))
        .collect::<Result<_>>()?;
    let ascent = glyphs.iter().map(|g| g.top).max().unwrap_or(0).max(0);
    let descent = glyphs
        .iter()
        .map(|g| g.rows as i32 - g.top)
        .max()
        .unwrap_or(0)
        .max(0);
    let mut pen = 0.0f64;
    let mut right = 0i32;
    let mut placed = Vec::with_capacity(glyphs.len());
    for g in &glyphs {
        let x = (pen + 0.5).floor() as i32 + g.left;
        right = right.max(x + g.width as i32);
        placed.push(x);
        pen += g.advance;
    }
    let width = right.max(pen.ceil() as i32).max(1) as usize;
    let height = (ascent + descent).max(1) as usize;
    let mut coverage = vec![0u8; width * height];
    for (g, x0) in glyphs.iter().zip(placed) {
        let y0 = ascent - g.top;
        for y in 0..g.rows {
            for x in 0..g.width {
                let (px, py) = (x0 + x as i32, y0 + y as i32);
                if px < 0 || py < 0 || px as usize >= width || py as usize >= height {
                    continue;
                }
                let i = py as usize * width + px as usize;
                coverage[i] = coverage[i].saturating_add(g.coverage[y * g.width + x]);
            }
        }
    }
    Ok(TextBitmap {
        width,
        height,
        baseline: ascent as usize,
        advance: pen,
        coverage,
    })
}
