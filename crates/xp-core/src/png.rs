//! PNG decoding for shipped X-Plane 10.51r2 bitmaps.
//!
//! [`decode`] returns the raw sample array a `.png` describes: `IDAT` inflated, the PNG filters
//! reversed, and for interlaced files the Adam7 passes interleaved back into full-image rows.
//! There is deliberately **no colour expansion**: an indexed file stays one palette index per
//! pixel, gray stays one channel, RGB stays three, and 16-bit stays two bytes per channel in
//! PNG's own big-endian order. Choosing an upload layout is `xp`'s decision, not this module's.
//!
//! ## Measured from shipped content
//!
//! The differential reference is a Python decoder built on `zlib` (`png_reference.py` in the
//! session scratch directory). Its output is pinned in `png_reference_sample.tsv`, and
//! `tests/png_golden.rs` walks all 6 862 shipped `.png` files through this module. The numbers
//! below come from that walk, not from the PNG specification:
//!
//! * **Histogram, with zero decode failures**: 8-bit RGBA 6 071, 8-bit RGB 589, 8-bit gray 97,
//!   16-bit RGBA 41, interlaced 8-bit RGBA 34, 1-bit indexed 20, 8-bit gray+alpha 9, interlaced
//!   8-bit RGB 1. Two further files use the `.PNG` spelling and are out of scope. Dimensions run
//!   from 1x27 to 4096x4096; no file has a zero width or height.
//! * **Only 14 chunk types appear**: `IHDR`, `PLTE`, `tRNS`, `IDAT`, `IEND`, `pHYs`, `cHRM`,
//!   `gAMA`, `iCCP`, `tEXt`, `tIME`, `sRGB`, `bKGD` and `iTXt`. Every ancillary name begins with
//!   a lowercase letter, so that is the rule this reader uses: validate the CRC of every chunk,
//!   handle the five that carry pixels or palette, skip anything whose first letter is a
//!   lowercase ASCII letter — which is how every standard ancillary chunk is spelled (`gAMA`,
//!   `cHRM`, `iCCP`, `sBIT`, `sRGB`, `tEXt`, `zTXt`, `iTXt`, `bKGD`, `hIST`, `pHYs`, `sPLT`,
//!   `tIME`, `oFFs`, `pCAL`, `gIFx`) — and reject anything else as an unhandled critical chunk.
//!   **`iTXt` occurs in 102 shipped files**, including `Resources/default scenery/1000 roads/
//!   textures/rail_swizzle.png` and `rail_swizzle8.png`, so a hand-written allowlist shorter than
//!   "lowercase first letter" fails shipped content.
//! * **`IDAT` is often split**: 6 350 files carry one `IDAT` and 512 carry between 2 and 2 866
//!   (`Custom Scenery/KSEA Demo Area/Overlay_Polys/SEATAC.png`), so the payloads are concatenated
//!   in file order and inflated once.
//! * **The zlib header is not always `78 da`**: nine distinct `CMF`/`FLG` pairs occur — `78da`
//!   (6 119), `789c` (574), `68de` (53), `58c3` (26), `38cb` (24), `7801` (23), `08d7` (20),
//!   `48c7` (15) and `785e` (8). 138 files declare a window smaller than 32 KiB (`CINFO` 0 to 6,
//!   the smallest being 256 bytes), so a reader that matches the two header bytes literally
//!   rejects them. `FDICT` is never set: no shipped file needs a preset dictionary.
//! * **All five filters are in use**: Sub appears in 5 693 files, Up in 5 482, Paeth in 4 770,
//!   None in 2 767 and Average in 868, and 5 642 files mix more than one type. No filter can be
//!   special-cased away.
//! * **`tRNS` appears only on the 20 indexed files, one byte each.** The two-byte gray and
//!   six-byte RGB colour-key forms the specification defines occur zero times in this install,
//!   and `PLTE` occurs zero times on a non-indexed file.
//! * **Everything else the specification worries about is well-formed here**: `IHDR` is the first
//!   chunk in all 6 862 files, `IEND` is present, last and empty in all of them, no chunk has a
//!   bad CRC, no `PLTE` or `tRNS` follows the first `IDAT`, nothing trails `IEND`, no file has a
//!   second `IHDR` or a second `PLTE`, and no interlaced file has a sub-byte bit depth.
//!
//! ## Follows the specification, unexercised by the corpus
//!
//! * **DEFLATE and the zlib wrapper are implemented here because no inflate crate was available.**
//!   `xp-core` may depend only on `anyhow`, with `sha2` as a dev-dependency, so `flate2`,
//!   `miniz_oxide`, `adler32`, `image` and `png` are all out of reach. Stored, fixed-Huffman and
//!   dynamic-Huffman blocks (RFC 1951 3.2.3 to 3.2.7), the two-byte `CMF`/`FLG` header and the
//!   Adler-32 trailer (RFC 1950 2.2), and the CRC-32 used by the chunk check are written from
//!   the RFCs.
//! * Tolerances match the reference decoder, which is `zlib` itself: bytes after the Adler-32
//!   trailer and inflated bytes past the last scanline are ignored, and a file with no `IEND` is
//!   accepted as long as its chunks tile the file exactly. Nothing in the install exercises any
//!   of that.
//! * The colour-type/bit-depth table, the Adam7 pass geometry and the five filter definitions
//!   come from the PNG specification; the corpus agrees with all three, so nothing here is a
//!   guess about shipped content.

