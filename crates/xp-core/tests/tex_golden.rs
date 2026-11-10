//! Golden tests for `xp_core::tex` — the shipped DDS header reader.
//!
//! Every expectation here was measured from the shipped install: the per-file fixtures were read
//! field by field out of their headers, and the corpus numbers come from walking all 1 901 `.dds`
//! files. The point of the corpus test is the three places where these files disagree with what a
//! spec-driven reader assumes — `dwCaps1` being unused, stale channel masks on fourcc files, and
//! declared mip counts that exceed what the payload holds.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use xp_core::tex::{self, DdsInfo, DdsVariant};

fn install() -> Result<PathBuf> {
    let root = xp_core::install_root().context("set XP_INSTALL to the shipped installation")?;
    if !root.is_dir() {
        bail!("XP_INSTALL={} is not a directory", root.display());
    }
    Ok(root)
}

fn read(root: &Path, rel: &str) -> Result<Vec<u8>> {
    let p = root.join(rel);
    std::fs::read(&p).with_context(|| format!("reading {rel}"))
}

/// One fixture: the header fields the file actually carries.
fn fixture(root: &Path, rel: &str) -> Result<DdsInfo> {
    let bytes = read(root, rel)?;
    tex::parse_dds(&bytes).with_context(|| format!("parsing {rel}"))
}

#[test]
fn smallest_dxt1_fixture_is_byte_exact() -> Result<()> {
    let root = install()?;
    let info = fixture(
        &root,
        "Resources/default scenery/1000 world terrain/textures/any.dds",
    )?;
    assert_eq!((info.width, info.height), (16, 16), "dimensions");
    assert_eq!(info.variant, DdsVariant::Bc1, "variant");
    assert_eq!((info.declared_mips, info.stored_mips), (5, 5), "mips");
    assert_eq!(info.payload_offset, 128, "payload offset");
    // 16x16 → 4x4 → 2x2 → 1x1 → 1x1 → 1x1 blocks: 4+1+1+1+1+1 = 9 blocks at 8 B = 72,
    // and the file is 312 B, so the chain the header declares is exactly the payload.
    assert_eq!(info.payload_len, 184, "payload length");
    assert_eq!(
        (0..info.stored_mips)
            .map(|l| info.level_bytes(l))
            .sum::<u64>(),
        info.payload_len,
        "declared chain fills the payload exactly"
    );
    assert!(info.gamma_hint, "ddspf.dwCaps carries the gamma hint");
    Ok(())
}

#[test]
fn dxt3_and_dxt5_blocks_are_sixteen_bytes() -> Result<()> {
    let root = install()?;

    let glass = fixture(
        &root,
        "Aircraft/Heavy Metal/B747-100 NASA/objects/Glass.dds",
    )?;
    assert_eq!(glass.variant, DdsVariant::Bc2, "DXT3 is BC2");
    assert_eq!((glass.width, glass.height), (256, 256), "Glass dimensions");
    assert_eq!(
        (glass.declared_mips, glass.stored_mips),
        (9, 9),
        "Glass mips"
    );
    assert_eq!(glass.payload_len, 87_408, "Glass payload");
    assert_eq!(
        (0..9).map(|l| glass.level_bytes(l)).sum::<u64>(),
        glass.payload_len,
        "a 16-byte BC2 chain fills the payload exactly"
    );

    let flame = fixture(&root, "Resources/bitmaps/world/fire/flame_burner_slow.dds")?;
    assert_eq!(flame.variant, DdsVariant::Bc3, "DXT5 is BC3");
    // Non-square on purpose: 128x16 exercises the per-dimension mip halving.
    assert_eq!((flame.width, flame.height), (128, 16), "flame dimensions");
    assert_eq!(
        (flame.declared_mips, flame.stored_mips),
        (8, 8),
        "flame mips"
    );
    assert_eq!(flame.payload_len, 2_832, "flame payload");
    assert_eq!(
        (0..8).map(|l| flame.level_bytes(l)).sum::<u64>(),
        flame.payload_len,
        "a 16-byte BC3 chain fills the payload exactly"
    );
    Ok(())
}

