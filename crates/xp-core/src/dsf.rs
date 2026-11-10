//! DSF — X-Plane 10 terrain/scenery patch files (`*.dsf`).
//!
//! Everything here was measured from the 1 747 shipped `.dsf` files, because the pre-existing
//! evidence ledger described only the first two chunks and got the size semantics wrong.
//!
//! ```text
//! "XPLNEDSF"            magic
//! u32 version           every shipped file is 1
//! chunk*                tag(4) + u32 size, where SIZE INCLUDES ITS OWN 8-BYTE HEADER
//!   └ nested chunk*     HEAD wraps the rest: PROP, TERT, OBJT, POLY, NETW, DEMN and the
//!                       POOL/SCAL pairs all live inside HEAD's payload, so the tree is real
//! 16-byte trailer       MD5 of every byte before it (verified on 200/200 sampled files)
//! ```
//!
//! Chunk tags are stored **byte-reversed** relative to their logical names: the file holds
//! `DAEH`/`PORP`/`TJBO`, the original's own strings hold `HEAD`/`PROP`/`LAND`. The logical
//! names measured across the corpus are `HEAD`, `PROP`, `DEFN`, `TERT`, `OBJT`, `POLY`,
//! `NETW`, `GEOD`, `CMDS`, `DEMN`, `POOL`, `SCAL`, `PO32`, `SC32`.
//!
//! The `OBJT` chunk payload is a flat list of NUL-terminated library names — the object
//! references this terrain patch draws — which is what makes this reader useful to the content
//! checker before any mesh decoding exists.
//!
//! 12 shipped files are not DSF at all: `Custom Scenery/KSEA Demo Terrain/**` are **7z
//! archives** (`37 7A BC AF 27 1C`), i.e. compressed patches. They are classified, not
//! reported as corrupt; decompression is not implemented.
//!
//! Chunk *semantics* beyond `OBJT` are still unrecovered. [`Dsf::unmodeled_bytes`] reports how
//! much of a file that leaves, so "parsed" is never mistaken for "understood".

use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};

/// Magic bytes of an uncompressed DSF.
pub const MAGIC: &[u8; 8] = b"XPLNEDSF";
/// Magic bytes of a 7z-compressed patch.
pub const MAGIC_7Z: &[u8; 8] = b"7z\xbc\xaf'\x1c\x00\x00";
/// The trailer is an MD5 digest of everything before it.
pub const TRAILER_LEN: usize = 16;

/// What kind of `.dsf` file this is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DsfKind {
    /// Plain `XPLNEDSF` patch.
    Plain { version: u32 },
    /// 7z-compressed patch. Decompression is not implemented.
    Compressed7z,
    /// Neither.
    Unknown,
}

/// Classify by leading bytes.
pub fn classify(bytes: &[u8]) -> DsfKind {
    if bytes.len() < 12 {
        return DsfKind::Unknown;
    }
    if &bytes[..8] == MAGIC {
        return DsfKind::Plain {
            // A truncated or corrupt version field is reported, not guessed at.
            version: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        };
    }
    if bytes[..6] == MAGIC_7Z[..6] {
        return DsfKind::Compressed7z;
    }
    DsfKind::Unknown
}

/// One chunk. `name` is the logical (byte-reversed) tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Logical name, e.g. `HEAD` for the file bytes `DAEH`.
    pub name: String,
    /// Offset of the chunk header in the file.
    pub offset: usize,
    /// Total length including this chunk's own 8-byte header.
    pub size: usize,
    /// Chunks nested inside this one's payload.
    pub children: Vec<Chunk>,
}

impl Chunk {
    /// Length of this chunk's own payload, after nested children are subtracted by the
    /// caller walking [`Chunk::children`].
    pub fn payload_len(&self) -> usize {
        self.size.saturating_sub(8)
    }

    /// Find the first nested child with this logical name.
    pub fn child(&self, name: &str) -> Option<&Chunk> {
        self.children.iter().find(|c| c.name == name)
    }
}

/// A parsed (or classified) `.dsf` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dsf {
    pub kind: DsfKind,
    /// Top-level chunks, in file order.
    pub chunks: Vec<Chunk>,
    /// Bytes between the end of the last chunk and the trailer, if any.
    pub gap_bytes: usize,
    /// Trailer length actually found.
    pub trailer_len: usize,
    /// Whether the trailer equals the MD5 of the preceding bytes.
    pub trailer_is_md5: bool,
    /// Chunk bytes whose semantics this reader does not model. Everything except the `OBJT`
    /// name table counts as unmodeled.
    pub unmodeled_bytes: usize,
    /// Library names referenced by the `OBJT` chunk, in file order.
    pub object_names: Vec<String>,
}