use anyhow::{anyhow, bail, Context, Result};

/// The eight-byte signature every PNG begins with.
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// Adam7 pass geometry as `(x_offset, y_offset, x_step, y_step)`.
const ADAM7: [(usize, usize, usize, usize); 7] = [
    (0, 0, 8, 8),
    (4, 0, 8, 8),
    (0, 4, 4, 8),
    (2, 0, 4, 4),
    (0, 2, 2, 4),
    (1, 0, 2, 2),
    (0, 1, 1, 2),
];

/// A decoded PNG: the header, the palette and transparency if the file carries them, and the
/// sample array in full-image order.
///
/// `samples` is `ceil(width * channels * bit_depth / 8)` bytes per row, `height` rows, with
/// sub-byte depths packed most-significant-bit first within each row and 16-bit samples stored
/// big-endian. It is the file's own layout, with nothing added or reordered.
#[derive(Debug, Clone)]
pub struct PngInfo {
    /// `IHDR` width.
    pub width: u32,
    /// `IHDR` height.
    pub height: u32,
    /// `IHDR` bit depth, 1, 2, 4, 8 or 16.
    pub bit_depth: u8,
    /// `IHDR` colour type: 0 gray, 2 RGB, 3 indexed, 4 gray+alpha, 6 RGBA.
    pub color_type: u8,
    /// Samples per pixel for the colour type: 1, 3, 1, 2 or 4.
    pub channels: u8,
    /// True when `IHDR` declares Adam7 interlacing.
    pub interlaced: bool,
    /// `PLTE` as RGB triples, present only when the file carries a palette.
    pub palette: Option<Vec<[u8; 3]>>,
    /// Raw `tRNS` payload: one byte per index when indexed, otherwise two bytes per channel.
    pub transparency: Option<Vec<u8>>,
    /// Raw samples, filters reversed and passes interleaved, `height * row_bytes()` bytes.
    pub samples: Vec<u8>,
    /// Bytes the `IDAT` stream inflates to, before filters are reversed.
    pub inflated_len: usize,
}

impl PngInfo {
    /// Bytes in one full-image row: `ceil(width * channels * bit_depth / 8)`.
    pub fn row_bytes(&self) -> usize {
        let bits = (self.width as usize) * (self.channels as usize) * (self.bit_depth as usize);
        bits.div_ceil(8)
    }
}

/// Decode one shipped `.png` into its sample array.
pub fn decode(bytes: &[u8]) -> Result<PngInfo> {
    let chunks = Chunks::scan(bytes)?;
    let (width, height, bit_depth, color_type, compression, filtering, interlace) =
        chunks.ihdr.context("file has no IHDR chunk")?;
    if compression != 0 || filtering != 0 {
        bail!("IHDR compression={compression} filtering={filtering}, only 0/0 is defined");
    }
    let channels = channels_for(color_type)
        .ok_or_else(|| anyhow::anyhow!("IHDR colour type {color_type} is not defined"))?;
    if !depth_allowed(color_type, bit_depth) {
        bail!("IHDR bit depth {bit_depth} is illegal for colour type {color_type}");
    }
    if color_type == 3 {
        let plte = chunks
            .palette
            .as_ref()
            .context("indexed PNG (colour type 3) with no PLTE")?;
        if plte.len() % 3 != 0 {
            bail!("PLTE is {} bytes, not a multiple of 3", plte.len());
        }
    }

    let interlaced = interlace != 0;
    let hint = inflated_size_hint(
        width as usize,
        height as usize,
        channels as usize,
        bit_depth as usize,
        interlaced,
    )
    .context("sizing the IDAT stream")?;
    let raw = inflate_zlib(&chunks.idat, hint).context("inflating IDAT")?;
    let inflated_len = raw.len();
    let samples = unfilter(&raw, width, height, bit_depth, channels, interlaced)
        .with_context(|| format!("unfiltering a {width}x{height} type-{color_type} image"))?;

    let expect = (height as usize)
        .checked_mul(samples_row_bytes(
            width as usize,
            channels as usize,
            bit_depth as usize,
        )?)
        .context("sample array size overflows")?;
    if samples.len() != expect {
        bail!("decoded {} sample bytes, expected {expect}", samples.len());
    }

    Ok(PngInfo {
        width,
        height,
        bit_depth,
        color_type,
        channels,
        interlaced,
        palette: chunks
            .palette
            .map(|plte| plte.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect()),
        transparency: chunks.transparency,
        samples,
        inflated_len,
    })
}

/// Samples per pixel for a colour type; types 1 and 5 are the unused spellings.
fn channels_for(color_type: u8) -> Option<u8> {
    match color_type {
        0 => Some(1),
        2 => Some(3),
        3 => Some(1),
        4 => Some(2),
        6 => Some(4),
        _ => None,
    }
}

/// Bit depths a colour type may declare.
fn depth_allowed(color_type: u8, bit_depth: u8) -> bool {
    match color_type {
        0 => matches!(bit_depth, 1 | 2 | 4 | 8 | 16),
        2 | 4 | 6 => matches!(bit_depth, 8 | 16),
        3 => matches!(bit_depth, 1 | 2 | 4 | 8),
        _ => false,
    }
}

