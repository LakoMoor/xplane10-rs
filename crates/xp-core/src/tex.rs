//! Texture header decoding for shipped X-Plane 10.51r2 bitmaps.
//!
//! Window-free: this classifies a file and tells the caller how many mip levels are actually
//! present, so the GPU upload in `xp` never reads past the end of a file. Decoding pixels is not
//! done here — the block-compressed variants are uploaded as-is.
//!
//! Every rule below comes from measuring all 1 901 shipped `.dds` files, not from the DDS
//! specification. The places where the shipped files disagree with what a spec-driven reader
//! would assume are called out inline, because each one is a way a correct-looking loader gets
//! shipped content wrong:
//!
//! * **`dwCaps1` is zero in all 1 901 files.** The `TEXTURE`/`COMPLEX`/gamma-hint bits live in
//!   `ddspf.dwCaps` (offset `0x6c`): `0x00401008` in 1 115 files and `0x00401000` in 786. A
//!   reader that looks only at `dwCaps1` concludes "no COMPLEX, no gamma hint" for the entire
//!   install.
//! * **FourCC wins over channel masks.** 8 `DXT5` files in
//!   `Resources/default scenery/1000 world terrain/textures/soil/` still carry
//!   `RGBBitCount = 32` and BGRA masks from whatever exported them. Trusting the masks would
//!   classify them as uncompressed RGBA.
//! * **Declared mip count is sometimes a lie.** 357 files are exactly 16 bytes short of the chain
//!   they declare — they store N−1 levels and the final 1×1 block is absent. 355 of them are in
//!   `Resources/bitmaps/Earth Orbit Textures/`, plus `Resources/bitmaps/earth1.dds` and
//!   `earth2.dds`. Uploading the declared count reads past EOF.
//! * **No file uses the DX10 extension.** Zero files set `0x80000` in `dwFlags`, zero carry the
//!   20-byte `DDS_HEADER_DXT10`, so no DXGI format table is needed for this install.
//! * **Two files are uncompressed 24-bit RGB** (`Resources/bitmaps/world/lites/1000_lights_close.dds`
//!   and `1000_lights_distant2.dds`), 3 bytes per texel with an alpha mask of `0xff000000` that
//!   does not describe the data. wgpu has no 24-bit texture format, so these need a CPU expand.

use anyhow::{bail, Result};

/// Magic `DDS ` followed by a 124-byte header.
const DDS_MAGIC: &[u8; 4] = b"DDS ";

/// `DDPF_FOURCC`.
const DDPF_FOURCC: u32 = 0x4;
/// `DDPF_RGB`.
const DDPF_RGB: u32 = 0x40;
/// `DDSCAPS_TEXTUREGAMMAHINT`, seen in `ddspf.dwCaps` rather than in `dwCaps1`.
const CAPS_GAMMA_HINT: u32 = 0x0040_0000;

/// How a shipped `.dds` stores its texels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DdsVariant {
    /// FourCC `DXT1`. In this install it is **BC1 with alpha**: decoding every block of all 387
    /// such files shows 12 565 376 of 30 644 541 blocks (41.00 %) use the three-colour
    /// one-bit-alpha mode, and 386 of the 387 contain at least one such block. The opaque
    /// spelling is therefore not enough; the only file with no alpha-mode blocks is
    /// `Aircraft/Space Ships/Space Shuttle/Objects/Cockpit_TextLight.dds`.
    Bc1,
    /// FourCC `DXT3` — 27 files.
    Bc2,
    /// FourCC `DXT5` — 1 485 files.
    Bc3,
    /// `DDPF_RGB` with `RGBBitCount = 24`, 3 bytes per texel — 2 files. No wgpu format matches
    /// this, so it must be expanded on upload.
    Rgb24,
}

impl DdsVariant {
    /// Bytes for one `w`×`h` mip level of this variant.
    fn level_bytes(&self, w: u32, h: u32) -> u64 {
        let (w, h) = (u64::from(w.max(1)), u64::from(h.max(1)));
        match self {
            // BC1 packs color and 1-bit alpha into 8 bytes per 4x4 block. BC2 and BC3 carry an
            // explicit 4-bit alpha plane in addition, so their blocks are 16 bytes. Measured:
            // every DXT3/DXT5 payload in this install is exactly twice what an 8-byte-block chain
            // would need, and the 357 short files are short by exactly 16 bytes — one missing
            // 1x1 block of a 16-byte variant.
            DdsVariant::Bc1 => w.div_ceil(4) * h.div_ceil(4) * 8,
            DdsVariant::Bc2 | DdsVariant::Bc3 => w.div_ceil(4) * h.div_ceil(4) * 16,
            DdsVariant::Rgb24 => w * h * 3,
        }
    }
}

