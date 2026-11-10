//! Golden tests for `xp_core::png` — the shipped PNG reader.
//!
//! Every expectation here was measured from the shipped install. The differential reference is a
//! Python decoder built on `zlib` (`png_reference.py` in the session scratch directory); its
//! output is pinned in a TSV of `path`, the SHA-256 of the decoded sample array, and the header
//! fields, and these tests decode the same files through the Rust reader and compare.
//!
//! The sample table (`png_reference_sample.tsv`, 111 rows) is read from `XP_PNG_REFERENCE`; the
//! full-corpus table (`png_reference.tsv`, 6 862 rows) is read from `XP_PNG_REFERENCE_FULL`, and
//! the corpus digest check skips loudly when it is absent.
//!
//! What the corpus tests pin is the measured shape of the install, not a theory about PNG: the
//! 6 862-file count with the two `.PNG` files excluded, the exact format histogram, zero decode
//! failures, and a digest over every file's sample bytes. The byte-exact fixtures are pinned as
//! bytes rather than hashes so that a systematic byte-order or stride bug cannot hide behind a
//! matching digest.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use xp_core::png::{self, PngInfo};

/// Pinned sample of the reference decoder's output, 111 rows covering every shipped format.
const DEFAULT_SAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/golden/png/png_reference_sample.tsv"
);

/// The same output for all 6 862 shipped `.png` files.
const DEFAULT_FULL: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/golden/png/png_reference.tsv"
);

fn install() -> Result<PathBuf> {
    let root = xp_core::install_root().context("set XP_INSTALL to the shipped installation")?;
    if !root.is_dir() {
        bail!("XP_INSTALL={} is not a directory", root.display());
    }
    Ok(root)
}

fn read(root: &Path, rel: &str) -> Result<Vec<u8>> {
    let path = root.join(rel);
    std::fs::read(&path).with_context(|| format!("reading {rel}"))
}

fn decode_file(root: &Path, rel: &str) -> Result<PngInfo> {
    let bytes = read(root, rel)?;
    png::decode(&bytes).with_context(|| format!("decoding {rel}"))
}