/// `Resources/bitmaps/Earth Orbit Textures/+00+000.dds` is the head of the 357-file class that
/// declares one level more than it stores. Uploading the declared count reads past EOF.
#[test]
fn truncated_mip_chain_is_corrected_to_what_the_file_holds() -> Result<()> {
    let root = install()?;
    let info = fixture(&root, "Resources/bitmaps/Earth Orbit Textures/+00+000.dds")?;
    assert_eq!(info.variant, DdsVariant::Bc3, "variant");
    assert_eq!((info.width, info.height), (1024, 1024), "dimensions");
    assert_eq!(info.declared_mips, 11, "header declares 11 levels");
    assert_eq!(info.stored_mips, 10, "payload holds 10");
    // The shortfall is exactly one 1x1 BC3 block.
    assert_eq!(
        info.payload_len + info.level_bytes(10),
        (0..11).map(|l| info.level_bytes(l)).sum::<u64>(),
        "the missing bytes are one 1x1 level"
    );
    assert!(
        (0..info.stored_mips)
            .map(|l| info.level_bytes(l))
            .sum::<u64>()
            <= info.payload_len,
        "the corrected chain must fit the payload"
    );
    Ok(())
}

/// 8 `DXT5` files in `1000 world terrain/textures/soil/` still carry `RGBBitCount = 32` and BGRA
/// masks. Trusting the masks would classify them as uncompressed RGBA.
#[test]
fn fourcc_beats_stale_channel_masks() -> Result<()> {
    let root = install()?;
    let info = fixture(
        &root,
        "Resources/default scenery/1000 world terrain/textures/soil/cliff_dark_tan.dds",
    )?;
    assert_eq!(
        info.variant,
        DdsVariant::Bc3,
        "fourcc decides, not the masks"
    );
    assert_eq!((info.width, info.height), (512, 512), "dimensions");
    assert_eq!((info.declared_mips, info.stored_mips), (10, 10), "mips");
    assert_eq!(info.payload_len, 349_552, "payload");
    assert_eq!(
        (0..10).map(|l| info.level_bytes(l)).sum::<u64>(),
        info.payload_len,
        "chain fills the payload"
    );
    Ok(())
}

/// The only two non-block-compressed `.dds` in the install. wgpu has no 24-bit texture format, so
/// the reader has to name the variant rather than map it.
#[test]
fn the_two_uncompressed_files_are_recognized_as_24_bit_rgb() -> Result<()> {
    let root = install()?;
    let info = fixture(&root, "Resources/bitmaps/world/lites/1000_lights_close.dds")?;
    assert_eq!(info.variant, DdsVariant::Rgb24, "variant");
    assert_eq!((info.width, info.height), (1024, 512), "dimensions");
    assert_eq!((info.declared_mips, info.stored_mips), (11, 11), "mips");
    // 3 bytes per texel, and the payload is exactly the 3-byte chain: 2 097 153 bytes.
    assert_eq!(info.payload_len, 2_097_153, "payload");
    assert_eq!(
        (0..11).map(|l| info.level_bytes(l)).sum::<u64>(),
        info.payload_len,
        "a 3-byte-per-texel chain fills the payload exactly"
    );
    // 0x00401000: TEXTURE | gamma hint, without the COMPLEX bit the BC files set.
    assert!(info.gamma_hint, "the gamma hint is set here too");
    Ok(())
}

/// The largest texture in the install, and the autogen `_ALB` page convention.
#[test]
fn largest_page_parses() -> Result<()> {
    let root = install()?;
    let info = fixture(
        &root,
        "Resources/default scenery/1000 autogen/US/urban_high/textures/Skyscrapers5_ALB.dds",
    )?;
    assert_eq!(info.variant, DdsVariant::Bc3, "variant");
    assert_eq!((info.width, info.height), (4096, 2048), "dimensions");
    assert_eq!((info.declared_mips, info.stored_mips), (13, 13), "mips");
    assert_eq!(info.payload_len, 11_184_848, "payload");
    assert_eq!(
        (0..13).map(|l| info.level_bytes(l)).sum::<u64>(),
        info.payload_len,
        "chain fills the payload"
    );
    Ok(())
}