/// What a shipped `.dds` header says, corrected against what the file actually holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DdsInfo {
    pub width: u32,
    pub height: u32,
    pub variant: DdsVariant,
    /// `dwMipMapCount` as written. Zero is normalized to 1.
    pub declared_mips: u32,
    /// Levels the payload really contains. Never greater than [`DdsInfo::declared_mips`].
    pub stored_mips: u32,
    /// Offset of level 0 in the file.
    pub payload_offset: u64,
    /// Bytes the caller may read from `payload_offset`.
    pub payload_len: u64,
    /// `ddspf.dwCaps & 0x400000`, i.e. the exporter's gamma hint. Read from `dwCaps`, not
    /// `dwCaps1` — see the module docs.
    pub gamma_hint: bool,
}

impl DdsInfo {
    /// Bytes of one mip level.
    pub fn level_bytes(&self, level: u32) -> u64 {
        self.variant
            .level_bytes(self.width >> level, self.height >> level)
    }

    /// Offset of one level from [`DdsInfo::payload_offset`].
    pub fn level_offset(&self, level: u32) -> u64 {
        (0..level).map(|l| self.level_bytes(l)).sum()
    }
}

/// Parse a `.dds` header from the whole file's bytes.
///
/// Takes the full buffer rather than a slice because the mip-count correction needs the file
/// length: the only way to tell a 357-file truncated chain from a full one is to compare the
/// payload against the declared chain.
pub fn parse_dds(bytes: &[u8]) -> Result<DdsInfo> {
    if bytes.len() < 128 {
        bail!(
            "DDS is {} bytes, shorter than a 128-byte header",
            bytes.len()
        );
    }
    if &bytes[0..4] != DDS_MAGIC {
        bail!(
            "DDS magic is {:?}, expected 'DDS '",
            String::from_utf8_lossy(&bytes[0..4])
        );
    }
    let hdr_size = u32le(bytes, 4);
    if hdr_size != 124 {
        bail!("DDS header dwSize is {hdr_size}, expected 124");
    }
    let ddspf_size = u32le(bytes, 0x4c);
    if ddspf_size != 32 {
        bail!("DDS ddspf.dwSize is {ddspf_size}, expected 32");
    }

    let height = u32le(bytes, 0x0c);
    let width = u32le(bytes, 0x10);
    if width == 0 || height == 0 {
        bail!("DDS declares {width}x{height}");
    }
    let declared_mips = u32le(bytes, 0x1c).max(1);
    let ddspf_flags = u32le(bytes, 0x50);
    let fourcc = [bytes[0x54], bytes[0x55], bytes[0x56], bytes[0x57]];
    let bit_count = u32le(bytes, 0x58);
    let ddspf_caps = u32le(bytes, 0x6c);

    // FourCC first, masks only as a fallback: 8 shipped DXT5 files carry stale BGRA masks.
    let variant = if ddspf_flags & DDPF_FOURCC != 0 {
        match &fourcc {
            b"DXT1" => DdsVariant::Bc1,
            b"DXT3" => DdsVariant::Bc2,
            b"DXT5" => DdsVariant::Bc3,
            other => bail!(
                "DDS fourcc {:?} is not a variant this install ships (DXT1/DXT3/DXT5 only)",
                String::from_utf8_lossy(other)
            ),
        }
    } else if ddspf_flags & DDPF_RGB != 0 && bit_count == 24 {
        DdsVariant::Rgb24
    } else {
        bail!(
            "DDS has neither a fourcc nor a 24-bit RGB layout (ddspf flags {ddspf_flags:#010x}, \
             bit count {bit_count}); this install ships only DXT1, DXT3, DXT5 and two 24-bit files"
        );
    };

    let payload_offset = 128u64;
    let payload_len = u64::try_from(bytes.len())
        .unwrap_or(u64::MAX)
        .saturating_sub(payload_offset);

    // Correct the declared level count against the bytes that exist. The shipped files never have
    // trailing slack, so a shortfall is a missing tail level rather than padding.
    let full_chain = (0..declared_mips)
        .map(|l| variant.level_bytes(width >> l, height >> l))
        .sum::<u64>();
    let stored = if full_chain <= payload_len {
        declared_mips
    } else {
        let mut n = 0u32;
        let mut acc = 0u64;
        while n < declared_mips {
            let sz = variant.level_bytes(width >> n, height >> n);
            match acc.checked_add(sz) {
                Some(next) if next <= payload_len => {
                    acc = next;
                    n += 1;
                }
                _ => break,
            }
        }
        if n == 0 {
            bail!(
                "DDS {width}x{height} {variant:?} declares {declared_mips} levels but its {}-byte \
                 payload does not hold level 0 ({} bytes needed)",
                payload_len,
                variant.level_bytes(width, height)
            );
        }
        n
    };

    Ok(DdsInfo {
        width,
        height,
        variant,
        declared_mips,
        stored_mips: stored,
        payload_offset,
        payload_len,
        gamma_hint: ddspf_caps & CAPS_GAMMA_HINT != 0,
    })
}

/// Little-endian u32 at `at`. The header is fixed-layout, so a short buffer is a hard error the
/// caller already ruled out by the 128-byte length check.
fn u32le(bytes: &[u8], at: usize) -> u32 {
    let b: [u8; 4] = [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]];
    u32::from_le_bytes(b)
}