/// Bytes in one row of `width` pixels, checked for overflow.
fn samples_row_bytes(width: usize, channels: usize, bit_depth: usize) -> Result<usize> {
    width
        .checked_mul(channels)
        .and_then(|x| x.checked_mul(bit_depth))
        .map(|bits| bits.div_ceil(8))
        .context("row size overflows usize")
}

/// Bytes the inflated `IDAT` stream has to hold: one filter byte plus one row per scanline,
/// summed over the Adam7 passes that survive for an interlaced image.
fn inflated_size_hint(
    width: usize,
    height: usize,
    channels: usize,
    bit_depth: usize,
    interlaced: bool,
) -> Result<usize> {
    if !interlaced {
        let stride = samples_row_bytes(width, channels, bit_depth)?;
        return height
            .checked_mul(stride + 1)
            .context("image size overflows usize");
    }
    let mut total = 0usize;
    for &(x_off, y_off, x_step, y_step) in ADAM7.iter() {
        let pw = span(width, x_off, x_step);
        let ph = span(height, y_off, y_step);
        if pw == 0 || ph == 0 {
            continue;
        }
        let stride = samples_row_bytes(pw, channels, bit_depth)?;
        let need = ph.checked_mul(stride + 1).context("pass size overflows")?;
        total = total
            .checked_add(need)
            .context("pass sizes overflow usize")?;
    }
    Ok(total)
}

/// How many of `0..total` fall on the Adam7 grid starting at `off` with the given step.
fn span(total: usize, off: usize, step: usize) -> usize {
    if total <= off {
        0
    } else {
        (total - 1 - off) / step + 1
    }
}

/// Bytes per pixel used by the Sub/Average/Paeth filters, at least one for sub-byte depths.
fn bpp(channels: usize, bit_depth: usize) -> usize {
    (channels * bit_depth).div_ceil(8)
}

/// What the chunk walk collects. `IHDR`, `PLTE` and `tRNS` keep the copy seen, `IDAT` payloads
/// concatenate in file order, and the walk stops at `IEND` without looking at what follows.
struct Chunks {
    ihdr: Option<(u32, u32, u8, u8, u8, u8, u8)>,
    palette: Option<Vec<u8>>,
    transparency: Option<Vec<u8>>,
    idat: Vec<u8>,
}

impl Chunks {
    /// Walk every chunk, validating each CRC-32 before trusting its payload.
    fn scan(bytes: &[u8]) -> Result<Chunks> {
        if !bytes.starts_with(&PNG_MAGIC) {
            bail!("not a PNG: the eight-byte signature does not match");
        }
        let mut out = Chunks {
            ihdr: None,
            palette: None,
            transparency: None,
            idat: Vec::new(),
        };
        let mut pos = PNG_MAGIC.len();
        while pos < bytes.len() {
            if pos + PNG_MAGIC.len() > bytes.len() {
                bail!("truncated chunk header at offset {pos}");
            }
            let len =
                u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
                    as usize;
            let name = [
                bytes[pos + 4],
                bytes[pos + 5],
                bytes[pos + 6],
                bytes[pos + 7],
            ];
            let end = pos
                .checked_add(PNG_MAGIC.len())
                .and_then(|x| x.checked_add(len))
                .context("chunk length overflows the address space")?;
            let crc_at = end.checked_add(4).context("chunk end overflows")?;
            if crc_at > bytes.len() {
                bail!(
                    "truncated chunk body for `{}` at offset {pos}: {} bytes needed, {} left",
                    name_text(&name),
                    len,
                    bytes.len() - end
                );
            }
            let payload = &bytes[pos + 8..end];
            let stored =
                u32::from_be_bytes([bytes[end], bytes[end + 1], bytes[end + 2], bytes[end + 3]]);
            if crc32(&name, payload) != stored {
                bail!(
                    "CRC mismatch in `{}` at offset {pos}: chunk carries {stored:08x}, computed {:08x}",
                    name_text(&name),
                    crc32(&name, payload)
                );
            }
            match &name {
                b"IHDR" => {
                    if out.ihdr.is_some() {
                        bail!("duplicate IHDR at offset {pos}");
                    }
                    if payload.len() != 13 {
                        bail!("IHDR payload is {} bytes, expected 13", payload.len());
                    }
                    out.ihdr = Some((
                        u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]),
                        u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]),
                        payload[8],
                        payload[9],
                        payload[10],
                        payload[11],
                        payload[12],
                    ));
                }
                b"IDAT" => out.idat.extend_from_slice(payload),
                b"PLTE" => out.palette = Some(payload.to_vec()),
                b"tRNS" => out.transparency = Some(payload.to_vec()),
                b"IEND" => break,
                _ => {
                    // A lowercase first letter is the specification's own mark for an ancillary
                    // chunk, and every shipped one uses it; anything else is critical and this
                    // reader cannot honour it.
                    if !name[0].is_ascii_lowercase() {
                        bail!("unhandled critical chunk `{}`", name_text(&name));
                    }
                }
            }
            pos = crc_at;
        }
        Ok(out)
    }
}