/// The whole install: every `.dds` parses, the variant histogram matches the measured counts, and
/// the truncated-chain class is exactly 357 files.
#[test]
fn every_shipped_dds_parses_and_matches_the_measured_histogram() -> Result<()> {
    let root = install()?;
    let mut total = 0usize;
    let mut hist: [usize; 4] = [0; 4];
    let mut truncated = 0usize;
    let mut gamma = 0usize;
    let mut overlong = Vec::new();

    for path in walk(&root, "dds")? {
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let info = tex::parse_dds(&bytes).with_context(|| format!("parsing {}", path.display()))?;
        total += 1;
        match info.variant {
            DdsVariant::Bc1 => hist[0] += 1,
            DdsVariant::Bc2 => hist[1] += 1,
            DdsVariant::Bc3 => hist[2] += 1,
            DdsVariant::Rgb24 => hist[3] += 1,
        }
        if info.stored_mips < info.declared_mips {
            truncated += 1;
        }
        if info.gamma_hint {
            gamma += 1;
        }
        let chain: u64 = (0..info.stored_mips).map(|l| info.level_bytes(l)).sum();
        if chain > info.payload_len {
            overlong.push(format!("{}", path.display()));
        }
    }

    assert_eq!(total, 1901, "shipped .dds files");
    assert_eq!(hist[0], 387, "DXT1 files");
    assert_eq!(hist[1], 27, "DXT3 files");
    assert_eq!(hist[2], 1485, "DXT5 files");
    assert_eq!(hist[3], 2, "uncompressed 24-bit files");
    assert_eq!(
        truncated, 357,
        "files storing fewer levels than they declare"
    );
    assert_eq!(
        gamma, 1901,
        "files whose ddspf.dwCaps carries the gamma hint"
    );
    assert!(
        overlong.is_empty(),
        "corrected chains that still overflow the payload: {}",
        overlong.join(", ")
    );
    Ok(())
}

/// A spec-driven reader that trusts `dwCaps1` gets the whole install wrong: it is zero in every
/// shipped file while the real caps live in `ddspf.dwCaps`.
#[test]
fn gamma_hint_is_read_from_ddspf_caps_not_dwcaps1() -> Result<()> {
    let root = install()?;
    let bytes = read(
        &root,
        "Resources/default scenery/1000 world terrain/textures/any.dds",
    )?;
    assert_eq!(
        u32::from_le_bytes([bytes[0x70], bytes[0x71], bytes[0x72], bytes[0x73]]),
        0,
        "dwCaps1 is zero in the shipped files"
    );
    assert_ne!(
        u32::from_le_bytes([bytes[0x6c], bytes[0x6d], bytes[0x6e], bytes[0x6f]]) & 0x0040_0000,
        0,
        "ddspf.dwCaps carries 0x400000"
    );
    let info = tex::parse_dds(&bytes)?;
    assert!(
        info.gamma_hint,
        "the reader must follow the bytes, not the field name"
    );
    Ok(())
}

/// Every `.obj`-referenced texture name is a separate problem from the file bytes, and the loader
/// needs both: 3 183 `TEXTURE` lines name a `.png` where only a `.dds` exists. Asserted here so
/// the extension-substitution rule has a test behind it.
#[test]
fn extension_substitution_cases_are_present() -> Result<()> {
    let root = install()?;
    // Named .png, stored .dds.
    let obj =
        xp_core::read_text_lossy(root.join("Aircraft/Fighters/F-4 Phantom/objects/F4D_Wings.obj"))?;
    // The directive is whitespace-separated and this file separates with two TABs, so compare
    // fields rather than the whole line.
    assert!(
        obj.lines().any(|l| {
            let mut it = l.split_whitespace();
            it.next() == Some("TEXTURE") && it.next() == Some("F4D_Wings.png")
        }),
        "the object names a .png"
    );
    assert!(
        !root
            .join("Aircraft/Fighters/F-4 Phantom/objects/F4D_Wings.png")
            .exists(),
        "the named .png does not exist"
    );
    assert!(
        root.join("Aircraft/Fighters/F-4 Phantom/objects/F4D_Wings.dds")
            .exists(),
        "the .dds with the same stem does"
    );
    // And it parses as a real texture.
    let info = fixture(&root, "Aircraft/Fighters/F-4 Phantom/objects/F4D_Wings.dds")?;
    assert!(matches!(
        info.variant,
        DdsVariant::Bc1 | DdsVariant::Bc2 | DdsVariant::Bc3
    ));
    Ok(())
}

/// Every file under the install with the given extension, in a stable order.
fn walk(root: &Path, ext: &str) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = std::fs::read_dir(&dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .filter_map(|e| e.ok())
            .collect::<Vec<_>>();
        for e in rd {
            let p = e.path();
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                stack.push(p);
            } else if p.extension().and_then(|s| s.to_str()) == Some(ext) {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}