impl Dsf {
    /// Read and parse a `.dsf` file.
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Dsf> {
        let path = path.as_ref();
        let bytes =
            std::fs::read(path).with_context(|| format!("reading DSF {}", path.display()))?;
        Dsf::parse(&bytes).with_context(|| format!("parsing DSF {}", path.display()))
    }

    /// Parse `.dsf` bytes. A compressed or unrecognized file yields a [`DsfKind`] with no
    /// chunks rather than an error, so a content walk can report it and keep going.
    pub fn parse(bytes: &[u8]) -> Result<Dsf> {
        let DsfKind::Plain { version } = classify(bytes) else {
            return Ok(Dsf {
                kind: classify(bytes),
                chunks: Vec::new(),
                gap_bytes: 0,
                trailer_len: 0,
                trailer_is_md5: false,
                unmodeled_bytes: bytes.len(),
                object_names: Vec::new(),
            });
        };

        let total = bytes.len();
        if total < 12 + TRAILER_LEN {
            bail!("file is too short to hold a header and an MD5 trailer ({total} bytes)");
        }

        let body_end = total - TRAILER_LEN;
        let chunks = walk_chunks(bytes, 12, body_end, 0)?;

        // Account for every byte: chunks, then any gap, then the trailer.
        let last_end = chunks.iter().map(|c| c.offset + c.size).max().unwrap_or(12);
        let gap_bytes = body_end.saturating_sub(last_end);

        let trailer = &bytes[body_end..];
        let trailer_is_md5 = md5(&bytes[..body_end]) == trailer;

        // Top-level sizes already contain their nested children, so unmodeled bytes are the
        // top-level total minus whatever this reader actually decodes.
        let mut unmodeled: usize = chunks.iter().map(|c| c.size).sum();

        let mut object_names = Vec::new();
        if let Some(objt) = find_chunk(&chunks, "OBJT") {
            object_names = read_name_table(bytes, objt.offset + 8, objt.offset + objt.size)?;
            unmodeled = unmodeled.saturating_sub(objt.size);
        }
        unmodeled += gap_bytes;

        Ok(Dsf {
            kind: DsfKind::Plain { version },
            chunks,
            gap_bytes,
            trailer_len: TRAILER_LEN,
            trailer_is_md5,
            unmodeled_bytes: unmodeled,
            object_names,
        })
    }

    /// Logical chunk name → count, over the whole tree.
    pub fn chunk_counts(&self) -> BTreeMap<String, usize> {
        let mut m: BTreeMap<String, usize> = BTreeMap::new();
        fn visit(c: &Chunk, m: &mut BTreeMap<String, usize>) {
            *m.entry(c.name.clone()).or_default() += 1;
            for k in &c.children {
                visit(k, m);
            }
        }
        for c in &self.chunks {
            visit(c, &mut m);
        }
        m
    }

    /// Find a chunk anywhere in the tree by logical name.
    ///
    /// `OBJT` is nested inside `HEAD` in every shipped patch, so a top-level-only search
    /// finds nothing.
    pub fn find(&self, name: &str) -> Option<&Chunk> {
        find_chunk(&self.chunks, name)
    }

    /// Total bytes covered by top-level chunks (nested chunks are already included).
    pub fn chunk_bytes(&self) -> usize {
        self.chunks.iter().map(|c| c.size).sum()
    }

    /// Validate the file the way the original does, reporting what is wrong rather than
    /// failing the walk.
    pub fn diagnostics(&self, name: &str) -> Vec<String> {
        let mut out = Vec::new();
        match self.kind {
            DsfKind::Compressed7z => out.push(format!(
                "The patch {name} is a 7z-compressed DSF; decompression is not implemented"
            )),
            DsfKind::Unknown => out.push(format!("The patch {name} is not an XPLNEDSF DSF")),
            DsfKind::Plain { version } => {
                if version != 1 {
                    out.push(format!(
                        "The patch {name} reports DSF version {version}; this reader was measured against version 1"
                    ));
                }
                if !self.trailer_is_md5 {
                    out.push(format!(
                        "The patch {name} has a trailer that is not the MD5 of the preceding bytes"
                    ));
                }
                if self.gap_bytes != 0 {
                    out.push(format!(
                        "The patch {name} leaves {} bytes between the last chunk and the trailer",
                        self.gap_bytes
                    ));
                }
            }
        }
        out
    }
}