/// A chunk name for an error message, keeping non-ASCII bytes visible.
fn name_text(name: &[u8; 4]) -> String {
    match std::str::from_utf8(name) {
        Ok(text) => text.to_string(),
        Err(_) => format!(
            "{:02x}{:02x}{:02x}{:02x}",
            name[0], name[1], name[2], name[3]
        ),
    }
}

/// Reverse the PNG filters, and for an interlaced image spread the passes back into full rows.
fn unfilter(
    raw: &[u8],
    width: u32,
    height: u32,
    bit_depth: u8,
    channels: u8,
    interlaced: bool,
) -> Result<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    let ch = channels as usize;
    let bd = bit_depth as usize;
    let stride = samples_row_bytes(w, ch, bd)?;
    if !interlaced {
        return unfilter_region(raw, h, stride, bpp(ch, bd));
    }

    let mut out = vec![
        0u8;
        h.checked_mul(stride)
            .context("image size overflows usize")?
    ];
    let sample_bits = ch * bd;
    let whole_bytes = sample_bits / 8;
    let mut pos = 0usize;
    for (pass, &(x_off, y_off, x_step, y_step)) in ADAM7.iter().enumerate() {
        let pw = span(w, x_off, x_step);
        let ph = span(h, y_off, y_step);
        if pw == 0 || ph == 0 {
            continue;
        }
        let p_stride = samples_row_bytes(pw, ch, bd)?;
        let need = ph
            .checked_mul(p_stride + 1)
            .context("pass size overflows usize")?;
        if need > raw.len() - pos {
            bail!(
                "interlaced pass {}: needs {need} bytes, {} left in the inflated stream",
                pass + 1,
                raw.len() - pos
            );
        }
        let pass_raw = unfilter_region(&raw[pos..pos + need], ph, p_stride, bpp(ch, bd))
            .with_context(|| format!("unfiltering interlaced pass {}", pass + 1))?;
        pos += need;
        for src_y in 0..ph {
            let dst_y = y_off + src_y * y_step;
            let src_base = src_y * p_stride;
            let dst_base = dst_y * stride;
            for x in 0..pw {
                let src_bit = x * sample_bits;
                let dst_bit = (x_off + x * x_step) * sample_bits;
                if sample_bits >= 8 {
                    for b in 0..whole_bytes {
                        out[dst_base + (dst_bit / 8) + b] = pass_raw[src_base + (src_bit / 8) + b];
                    }
                } else {
                    copy_subbyte(
                        &mut out,
                        dst_base,
                        dst_bit,
                        &pass_raw,
                        src_base,
                        src_bit,
                        sample_bits,
                    );
                }
            }
        }
    }
    Ok(out)
}

/// Move one sub-byte pixel from a pass row into its place in the full-image row.
fn copy_subbyte(
    out: &mut [u8],
    dst_base: usize,
    dst_bit: usize,
    src: &[u8],
    src_base: usize,
    src_bit: usize,
    bits: usize,
) {
    for b in 0..bits {
        let src_byte = src[src_base + ((src_bit + b) / 8)];
        let bit = (src_byte >> (7 - ((src_bit + b) % 8))) & 1;
        let dst_index = dst_base + ((dst_bit + b) / 8);
        let mask = 1u8 << (7 - ((dst_bit + b) % 8));
        if bit != 0 {
            out[dst_index] |= mask;
        } else {
            out[dst_index] &= !mask;
        }
    }
}

/// Undo the per-row filters for one (sub)image of `rows` rows, `stride` bytes each.
fn unfilter_region(raw: &[u8], rows: usize, stride: usize, pixel_bytes: usize) -> Result<Vec<u8>> {
    let mut out = vec![
        0u8;
        rows.checked_mul(stride)
            .context("region size overflows usize")?
    ];
    let mut prev = vec![0u8; stride];
    for row in 0..rows {
        let base = row
            .checked_mul(stride + 1)
            .context("row offset overflows usize")?;
        if base >= raw.len() {
            bail!(
                "row {row}: filter byte missing, the inflated stream holds {} bytes",
                raw.len()
            );
        }
        let filter = raw[base];
        let line_start = base + 1;
        let line_end = line_start
            .checked_add(stride)
            .context("scanline end overflows usize")?;
        if line_end > raw.len() {
            bail!(
                "row {row}: short scanline, {} of {stride} bytes present",
                raw.len() - base - 1
            );
        }
        let start = row * stride;
        let line = &mut out[start..start + stride];
        line.copy_from_slice(&raw[line_start..line_end]);
        match filter {
            0 => {}
            1 => {
                for i in pixel_bytes..stride {
                    line[i] = line[i].wrapping_add(line[i - pixel_bytes]);
                }
            }
            2 => {
                for i in 0..stride {
                    line[i] = line[i].wrapping_add(prev[i]);
                }
            }
            3 => {
                for i in 0..stride {
                    let left = if i >= pixel_bytes {
                        line[i - pixel_bytes]
                    } else {
                        0
                    };
                    line[i] = line[i].wrapping_add(((left as u16 + prev[i] as u16) >> 1) as u8);
                }
            }
            4 => {
                for i in 0..stride {
                    let left = if i >= pixel_bytes {
                        line[i - pixel_bytes]
                    } else {
                        0
                    };
                    let up = prev[i];
                    let upleft = if i >= pixel_bytes {
                        prev[i - pixel_bytes]
                    } else {
                        0
                    };
                    // Paeth picks the neighbour whose predicted value `left + up - upleft` is
                    // closest to, preferring left, then up, then upleft on ties.
                    let p = i32::from(left) + i32::from(up) - i32::from(upleft);
                    let pa = (p - i32::from(left)).abs();
                    let pb = (p - i32::from(up)).abs();
                    let pc = (p - i32::from(upleft)).abs();
                    let pred = if pa <= pb && pa <= pc {
                        left
                    } else if pb <= pc {
                        up
                    } else {
                        upleft
                    };
                    line[i] = line[i].wrapping_add(pred);
                }
            }
            other => bail!("row {row}: filter type {other} is not defined"),
        }
        prev.copy_from_slice(line);
    }
    Ok(out)
}