/// Where the reference tables live, overridable so the tests can be re-pointed at a fresh run.
fn reference_path(var: &str, default: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

/// The full table sits next to the sample table unless it is named explicitly.
fn full_reference_path() -> PathBuf {
    if let Some(path) = std::env::var_os("XP_PNG_REFERENCE_FULL") {
        return PathBuf::from(path);
    }
    let sample = reference_path("XP_PNG_REFERENCE", DEFAULT_SAMPLE);
    match sample.parent() {
        Some(dir) => dir.join("png_reference.tsv"),
        None => PathBuf::from(DEFAULT_FULL),
    }
}

/// One row of a reference table.
struct Row {
    path: String,
    sha256: String,
    width: u64,
    height: u64,
    bit_depth: u64,
    color_type: u64,
    channels: u64,
    interlace: u64,
    plte: u64,
    trns: u64,
    inflated: u64,
}

/// Parse a reference table. The header line is skipped; every other line must have 11 fields.
fn reference_table(path: &Path) -> Result<Vec<Row>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading reference table {}", path.display()))?;
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if index == 0 || line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 11 {
            bail!(
                "{} line {}: {} tab-separated fields, expected 11",
                path.display(),
                index + 1,
                fields.len()
            );
        }
        let number = |field: &str, which: &str| -> Result<u64> {
            field.parse::<u64>().with_context(|| {
                format!(
                    "{} line {}: {which} is {field:?}",
                    path.display(),
                    index + 1
                )
            })
        };
        rows.push(Row {
            path: fields[0].to_string(),
            sha256: fields[1].to_string(),
            width: number(fields[2], "width")?,
            height: number(fields[3], "height")?,
            bit_depth: number(fields[4], "bit depth")?,
            color_type: number(fields[5], "colour type")?,
            channels: number(fields[6], "channels")?,
            interlace: number(fields[7], "interlace")?,
            plte: number(fields[8], "palette count")?,
            trns: number(fields[9], "tRNS length")?,
            inflated: number(fields[10], "inflated length")?,
        });
    }
    Ok(rows)
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Decode a hex literal into the bytes it spells.
fn unhex(text: &str) -> Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        bail!("hex literal has an odd length");
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    for pair in text.as_bytes().chunks_exact(2) {
        let hi = hex_digit(pair[0])?;
        let lo = hex_digit(pair[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_digit(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => bail!("not a hex digit: {}", byte as char),
    }
}

/// Compare one decoded file against its reference row, collecting every disagreement.
fn compare(row: &Row, info: &PngInfo) -> Vec<String> {
    let mut bad = Vec::new();
    let digest = hex_sha256(&info.samples);
    if digest != row.sha256 {
        bad.push(format!("samples sha256 {digest} != {}", row.sha256));
    }
    let checks: [(&str, u64, u64); 8] = [
        ("width", u64::from(info.width), row.width),
        ("height", u64::from(info.height), row.height),
        ("bit depth", u64::from(info.bit_depth), row.bit_depth),
        ("colour type", u64::from(info.color_type), row.color_type),
        ("channels", u64::from(info.channels), row.channels),
        ("interlace", u64::from(info.interlaced), row.interlace),
        (
            "palette count",
            info.palette.as_ref().map_or(0, |p| p.len() as u64),
            row.plte,
        ),
        (
            "tRNS length",
            info.transparency.as_ref().map_or(0, |t| t.len() as u64),
            row.trns,
        ),
    ];
    for (what, got, want) in checks {
        if got != want {
            bad.push(format!("{what} {got} != {want}"));
        }
    }
    if info.inflated_len as u64 != row.inflated {
        bad.push(format!(
            "inflated length {} != {}",
            info.inflated_len, row.inflated
        ));
    }
    bad
}

// ---------------------------------------------------------------- sample table --

/// Every pinned row: the sample bytes, the header fields and the inflated length have to agree
/// with the reference decoder. All 111 rows are checked before the test reports.
#[test]
fn sample_rows_match_the_reference_decoder() -> Result<()> {
    let root = install()?;
    let table = reference_path("XP_PNG_REFERENCE", DEFAULT_SAMPLE);
    let rows = reference_table(&table)?;
    assert_eq!(rows.len(), 111, "pinned sample rows");

    let mut mismatches = Vec::new();
    for row in &rows {
        match decode_file(&root, &row.path) {
            Ok(info) => {
                let bad = compare(row, &info);
                if !bad.is_empty() {
                    mismatches.push(format!("{}: {}", row.path, bad.join("; ")));
                }
            }
            Err(err) => mismatches.push(format!("{}: rejected: {err:#}", row.path)),
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} of {} reference rows disagree:\n{}",
        mismatches.len(),
        rows.len(),
        mismatches.join("\n")
    );
    Ok(())
}

// ------------------------------------------------------------------- fixtures --

/// `Resources/default scenery/1000 roads/textures/rail_swizzle.png`: 1x64, colour type 2, 8-bit,
/// 3 bytes per row. All 192 sample bytes, so the Sub/Up filters and the one-pixel stride are
/// pinned exactly. This file also carries a 18 702-byte `iTXt`, so it fails a decoder whose
/// ancillary allowlist is shorter than "the first letter is lowercase".
const RAIL_SWIZZLE: &str = "1d9e15157f0f022001157f0f2cde220220012cde2224be1b063f030d5f08022001000100063f0302200100010024be1b0d5f080001000220010001001d9e15000100063f031d9e150220011d9e1502200124be1b2cde22000100063f03157f0f1d9e152cde220001002cde22063f0300010024be1b0d5f08000100157f0f063f03022001000100063f031d9e150d5f08157f0f0d5f0824be1b157f0f0d5f0802200100010024be1b0d5f08063f030001001d9e15157f0f0d5f082cde220d5f08";

/// First 64 sample bytes of `Aircraft/X-Planes/X-15/COCKPIT/generic/Pitch-1.png`: 106x45, 16-bit
/// RGBA. Each pixel is eight bytes, and the values (`139a`, `1388`, `136e`, ...) are different in
/// the two halves of every sample, so a decoder that emits little-endian 16-bit samples cannot
/// produce this.
const PITCH_16_HEAD: &str = "139a139a139affff138813881388ffff136e136e136effff134c134c134cffff\
    132013201320ffff12ee12ee12eeffff12b412b412b4ffff127812781278ffff";

/// First 64 sample bytes of `Resources/default scenery/1000 urban terrain/textures/border/soft.png`:
/// 128x64, 8-bit gray+alpha. Two bytes per pixel, gray first, then alpha: the pairing is what a
/// channel-order bug would break.
const SOFT_GRAY_ALPHA_HEAD: &str = "00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff02ff06\
    ff0cff10ff13ff14ff15ff16ff17ff18ff1aff1bff1bff1cff1eff1fff21ff22ff24ff";

/// First 22 pixels of rows 0 through 7 of `Aircraft/Fighters/Viggen JA37/Cockpit/standard six/
/// artificial horizons/horizon_small_adj-1.png`: 44x120, 8-bit RGBA, Adam7 interlaced. Row 0 is
/// written by passes 0, 1, 3 and 5, rows 1, 3, 5 and 7 by pass 6 alone, rows 2 and 6 by passes 4
/// and 5, and row 4 by passes 2, 3 and 5, so these eight rows pin where every pass puts pixels.
const HORIZON_INTERLACED_HEAD: [&str; 8] = [
    "fc0000ffed2120ffb5a29cffabaeaaffabadafffabadb0ffa6aba9ffa8aaa9ffbf787bffff0201fffc0807ffc2807bffaaaaadffb1aeb3ffa9afadffaaaaaaffadadaeffaaaba9ffa7afafffabb0aeffafabb0ffa9a9aeff",
    "ee1f1efffc0000ffed2020ffb3a19affaaa9aeffa8aaadffababa9ffaeafadffa8a8abffdb4a4cffff0201fffc0807ffc17d7cffa8b2afffaca9aaffadafacffaeacafffaea7aaffaeaaaeffaaa9abffadabafffacacabff",
    "b2a4a1ffee1f1ffffc0000ffed2120ffb69c9fffaea9acffabaeabffadada8ffb4a9a7ffa9a9aeffde4c4affff0201fffc0807ffc67a7affadadafffaba9b0ffacacadffb0abafffaea8aaffacb0acffaba9acffacacaaff",
    "abadabffb7a1a0ffee1e1efffc0000ffee1f1fffb89a9fffadaeafffabaea8ffacb2a9ffafafaeffadaaadffdc4c4affff0201fffc0807ffc57b7affadafa9ffa9aea9ffa9afadffadaea9ffaaabacffa9ababffa9a7a8ff",
    "aab4b0ffadaaafffb09f9fffed1f1efffc0000ffee1e1effb2a1a0ffaca9acffabb0adffafaaaeffaaaaaaffafaeb4ffdd4b4cffff0201fffc0807ffc77c7bffaea6b1ffafaaa9ffaca9abffaeafa9ffadaeadffabafb2ff",
    "b0b2aaffadadaaffa9ababffb49ea2ffee1f1ffffc0000ffee1e1effb39d9bffaaacacffadabaeffb1adaaffaeacacffacabacffdd484bffff0201fffc0807ffc37a79ffaaadabffa8aba7ffafaba9ffacacaaffadafabff",
    "acabaaffaeadaeffa8aeabffaeacb0ffb49d9effed1f1efffc0000ffee1e1fffafa09effabaeafffa9adabffaaaba7ffaba6a9ffa9abaaffde4a4affff0201fffc0807ffc3797affadb0aeffada9acffabb1a9ffaeaeabff",
    "acaeadffafacabffadabadffacada9ffa7aba9ffb39d9dffee1e1efffc0000ffee1e1dffb39c9effa9adafffaba8acffaeaea7ffada8aaffaaa9adffda4b48ffff0201fffc0807ffc17977ffaeacacffaeadb2ffabaab6ff",
];

/// The 1-bit indexed class: 20 files, all 50x25, two palette entries, one `tRNS` byte. Row stride
/// is `ceil(50 / 8) = 7`, so 25 rows are 175 bytes, and every pixel in this one is index 0.
const AP_ALT: &str = "Aircraft/General Aviation/KingAir C90B/cockpit_3D/generic/ap_alt.png";

#[test]
fn a_one_pixel_wide_rgb_file_is_byte_exact() -> Result<()> {
    let root = install()?;
    let rel = "Resources/default scenery/1000 roads/textures/rail_swizzle.png";
    let info = decode_file(&root, rel)?;
    assert_eq!((info.width, info.height), (1, 64), "dimensions");
    assert_eq!(
        (info.bit_depth, info.color_type, info.channels),
        (8, 2, 3),
        "8-bit RGB, three channels, no expansion to RGBA"
    );
    assert!(!info.interlaced, "not interlaced");
    assert_eq!(info.row_bytes(), 3, "stride for one pixel of RGB8");
    assert_eq!(info.samples.len(), 192, "64 rows of 3 bytes");
    assert_eq!(info.samples, unhex(RAIL_SWIZZLE)?, "every sample byte");
    assert_eq!(
        info.inflated_len, 256,
        "64 rows of a filter byte plus 3 bytes"
    );
    assert!(info.palette.is_none(), "no PLTE in this file");
    assert!(info.transparency.is_none(), "no tRNS in this file");
    Ok(())
}

#[test]
fn sixteen_bit_samples_stay_big_endian_two_bytes_per_channel() -> Result<()> {
    let root = install()?;
    let rel = "Aircraft/X-Planes/X-15/COCKPIT/generic/Pitch-1.png";
    let info = decode_file(&root, rel)?;
    assert_eq!((info.width, info.height), (106, 45), "dimensions");
    assert_eq!(
        (info.bit_depth, info.color_type, info.channels),
        (16, 6, 4),
        "16-bit RGBA"
    );
    assert_eq!(info.row_bytes(), 848, "106 px * 4 channels * 2 bytes");
    assert_eq!(info.samples.len(), 38_160, "45 rows of 848 bytes");
    assert_eq!(
        info.samples.len(),
        info.row_bytes() * 45,
        "stride times rows"
    );
    assert_eq!(
        info.samples[..64],
        unhex(PITCH_16_HEAD)?[..],
        "first eight pixels"
    );
    Ok(())
}

#[test]
fn gray_alpha_pairs_are_two_bytes_per_pixel_in_that_order() -> Result<()> {
    let root = install()?;
    let rel = "Resources/default scenery/1000 urban terrain/textures/border/soft.png";
    let info = decode_file(&root, rel)?;
    assert_eq!((info.width, info.height), (128, 64), "dimensions");
    assert_eq!(
        (info.bit_depth, info.color_type, info.channels),
        (8, 4, 2),
        "8-bit gray+alpha"
    );
    assert_eq!(info.row_bytes(), 256, "128 px * 2 bytes");
    assert_eq!(info.samples.len(), 16_384, "64 rows of 256 bytes");
    assert_eq!(
        info.samples[..64],
        unhex(SOFT_GRAY_ALPHA_HEAD)?[..],
        "first 32 pixels"
    );
    Ok(())
}

#[test]
fn interlaced_passes_interleave_into_full_image_rows() -> Result<()> {
    let root = install()?;
    let rel = "Aircraft/Fighters/Viggen JA37/Cockpit/standard six/artificial horizons/horizon_small_adj-1.png";
    let info = decode_file(&root, rel)?;
    assert_eq!((info.width, info.height), (44, 120), "dimensions");
    assert_eq!(
        (info.bit_depth, info.color_type, info.channels),
        (8, 6, 4),
        "8-bit RGBA"
    );
    assert!(info.interlaced, "IHDR interlace byte is 1");
    assert_eq!(info.row_bytes(), 176, "44 px * 4 bytes");
    assert_eq!(info.samples.len(), 21_120, "120 rows of 176 bytes");
    for (row, text) in HORIZON_INTERLACED_HEAD.iter().enumerate() {
        let start = row * 176;
        let want = unhex(text)?;
        assert_eq!(
            &info.samples[start..start + 88],
            &want[..],
            "first 22 pixels of row {row}"
        );
    }
    Ok(())
}

#[test]
fn indexed_files_stay_one_byte_per_pixel_with_their_palette() -> Result<()> {
    let root = install()?;
    let info = decode_file(&root, AP_ALT)?;
    assert_eq!((info.width, info.height), (50, 25), "dimensions");
    assert_eq!(
        (info.bit_depth, info.color_type, info.channels),
        (1, 3, 1),
        "1-bit indexed, one sample per pixel"
    );
    assert_eq!(info.row_bytes(), 7, "ceil(50 / 8), not 50");
    assert_eq!(
        info.samples,
        vec![0u8; 175],
        "every pixel is index 0, MSB-first"
    );
    let palette = info.palette.as_ref().context("this file carries a PLTE")?;
    assert_eq!(palette.len(), 2, "two palette entries");
    assert_eq!(
        &palette[..],
        &[[0x00, 0x00, 0x00], [0xff, 0xff, 0xff]],
        "black then white"
    );
    assert_eq!(
        info.transparency,
        Some(vec![0x00]),
        "index 0 is transparent"
    );
    assert_eq!(
        info.inflated_len, 200,
        "25 rows of a filter byte plus 7 bytes"
    );
    Ok(())
}

#[test]
fn a_two_by_two_rgb_file_is_twelve_bytes() -> Result<()> {
    let root = install()?;
    let info = decode_file(&root, "Resources/bitmaps/blank.png")?;
    assert_eq!(
        (info.width, info.height, info.channels),
        (2, 2, 3),
        "2x2 RGB8"
    );
    assert_eq!(
        info.samples,
        unhex("8b8b8b8b8b8b8b8b8b8b8b8b")?,
        "all twelve bytes"
    );
    assert_eq!(
        info.inflated_len, 14,
        "2 rows of a filter byte plus 6 bytes"
    );
    Ok(())
}

// ------------------------------------------------------------------- the corpus --

/// Every `.png` under the install, decoded. Pins the file count, the format histogram and the
/// fact that nothing fails, all measured from the shipped tree. When the full reference table is
/// present it also pins a SHA-256 over every file's sample digest, which is the corpus-wide
/// differential check against the reference decoder.
#[test]
fn every_shipped_png_decodes_and_matches_the_measured_histogram() -> Result<()> {
    let root = install()?;
    let paths = walk(&root, "png")?;
    assert_eq!(
        paths.len(),
        6862,
        "shipped .png files (.PNG is a different spelling)"
    );

    let mut decoded = 0usize;
    let mut hist: BTreeMap<(u8, u8, bool), usize> = BTreeMap::new();
    let mut failures: Vec<String> = Vec::new();
    let mut digests: Vec<String> = Vec::new();
    for path in &paths {
        let rel = relative(&root, path);
        let bytes = std::fs::read(path).with_context(|| format!("reading {rel}"))?;
        match png::decode(&bytes) {
            Ok(info) => {
                decoded += 1;
                *hist
                    .entry((info.bit_depth, info.color_type, info.interlaced))
                    .or_insert(0) += 1;
                digests.push(format!("{rel}\t{}", hex_sha256(&info.samples)));
            }
            Err(err) => failures.push(format!("{rel}: {err:#}")),
        }
    }

    assert!(
        failures.is_empty(),
        "{} files the reference decoder accepts and this one does not:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(decoded, 6862, "files decoded");

    let count = |bit_depth: u8, color_type: u8, interlaced: bool| {
        hist.get(&(bit_depth, color_type, interlaced))
            .copied()
            .unwrap_or(0)
    };
    assert_eq!(count(8, 6, false), 6071, "8-bit RGBA");
    assert_eq!(count(8, 2, false), 589, "8-bit RGB");
    assert_eq!(count(8, 0, false), 97, "8-bit gray");
    assert_eq!(count(16, 6, false), 41, "16-bit RGBA");
    assert_eq!(count(8, 6, true), 34, "interlaced 8-bit RGBA");
    assert_eq!(count(1, 3, false), 20, "1-bit indexed");
    assert_eq!(count(8, 4, false), 9, "8-bit gray+alpha");
    assert_eq!(count(8, 2, true), 1, "interlaced 8-bit RGB");
    assert_eq!(hist.len(), 8, "no other format appears");
    assert_eq!(
        hist.values().sum::<usize>(),
        6862,
        "every file counted once"
    );

    check_corpus_digest(&mut digests)?;
    Ok(())
}

/// The corpus-wide check: the set of paths and the digest over their sample hashes have to match
/// what the reference decoder produced for the same tree.
fn check_corpus_digest(digests: &mut [String]) -> Result<()> {
    let table = full_reference_path();
    if !table.is_file() {
        eprintln!(
            "SKIP corpus digest: the full reference table {} is not present",
            table.display()
        );
        return Ok(());
    }
    let rows = reference_table(&table)?;
    if rows.len() != 6862 {
        eprintln!(
            "SKIP corpus digest: {} has {} rows, expected 6862 (the reference run is still going)",
            table.display(),
            rows.len()
        );
        return Ok(());
    }

    // A digest only means something if both walks cover the same files.
    let mut reference: Vec<&str> = rows.iter().map(|row| row.path.as_str()).collect();
    reference.sort();
    let mut mine: Vec<&str> = digests
        .iter()
        .map(|line| line.split('\t').next().unwrap_or(""))
        .collect();
    mine.sort();
    assert_eq!(
        mine.len(),
        reference.len(),
        "files decoded vs files in the reference table"
    );
    let drift: Vec<String> = mine
        .iter()
        .zip(reference.iter())
        .filter(|(got, want)| got != want)
        .take(8)
        .map(|(got, want)| format!("{got} != {want}"))
        .collect();
    assert!(
        drift.is_empty(),
        "the walk and the reference table cover different files:\n{}",
        drift.join("\n")
    );

    digests.sort();
    let mut blob = String::new();
    for line in digests.iter() {
        blob.push_str(line);
        blob.push('\n');
    }
    let digest = hex_sha256(blob.as_bytes());

    let golden_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/png/corpus_digest.txt");
    let golden = read_golden(&golden_path)?;
    let want_rows = golden
        .get("rows")
        .with_context(|| format!("{} has no rows line", golden_path.display()))?;
    let want_digest = golden
        .get("digest")
        .with_context(|| format!("{} has no digest line", golden_path.display()))?;
    assert_eq!(digests.len().to_string(), *want_rows, "corpus rows");
    assert_eq!(
        digest, *want_digest,
        "SHA-256 over sorted path<TAB>sample-digest lines"
    );
    Ok(())
}

/// A `key: value` golden file, comments ignored.
fn read_golden(path: &Path) -> Result<BTreeMap<String, String>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading golden {}", path.display()))?;
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.find(": ") {
            Some(split) => (&line[..split], &line[split + 2..]),
            None => bail!("{}: bad golden line {line:?}", path.display()),
        };
        out.insert(key.trim().to_string(), value.trim().to_string());
    }
    Ok(out)
}

/// Path relative to the install root, the spelling the reference table uses.
fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
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