/// Depth-first search for the first chunk with this logical name.
fn find_chunk<'a>(list: &'a [Chunk], name: &str) -> Option<&'a Chunk> {
    for c in list {
        if c.name == name {
            return Some(c);
        }
        if let Some(found) = find_chunk(&c.children, name) {
            return Some(found);
        }
    }
    None
}

/// Walk sibling chunks in `[from, to)`. `size` counts the chunk's own header.
fn walk_chunks(bytes: &[u8], from: usize, to: usize, depth: usize) -> Result<Vec<Chunk>> {
    let mut out = Vec::new();
    let mut off = from;
    while off + 8 <= to {
        let tag = &bytes[off..off + 4];
        if !tag.iter().all(|c| c.is_ascii_graphic()) {
            bail!(
                "offset {off}: expected a 4-character chunk tag, got {:?}",
                String::from_utf8_lossy(tag)
            );
        }
        let size = u32::from_le_bytes([
            bytes[off + 4],
            bytes[off + 5],
            bytes[off + 6],
            bytes[off + 7],
        ]) as usize;
        if size < 8 {
            bail!("offset {off}: chunk length {size} is smaller than its own 8-byte header");
        }
        if off + size > to {
            bail!(
                "offset {off}: chunk of {size} bytes runs past its parent (ends {}, limit {to})",
                off + size
            );
        }
        let name: String = tag.iter().rev().map(|c| *c as char).collect();
        let children = if depth < 8 {
            walk_nested(bytes, off + 8, off + size, depth + 1)?
        } else {
            Vec::new()
        };
        out.push(Chunk {
            name,
            offset: off,
            size,
            children,
        });
        off += size;
    }
    Ok(out)
}

/// A parent's payload may itself begin with chunk headers (HEAD wraps PROP and DEFN), so
/// nesting is detected by trying to read a chunk and only accepting it if it fits.
fn walk_nested(bytes: &[u8], from: usize, to: usize, depth: usize) -> Result<Vec<Chunk>> {
    if from + 8 > to {
        return Ok(Vec::new());
    }
    let tag = &bytes[from..from + 4];
    if !tag.iter().all(|c| c.is_ascii_graphic()) {
        return Ok(Vec::new());
    }
    let size = u32::from_le_bytes([
        bytes[from + 4],
        bytes[from + 5],
        bytes[from + 6],
        bytes[from + 7],
    ]) as usize;
    if size < 8 || from + size > to {
        return Ok(Vec::new());
    }
    walk_chunks(bytes, from, to, depth)
}

/// Read the NUL-terminated name list in an `OBJT` payload.
fn read_name_table(bytes: &[u8], from: usize, to: usize) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut off = from;
    while off < to {
        let end = bytes[off..to]
            .iter()
            .position(|c| *c == 0)
            .map(|p| off + p)
            .unwrap_or(to);
        let raw = &bytes[off..end];
        if raw.is_empty() {
            off = end + 1;
            continue;
        }
        out.push(
            std::str::from_utf8(raw)
                .with_context(|| format!("offset {off}: OBJT name is not UTF-8"))?
                .to_string(),
        );
        if end >= to {
            break;
        }
        off = end + 1;
    }
    Ok(out)
}

/// Dependency-free MD5 (RFC 1321), used to verify the DSF trailer.
///
/// Public so a test can pin it to the RFC 1321 vectors; without that guard the corpus-wide
/// "the trailer is the MD5" claim would be unverifiable.
pub fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: [u32; 64] = {
        let mut t = [0u32; 64];
        let mut i = 0;
        while i < 64 {
            t[i] = ((i as f64 + 1.0).sin().abs() * (2u64.pow(32) as f64)) as u32;
            i += 1;
        }
        t
    };

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());

    let mut a0: u32 = 0x6745_2301;
    let mut b0: u32 = 0xefcd_ab89;
    let mut c0: u32 = 0x98ba_dcfe;
    let mut d0: u32 = 0x1032_5476;

    for chunk in msg.chunks(64) {
        let mut m = [0u32; 16];
        for i in 0..16 {
            m[i] = u32::from_le_bytes([
                chunk[4 * i],
                chunk[4 * i + 1],
                chunk[4 * i + 2],
                chunk[4 * i + 3],
            ]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | ((!b) & d), i),
                1 => ((d & b) | ((!d) & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | (!d)), (7 * i) % 16),
            };
            let sum = a.wrapping_add(f).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(sum.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out
}