// ------------------------------------------------------------------ checksums --

/// The reflected CRC-32 polynomial, the same one zlib uses.
const CRC_POLYNOMIAL_REVERSED: u32 = 0xedb8_8320;

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0usize;
    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 != 0 {
                CRC_POLYNOMIAL_REVERSED ^ (value >> 1)
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

/// PNG stores CRC-32 over the chunk type and its data together, seeded and finalised with all
/// ones — the same polynomial as zlib's, a different accumulator.
const CRC_TABLE: [u32; 256] = crc_table();

fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    for &byte in data {
        crc = CRC_TABLE[((crc ^ u32::from(byte)) & 0xff) as usize] ^ (crc >> 8);
    }
    crc
}

fn crc32(name: &[u8; 4], payload: &[u8]) -> u32 {
    !crc32_update(crc32_update(0xffff_ffff, name), payload)
}

/// Adler-32 over the inflated bytes, as the zlib trailer requires.
fn adler32(data: &[u8]) -> u32 {
    // Reducing once per block keeps both sums inside a u64: 8 192 bytes add at most 2 087 936 to
    // `a` and 1.8e10 to `b`.
    const BLOCK: usize = 8192;
    let mut a: u64 = 1;
    let mut b: u64 = 0;
    for chunk in data.chunks(BLOCK) {
        for &byte in chunk {
            a += u64::from(byte);
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    ((b << 16) | a) as u32
}

// ------------------------------------------------------------------- inflate --

/// The longest code DEFLATE allows (RFC 1951 3.2.7).
const MAX_CODE_BITS: usize = 15;

/// LSB-first bit reader, the direction RFC 1951 3.1 fixes for everything except the Huffman codes
/// themselves, which are packed the other way round.
struct BitReader<'a> {
    src: &'a [u8],
    /// Next unread byte of `src`.
    pos: usize,
    /// Bits pulled from `src`, the first bit read in position 0.
    bitbuf: u64,
    bitcnt: u32,
}

impl BitReader<'_> {
    fn new(src: &[u8]) -> BitReader<'_> {
        BitReader {
            src,
            pos: 0,
            bitbuf: 0,
            bitcnt: 0,
        }
    }

    /// Pull whole bytes until `want` bits are buffered, stopping at the end of `src` so the caller
    /// can tell a short tail from a full read.
    fn fill(&mut self, want: u32) {
        while self.bitcnt < want && self.pos < self.src.len() {
            self.bitbuf |= u64::from(self.src[self.pos]) << self.bitcnt;
            self.pos += 1;
            self.bitcnt += 8;
        }
    }

    fn take(&mut self, n: u32) {
        self.bitbuf >>= n;
        self.bitcnt -= n;
    }

    /// One bit, least significant first. Huffman codes are assembled from these, one bit per
    /// length level, which is what keeps the decoder's bit order identical to the encoder's.
    fn read_bit(&mut self, what: &str) -> Result<u32> {
        self.fill(1);
        if self.bitcnt == 0 {
            bail!("deflate stream ends while reading {what}");
        }
        let bit = (self.bitbuf & 1) as u32;
        self.take(1);
        Ok(bit)
    }

    fn read_bits(&mut self, n: u32, what: &str) -> Result<u32> {
        self.fill(n);
        if self.bitcnt < n {
            bail!("deflate stream ends while reading {what}");
        }
        let value = (self.bitbuf & low_mask(n)) as u32;
        self.take(n);
        Ok(value)
    }

    /// Bytes of `src` the stream has consumed, rounded up to the byte boundary the zlib trailer
    /// sits on.
    fn consumed_bytes(&self) -> usize {
        let bits = (self.pos * 8) - self.bitcnt as usize;
        bits.div_ceil(8)
    }
}

fn low_mask(n: u32) -> u64 {
    if n >= 64 {
        !0u64
    } else {
        (1u64 << n) - 1
    }
}

/// A canonical Huffman code in the shape RFC 1951 3.2.7 describes it: how many codes exist at
/// each length, the lowest code value and first symbol index at each length, and the symbols in
/// (length, symbol) order.
///
/// Decoding walks the lengths one received bit at a time rather than indexing a lookup table.
/// That costs up to 15 bit reads per symbol instead of one load, and buys the thing that makes a
/// deflate decoder wrong in practice: a table decoder has to decide which end of the received bit
/// run the canonical code is matched against, and this form never asks the question — the code it
/// builds up is the same left-to-right number the encoder assigned.
#[derive(Default)]
struct Huffman {
    /// `counts[len]` symbols have a code of `len` bits.
    counts: [u32; MAX_CODE_BITS + 1],
    /// `first[len]` is the lowest `len`-bit code value the canonical assignment gives out.
    first: [u32; MAX_CODE_BITS + 1],
    /// `base[len]` is where the codes of length `len` start in `symbols`.
    base: [usize; MAX_CODE_BITS + 1],
    symbols: Vec<u16>,
    /// Longest code in use; zero means the alphabet names no code at all.
    max: usize,
    /// What this alphabet is, for the error message.
    what: &'static str,
}

