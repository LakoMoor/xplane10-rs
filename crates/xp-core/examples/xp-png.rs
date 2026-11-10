//! Inflate one shipped PNG's IDAT stream and report where it diverges from a reference dump.
//!
//! Usage: `cargo run -p xp-core --example xp-png -- <png> [reference-raw.bin]`
//!
//! With no reference it prints the zlib header, the sample-array length and a checksum of the
//! inflated bytes. With a reference file it finds the first differing byte, which is what localises
//! a deflate bug: "the output is wrong from byte N" tells you which block you are in.

use std::io::Write as _;

use anyhow::{bail, Context as _, Result};
use xp_core::png;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next().map(std::path::PathBuf::from) else {
        bail!("usage: xp-png <png> [reference-raw.bin]");
    };
    let reference = args.next().map(std::path::PathBuf::from);

    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let info = png::decode(&bytes).with_context(|| format!("decoding {}", path.display()))?;

    println!(
        "{}: {}x{} depth {} ctype {} interlaced {} -> {} sample bytes (inflate {} bytes)",
        path.display(),
        info.width,
        info.height,
        info.bit_depth,
        info.color_type,
        info.interlaced,
        info.samples.len(),
        info.inflated_len,
    );

    let mut digest: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in &info.samples {
        digest ^= u64::from(b);
        digest = digest.wrapping_mul(0x100_0000_01b3);
    }
    println!("samples fnv1a64 : {digest:016x}");

    if let Some(reference) = reference {
        let want = std::fs::read(&reference)
            .with_context(|| format!("reading {}", reference.display()))?;
        if want.len() != info.samples.len() {
            println!(
                "length differs: reference {} vs port {}",
                want.len(),
                info.samples.len()
            );
        }
        let n = want.len().min(info.samples.len());
        match want[..n]
            .iter()
            .zip(&info.samples[..n])
            .position(|(a, b)| a != b)
        {
            Some(at) => {
                let lo = at.saturating_sub(8);
                println!(
                    "first difference at byte {at}: reference {:02x?} vs port {:02x?}",
                    &want[lo..(at + 8).min(n)],
                    &info.samples[lo..(at + 8).min(n)]
                );
            }
            None => println!("no difference in the first {n} bytes"),
        }
    }

    // The raw inflated stream, for diffing against a zlib dump.
    let out = std::env::var("XP_PNG_RAW");
    if let Ok(out) = &out {
        let mut f = std::fs::File::create(out).with_context(|| format!("creating {out}"))?;
        f.write_all(&info.samples)?;
        println!("wrote {out}");
    }
    Ok(())
}