impl Huffman {
    /// Assign the canonical codes RFC 1951 3.2.7 derives from `lengths`, the per-symbol code
    /// lengths, and reject anything the encoder could not have meant.
    ///
    /// An *over-subscribed* set is rejected outright: more codes than the shortest lengths can
    /// name is not a prefix code. An *incomplete* set is accepted, because the shipped install has
    /// them and zlib — the reference this decoder is pinned against — decodes them. The unused bit
    /// patterns simply match no symbol, and `decode` reports that as an invalid code if a stream
    /// actually emits one.
    fn build(lengths: &[u8], max_limit: usize, what: &'static str) -> Result<Huffman> {
        let mut h = Huffman {
            what,
            ..Huffman::default()
        };
        let mut max = 0usize;
        for &length in lengths {
            let length = usize::from(length);
            if length > MAX_CODE_BITS {
                bail!("{what}: code length {length} exceeds {MAX_CODE_BITS} bits");
            }
            h.counts[length] += 1;
            if length > max {
                max = length;
            }
        }
        if max > max_limit {
            bail!("{what}: code length {max} exceeds the {max_limit}-bit limit");
        }

        // Kraft sum, carrying one look-up bit per level so an exactly complete set lands on zero.
        let mut left: i64 = 1;
        for length in 1..=MAX_CODE_BITS {
            left <<= 1;
            left -= i64::from(h.counts[length]);
            if left < 0 {
                bail!("{what}: over-subscribed Huffman code set");
            }
        }

        // Canonical assignment: the first code of a length is the last code of the previous length
        // plus the codes that length used, shifted up one bit for the new bit of code.
        let mut code: u32 = 0;
        let mut base: usize = 0;
        for length in 1..=max {
            code <<= 1;
            h.first[length] = code;
            h.base[length] = base;
            base += h.counts[length] as usize;
            code += h.counts[length];
        }

        // Symbols in (length, symbol) order, which is the order the codes above were handed out.
        h.symbols.clear();
        h.symbols.resize(base, 0);
        let mut at = 0usize;
        for length in 1..=max {
            for (sym, &sym_len) in lengths.iter().enumerate() {
                if usize::from(sym_len) != length {
                    continue;
                }
                let sym = u16::try_from(sym)
                    .map_err(|_| anyhow!("{what}: symbol {sym} does not fit a u16 alphabet"))?;
                h.symbols[at] = sym;
                at += 1;
            }
        }
        h.max = max;
        Ok(h)
    }

    /// Read one code out of `r`.
    fn decode(&self, r: &mut BitReader<'_>) -> Result<u16> {
        let mut code: u32 = 0;
        for length in 1..=self.max {
            code = (code << 1) | r.read_bit("a code bit")?;
            let count = self.counts[length];
            if count == 0 {
                continue;
            }
            let first = self.first[length];
            if code < first {
                continue;
            }
            let offset = code - first;
            if offset < count {
                let index = self.base[length] + usize::try_from(offset).unwrap_or(usize::MAX);
                return self.symbols.get(index).copied().ok_or_else(|| {
                    anyhow!(
                        "{}: symbol table is shorter than its code lengths",
                        self.what
                    )
                });
            }
        }
        bail!("{}: no code matches the bits read", self.what);
    }
}

/// The alphabets for one deflate stream, kept between blocks so a multi-block image does not
/// rebuild them every time.
#[derive(Default)]
struct Tables {
    lit: Huffman,
    dist: Huffman,
    clcl: Huffman,
    lengths: Vec<u8>,
}

/// RFC 1951 3.2.5 length codes 257..285: base length and extra bits.
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];

/// RFC 1951 3.2.5 distance codes 0..29: base distance and extra bits.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// RFC 1951 3.2.7: the order the code-length code lengths are transmitted in.
const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// The fixed alphabets of RFC 1951 3.2.6. The distance code is incomplete on purpose: 30 codes in
/// 5 bits leaves two bit patterns unused, and a stream that uses one is an error.
fn fixed_tables(t: &mut Tables) -> Result<()> {
    let mut lit_lengths = [0u8; 288];
    for (sym, length) in lit_lengths.iter_mut().enumerate() {
        *length = match sym {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    let dist_lengths = [5u8; 30];
    t.lit = Huffman::build(&lit_lengths, MAX_CODE_BITS, "fixed literal/length code")?;
    t.dist = Huffman::build(&dist_lengths, MAX_CODE_BITS, "fixed distance code")?;
    Ok(())
}

/// Read the dynamic block header of RFC 1951 3.2.7 and build both alphabets from it.
fn build_dynamic(r: &mut BitReader<'_>, t: &mut Tables) -> Result<()> {
    let hlit = usize::try_from(r.read_bits(5, "HLIT")?).unwrap_or(0) + 257;
    let hdist = usize::try_from(r.read_bits(5, "HDIST")?).unwrap_or(0) + 1;
    let hclen = usize::try_from(r.read_bits(4, "HCLEN")?).unwrap_or(0) + 4;
    if hlit > 288 {
        bail!("deflate dynamic block: HLIT codes {hlit} literals, at most 288 exist");
    }

    let mut cl_lengths = [0u8; 19];
    for k in 0..hclen {
        cl_lengths[CL_ORDER[k]] = r.read_bits(3, "code-length code length")? as u8;
    }
    t.clcl = Huffman::build(&cl_lengths, 7, "code-length alphabet")?;

    let total = hlit + hdist;
    t.lengths.clear();
    t.lengths.resize(total, 0);
    let mut i = 0usize;
    while i < total {
        let sym = t.clcl.decode(r)?;
        match sym {
            0..=15 => {
                t.lengths[i] = sym as u8;
                i += 1;
            }
            16 => {
                if i == 0 {
                    bail!("deflate dynamic block: repeat previous length with nothing to repeat");
                }
                let run = 3 + usize::try_from(r.read_bits(2, "repeat count")?).unwrap_or(0);
                if run > total - i {
                    bail!("deflate dynamic block: length repeat runs past the alphabet");
                }
                let previous = t.lengths[i - 1];
                for k in 0..run {
                    t.lengths[i + k] = previous;
                }
                i += run;
            }
            17 | 18 => {
                let (extra, bits) = if sym == 17 { (3, 3) } else { (11, 7) };
                let run = extra + usize::try_from(r.read_bits(bits, "zero run")?).unwrap_or(0);
                if run > total - i {
                    bail!("deflate dynamic block: zero run runs past the alphabet");
                }
                i += run;
            }
            other => bail!("deflate dynamic block: code-length symbol {other} is not defined"),
        }
    }

    t.lit = Huffman::build(&t.lengths[..hlit], MAX_CODE_BITS, "literal/length alphabet")?;
    t.dist = Huffman::build(&t.lengths[hlit..], MAX_CODE_BITS, "distance alphabet")?;
    Ok(())
}

/// Decode the compressed data of one block into `out`.
fn codes(r: &mut BitReader<'_>, t: &Tables, out: &mut Vec<u8>) -> Result<()> {
    loop {
        let sym = t.lit.decode(r)?;
        if sym < 256 {
            out.push(sym as u8);
            continue;
        }
        if sym == 256 {
            return Ok(());
        }
        let index = usize::from(sym - 257);
        let Some(&base) = LENGTH_BASE.get(index) else {
            bail!("deflate: length symbol {sym} is not defined");
        };
        let len = usize::from(base)
            + usize::try_from(r.read_bits(u32::from(LENGTH_EXTRA[index]), "length bits")?)
                .unwrap_or(0);
        let dist_sym = usize::from(t.dist.decode(r)?);
        let Some(&dist_base) = DIST_BASE.get(dist_sym) else {
            bail!("deflate: distance symbol {dist_sym} is not defined");
        };
        let dist = usize::from(dist_base)
            + usize::try_from(r.read_bits(u32::from(DIST_EXTRA[dist_sym]), "distance bits")?)
                .unwrap_or(0);
        if dist > out.len() {
            bail!(
                "deflate: distance {dist} reaches past {} decoded bytes",
                out.len()
            );
        }
        let start = out.len() - dist;
        if dist >= len {
            out.extend_from_within(start..start + len);
        } else {
            // Overlapping copy: the source runs into the bytes being appended, and each byte just
            // written is fair game for the next one.
            for k in 0..len {
                let byte = out[start + k];
                out.push(byte);
            }
        }
    }
}

/// A stored (uncompressed) block, RFC 1951 3.2.4: byte-align, then LEN and its complement.
fn stored(r: &mut BitReader<'_>, out: &mut Vec<u8>) -> Result<()> {
    let pad = r.bitcnt % 8;
    r.take(pad);
    let len = usize::try_from(r.read_bits(16, "stored block length")?).unwrap_or(usize::MAX);
    let nlen = r.read_bits(16, "stored block complement")? as u16;
    if (len as u16) ^ nlen != 0xffff {
        bail!("deflate stored block: LEN {len} and NLEN {nlen:04x} are not complements");
    }
    let buffered = (r.bitcnt / 8) as usize;
    let available = buffered + (r.src.len() - r.pos);
    if len > available {
        bail!("deflate stored block: {len} bytes requested, {available} available");
    }
    let from_buffer = buffered.min(len);
    for k in 0..from_buffer {
        let byte = ((r.bitbuf >> (8 * k)) & 0xff) as u8;
        out.push(byte);
    }
    r.take(8 * from_buffer as u32);
    let rest = len - from_buffer;
    out.extend_from_slice(&r.src[r.pos..r.pos + rest]);
    r.pos += rest;
    Ok(())
}

/// Inflate one raw DEFLATE stream, returning how many input bytes it consumed.
fn inflate(src: &[u8], out: &mut Vec<u8>, size_hint: usize) -> Result<usize> {
    let mut r = BitReader::new(src);
    let mut t = Tables::default();
    // A dynamic block overwrites both alphabets, so "the fixed tables are built" is not an
    // invariant that survives it: a fixed block after a dynamic one has to put them back. Getting
    // this wrong decodes the fixed block's end-of-block code against the previous block's dynamic
    // table, which is how 529 shipped PNGs used to fail.
    let mut fixed_in_place = false;
    out.reserve(size_hint.min(1 << 27));
    loop {
        let final_block = r.read_bits(1, "block header")?;
        let block_type = r.read_bits(2, "block header")?;
        match block_type {
            0 => stored(&mut r, out)?,
            1 => {
                if !fixed_in_place {
                    fixed_tables(&mut t)?;
                    fixed_in_place = true;
                }
                codes(&mut r, &t, out)?;
            }
            2 => {
                build_dynamic(&mut r, &mut t)?;
                fixed_in_place = false;
                codes(&mut r, &t, out)?;
            }
            other => bail!("deflate block type {other} is reserved"),
        }
        if final_block == 1 {
            break;
        }
    }
    Ok(r.consumed_bytes())
}

/// Inflate the zlib-wrapped stream the `IDAT` chunks carry (RFC 1950 2.2), checking the header,
/// the Adler-32 trailer and nothing else: bytes past the trailer are ignored, as zlib does.
fn inflate_zlib(src: &[u8], size_hint: usize) -> Result<Vec<u8>> {
    if src.len() < 2 {
        bail!(
            "IDAT holds {} bytes, too short for a zlib stream",
            src.len()
        );
    }
    let cmf = src[0];
    let flg = src[1];
    if cmf & 0x0f != 8 {
        bail!("zlib compression method {} is not deflate", cmf & 0x0f);
    }
    if cmf >> 4 > 7 {
        bail!("zlib window exponent {} exceeds 7", cmf >> 4);
    }
    if ((u32::from(cmf) << 8) | u32::from(flg)) % 31 != 0 {
        bail!("zlib header check failed: CMF {cmf:02x} FLG {flg:02x} is not a multiple of 31");
    }
    if flg & 0x20 != 0 {
        bail!("zlib preset dictionary (FDICT) is not supported");
    }
    let mut out: Vec<u8> = Vec::new();
    let consumed = inflate(&src[2..], &mut out, size_hint)?;
    let tail = 2 + consumed;
    if tail + 4 > src.len() {
        bail!("zlib stream is truncated: no room for the Adler-32 trailer after {consumed} bytes");
    }
    let stored_adler = u32::from_be_bytes([src[tail], src[tail + 1], src[tail + 2], src[tail + 3]]);
    let computed = adler32(&out);
    if stored_adler != computed {
        bail!(
            "Adler-32 mismatch: trailer says {stored_adler:08x}, the inflated bytes give {computed:08x}"
        );
    }
    Ok(out)
}

/// The `gAMA` chunk of a PNG, as the file stores it (gamma × 100 000), if it has one before
/// its first `IDAT`.
pub fn gama_chunk(bytes: &[u8]) -> Option<u32> {
    let mut i = 8usize;
    while i + 12 <= bytes.len() {
        let n = u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        let tag = &bytes[i + 4..i + 8];
        if tag == b"IDAT" {
            return None;
        }
        if tag == b"gAMA" && n == 4 && i + 12 <= bytes.len() {
            return Some(u32::from_be_bytes([
                bytes[i + 8],
                bytes[i + 9],
                bytes[i + 10],
                bytes[i + 11],
            ]));
        }
        i = i.checked_add(12 + n)?;
    }
    None
}

/// Screen gamma the original hands libpng (`_colr`). 2.2 reproduces the oracle; the global
/// itself has not been read out of a running original.
pub const SCREEN_GAMMA: f64 = 2.2;

/// File gamma the original assumes for a PNG with no `gAMA` chunk: the double `0.5555…`
/// (1 / 1.8) at oracle `0x1010b5718`, passed to `png_set_gamma` in `TEX_load_png`'s
/// no-chunk branch (`0x1000d15de`).
pub const DEFAULT_FILE_GAMMA: f64 = 0.555_555_555_555_555_6;

/// The 8-bit table libpng applies to colour samples after
/// `png_set_gamma(SCREEN_GAMMA, file_gamma)`: `out = in ^ (1 / (file_gamma · screen_gamma))`.
///
/// `TEX_load_png` (oracle, call at `0x1000d0af6`) asks for it when its gamma flag is set and the
/// target is not alpha-only. A file tagged `gAMA 45454` comes out unchanged; an untagged one is
/// brightened by the power 0.818 — measured independently on the loading screen, where the
/// untagged `intro.png` is displayed brighter by a power of 0.816 while the tagged
/// `interface.png` cells are displayed exactly. Alpha is never touched.
pub fn display_lut(gama: Option<u32>) -> [u8; 256] {
    let file_gamma = match gama {
        Some(g) if g > 0 => f64::from(g) / 100_000.0,
        _ => DEFAULT_FILE_GAMMA,
    };
    let exponent = 1.0 / (file_gamma * SCREEN_GAMMA);
    let mut lut = [0u8; 256];
    for (i, v) in lut.iter_mut().enumerate() {
        *v = ((i as f64 / 255.0).powf(exponent) * 255.0 + 0.5).floor() as u8;
    }
    lut
}
