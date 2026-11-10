//! ACF — X-Plane 10 aircraft description file (text, version 1004).
//!
//! Layout, measured from the shipped file (evidence E-0009 / E-0010 / E-0011):
//!
//! ```text
//! I                     <- line-ending marker: 'I' = IBM convention, 'M' = classic Mac
//! 1004 version          <- "<int> version"
//! ACF
//! PROPERTIES_BEGIN
//! P <key> <value>       <- 50 445 of these in the C172SP; arrays end with "<key>/count N"
//! PROPERTIES_END
//! PANEL_2D_BEGIN        <- "#Insts <id>, parent=<p>, ins_type=<t>" groups
//! PANEL_2D_END
//! PANEL_3D_BEGIN
//! PANEL_3D_END
//! ```
//!
//! Values are kept verbatim so a re-emit is byte-faithful; typed accessors parse on
//! demand. Property order is preserved in [`Acf::props`] because the original reads this
//! file sequentially and several consumers depend on that order.

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Context, Result};

/// One `P <key> <value>` property line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prop {
    /// Path-style key, e.g. `_wing/8/_chord_piv` or `acf/_Vne_kts`.
    pub key: String,
    /// Everything after `P <key> ` on the line, verbatim (may contain spaces).
    pub value: String,
}

/// A panel instance group: an optional `#Insts …` header, a name line at the same
/// indentation, then attributes indented two columns deeper.
///
/// Measured from shipped panels. Three shapes occur and all three are legal:
///
/// ```text
/// #Insts 0, parent=0, ins_type=350     <- header instance, indent 0
/// eng_rnd_FF eng_rnd_FF                <- name line, same indent as its header
///   POS 192.000000 150.000000          <- attributes, indent +2
///
/// GROUP Annunciators                   <- named group, indent 0
///   console_ott_GA console_autopilot_G <- anonymous instance, indent +2, no header
///     POS 500.999969 259.000000        <- attributes, indent +4
///
///   #Insts 4, parent=3, ins_type=802   <- header instance nested inside a group
///   gen_needle slip2
///     POS 838.000000 386.000000
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelInstance {
    /// Column the header (or the name line) starts at. Panels are indentation-scoped.
    pub indent: i64,
    /// False for the anonymous instances that appear inside a `GROUP` or directly under
    /// `PANEL_3D_BEGIN`, which carry no `#Insts` line at all.
    pub has_header: bool,
    pub id: i64,
    pub parent: i64,
    pub ins_type: i64,
    /// The line right after the header (or the name line itself), verbatim.
    pub name_line: String,
    /// `ATTR` -> raw value text, in file order. Unknown attributes are preserved.
    pub attrs: Vec<(String, String)>,
    /// 1-based line of the header or name line.
    pub line: usize,
}

/// A `GROUP <name>` line inside a panel block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelGroup {
    pub indent: i64,
    pub name: String,
    pub line: usize,
}

/// One `PANEL_2D_BEGIN`/`PANEL_3D_BEGIN` block.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PanelBlock {
    pub instances: Vec<PanelInstance>,
    pub groups: Vec<PanelGroup>,
    /// Lines that fit none of the shapes above, kept with their line numbers so a block is
    /// never silently truncated.
    pub other_lines: Vec<(usize, String)>,
}

impl PanelBlock {
    /// Instance count, the number the pre-existing evidence ledger recorded for a file.
    pub fn len(&self) -> usize {
        self.instances.len()
    }

    pub fn is_empty(&self) -> bool {
        self.instances.is_empty() && self.groups.is_empty() && self.other_lines.is_empty()
    }
}

/// A parsed ACF file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acf {
    /// `'I'` or `'M'`.
    pub line_ending_marker: String,
    /// The `<int>` from line 2, e.g. `1004`.
    pub version: u32,
    /// Every `P` line, in file order.
    pub props: Vec<Prop>,
    /// `PANEL_2D_BEGIN` block.
    pub panel_2d: PanelBlock,
    /// `PANEL_3D_BEGIN` block.
    pub panel_3d: PanelBlock,
    /// key -> position in [`Acf::props`]. Later duplicates win, matching a sequential
    /// reader that overwrites the previous slot.
    index: BTreeMap<String, usize>,
}

impl Acf {
    /// Read and parse a file.
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Acf> {
        let path = path.as_ref();
        let text = crate::read_text_lossy(path)
            .with_context(|| format!("reading ACF {}", path.display()))?;
        Acf::parse(&text).with_context(|| format!("parsing ACF {}", path.display()))
    }

    /// Parse ACF text. Accepts `\n`, `\r\n` and bare `\r` line endings.
    pub fn parse(text: &str) -> Result<Acf> {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() < 5 {
            bail!("ACF is too short ({} lines)", lines.len());
        }
        let marker = lines[0].trim_end().to_string();
        if marker != "I" && marker != "M" {
            bail!("line 1: expected ACF line-ending marker 'I' or 'M', got {marker:?}");
        }
        let version = parse_version_line(lines[1]).context("line 2")?;
        if lines[2].trim_end() != "ACF" {
            bail!("line 3: expected 'ACF', got {:?}", lines[2].trim_end());
        }

        let mut props: Vec<Prop> = Vec::new();
        let mut panel_2d = PanelBlock::default();
        let mut panel_3d = PanelBlock::default();

        let mut i = 3;
        while i < lines.len() {
            let line = lines[i].trim_end();
            i += 1;
            match line {
                "PROPERTIES_BEGIN" => parse_properties(&lines, &mut i, &mut props)?,
                "PANEL_2D_BEGIN" => parse_panel(&lines, &mut i, "PANEL_2D_END", &mut panel_2d)?,
                "PANEL_3D_BEGIN" => parse_panel(&lines, &mut i, "PANEL_3D_END", &mut panel_3d)?,
                "" => {}
                other => bail!("line {i}: unexpected top-level ACF line {other:?}"),
            }
        }

        let mut index: BTreeMap<String, usize> = BTreeMap::new();
        for (n, p) in props.iter().enumerate() {
            index.insert(p.key.clone(), n);
        }

        Ok(Acf {
            line_ending_marker: marker,
            version,
            props,
            panel_2d,
            panel_3d,
            index,
        })
    }

    /// The property with this exact key, if present.
    pub fn get(&self, key: &str) -> Option<&Prop> {
        self.index.get(key).map(|&n| &self.props[n])
    }

    /// Verbatim value text for `key`.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.get(key).map(|p| p.value.as_str())
    }

    /// `key` read as a float. The original reads ACF numbers with `scanf("%f")`, so this
    /// returns `f32`; the file's 9-digit decimals are f32 round-trips (e.g.
    /// `_z_ref 3.299999952` is `3.3_f32`).
    pub fn f32(&self, key: &str) -> Option<f32> {
        self.text(key).and_then(|v| v.trim().parse::<f32>().ok())
    }

    /// `key` read as a float, tolerating integer-shaped lines such as `acf/_num_engn 1`.
    pub fn f32_or_int(&self, key: &str) -> Option<f32> {
        self.f32(key).or_else(|| {
            self.text(key)
                .and_then(|v| v.trim().parse::<i64>().ok())
                .map(|i| i as f32)
        })
    }

    /// `key` read as an integer.
    pub fn i32(&self, key: &str) -> Option<i32> {
        self.text(key)
            .and_then(|v| v.trim().parse::<i64>().ok())
            .and_then(|v| i32::try_from(v).ok())
    }

    /// The `<prefix>/count N` terminator for an array, if present.
    pub fn count(&self, prefix: &str) -> Option<usize> {
        self.text(&format!("{prefix}/count"))
            .and_then(|v| v.trim().parse::<usize>().ok())
    }

    /// Properties under `prefix/…` in file order.
    pub fn children(&self, prefix: &str) -> Vec<&Prop> {
        let needle = format!("{prefix}/");
        self.props
            .iter()
            .filter(|p| p.key.starts_with(&needle))
            .collect()
    }

    /// All properties whose key's last segment is `leaf`, in file order.
    pub fn by_leaf(&self, leaf: &str) -> Vec<&Prop> {
        self.props
            .iter()
            .filter(|p| p.key.rsplit('/').next() == Some(leaf))
            .collect()
    }

    /// Number of distinct key shapes, with every numeric path segment replaced by `#`.
    /// Recorded in the C172SP golden as a structural fingerprint of the parser.
    pub fn distinct_key_shapes(&self) -> usize {
        let mut shapes: Vec<String> = self.props.iter().map(|p| normalize_key(&p.key)).collect();
        shapes.sort();
        shapes.dedup();
        shapes.len()
    }

    /// Number of distinct final path segments among `acf/*` keys.
    pub fn distinct_acf_leaves(&self) -> usize {
        let mut leaves: Vec<&str> = self
            .props
            .iter()
            .filter(|p| p.key.starts_with("acf/"))
            .filter_map(|p| p.key.rsplit('/').next())
            .collect();
        leaves.sort_unstable();
        leaves.dedup();
        leaves.len()
    }

    /// Classify every property value as integer-, float-, or string-shaped.
    pub fn value_classes(&self) -> (usize, usize, usize) {
        let (mut ints, mut floats, mut strs) = (0usize, 0usize, 0usize);
        for p in &self.props {
            let v = p.value.trim();
            if v.parse::<i64>().is_ok() {
                ints += 1;
            } else if v.parse::<f64>().is_ok() {
                floats += 1;
            } else {
                strs += 1;
            }
        }
        (ints, floats, strs)
    }
}

/// `_wing/8/_co_xyz/0,0` -> `_wing/#/_co_xyz/#,#`.
///
/// A key is `/`-separated; inside a segment ACF packs paired components with commas
/// (`_co_xyz/0,2`). Every numeric component collapses to `#` independently, so sibling
/// components of the same element share one shape while a name never collapses.
pub fn normalize_key(key: &str) -> String {
    key.split('/')
        .map(|seg| {
            seg.split(',')
                .map(|part| {
                    if is_numeric_component(part) {
                        "#"
                    } else {
                        part
                    }
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// True for an optional sign followed by at least one ASCII digit.
fn is_numeric_component(part: &str) -> bool {
    let bytes = part.as_bytes();
    let body = if bytes.first() == Some(&b'-') {
        &bytes[1..]
    } else {
        bytes
    };
    !body.is_empty() && body.iter().all(|c| c.is_ascii_digit())
}

fn parse_version_line(line: &str) -> Result<u32> {
    let mut it = line.split_whitespace();
    let (Some(num), Some(word), None) = (it.next(), it.next(), it.next()) else {
        bail!("expected \"<int> version\", got {line:?}");
    };
    if word != "version" {
        bail!("expected \"<int> version\", got {line:?}");
    }
    num.parse::<u32>()
        .with_context(|| format!("bad ACF version {num:?}"))
}

/// Consume `P <key> <value>` lines up to `PROPERTIES_END`.
fn parse_properties(lines: &[&str], i: &mut usize, props: &mut Vec<Prop>) -> Result<()> {
    while *i < lines.len() {
        let line = lines[*i].trim_end();
        *i += 1;
        match line {
            "PROPERTIES_END" => return Ok(()),
            "" => continue,
            _ => {}
        }
        let rest = line.strip_prefix("P ").ok_or_else(|| {
            anyhow!(
                "line {}: inside PROPERTIES, expected 'P …', got {line:?}",
                *i
            )
        })?;
        let (key, value) = rest.split_once(' ').unwrap_or((rest, ""));
        if key.is_empty() {
            bail!("line {}: empty property key", *i);
        }
        props.push(Prop {
            key: key.to_string(),
            value: value.to_string(),
        });
    }
    bail!("unterminated PROPERTIES block")
}

/// Consume one panel block. Nesting is expressed by indentation, not by markers, so the
/// walk keeps a stack of open scopes and never bails on a shape it does not recognize.
fn parse_panel(
    lines: &[&str],
    i: &mut usize,
    end_marker: &str,
    out: &mut PanelBlock,
) -> Result<()> {
    /// An open scope: its column and what kind of thing it is.
    #[derive(Clone, Copy)]
    enum Frame {
        Root,
        Group,
        /// `true` while the name line that follows a `#Insts` header is still owed.
        Inst(bool),
    }

    let mut stack: Vec<(i64, Frame)> = vec![(-2, Frame::Root)];

    while *i < lines.len() {
        let raw = lines[*i].trim_end();
        let line_no = *i + 1;
        *i += 1;

        if raw.trim_end() == end_marker {
            return Ok(());
        }
        if raw.trim().is_empty() {
            continue;
        }

        let indent = raw.chars().take_while(|c| *c == ' ').count() as i64;
        let body = raw.trim_start().to_string();

        while stack.last().is_some_and(|(ind, _)| *ind > indent) {
            stack.pop();
        }
        let (top_indent, top_frame) = stack.last().cloned().unwrap_or((-2, Frame::Root));

        // A name line sits at the same column as the `#Insts` header it belongs to.
        if let (ti, Frame::Inst(true)) = (top_indent, top_frame) {
            if indent == ti {
                if let Some(idx) = last_instance(&out.instances) {
                    out.instances[idx].name_line = body.clone();
                }
                stack.pop();
                stack.push((ti, Frame::Inst(false)));
                continue;
            }
        }

        if let Some(rest) = body.strip_prefix("#Insts ") {
            let (id, parent, ins_type) = parse_insts_header(rest)
                .with_context(|| format!("line {line_no}: bad #Insts header {body:?}"))?;
            out.instances.push(PanelInstance {
                indent,
                has_header: true,
                id,
                parent,
                ins_type,
                name_line: String::new(),
                attrs: Vec::new(),
                line: line_no,
            });
            stack.pop();
            stack.push((indent, Frame::Inst(true)));
            continue;
        }

        if let Some(rest) = body.strip_prefix("GROUP ") {
            out.groups.push(PanelGroup {
                indent,
                name: rest.trim().to_string(),
                line: line_no,
            });
            stack.pop();
            stack.push((indent, Frame::Group));
            continue;
        }

        // Attributes are indented two columns past the instance they belong to.
        if matches!(top_frame, Frame::Inst(false)) && indent == top_indent + 2 {
            if let Some(idx) = last_instance(&out.instances) {
                let (attr, value) = body.split_once(' ').unwrap_or((body.as_str(), ""));
                out.instances[idx]
                    .attrs
                    .push((attr.to_string(), value.to_string()));
                continue;
            }
        }

        // Anything else at the child column is an anonymous instance (no `#Insts` header).
        if indent == top_indent + 2 {
            out.instances.push(PanelInstance {
                indent,
                has_header: false,
                id: -1,
                parent: -1,
                ins_type: -1,
                name_line: body.clone(),
                attrs: Vec::new(),
                line: line_no,
            });
            stack.pop();
            stack.push((indent, Frame::Inst(false)));
            continue;
        }

        out.other_lines.push((line_no, raw.to_string()));
    }
    bail!("unterminated panel block (looking for {end_marker:?})")
}

/// Index of the most recently added instance, if any.
fn last_instance(list: &[PanelInstance]) -> Option<usize> {
    if list.is_empty() {
        None
    } else {
        Some(list.len() - 1)
    }
}

/// `0, parent=0, ins_type=350` -> `(0, 0, 350)`.
fn parse_insts_header(s: &str) -> Result<(i64, i64, i64)> {
    let mut vals: [Option<i64>; 3] = [None, None, None];
    for part in s.split(',') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("parent=") {
            vals[1] = Some(v.parse::<i64>().with_context(|| format!("parent={v}"))?);
        } else if let Some(v) = part.strip_prefix("ins_type=") {
            vals[2] = Some(v.parse::<i64>().with_context(|| format!("ins_type={v}"))?);
        } else {
            vals[0] = Some(
                part.parse::<i64>()
                    .with_context(|| format!("inst id {part:?}"))?,
            );
        }
    }
    let [Some(id), Some(parent), Some(ins_type)] = vals else {
        bail!("missing id/parent/ins_type in {s:?}");
    };
    Ok((id, parent, ins_type))
}

/// The size every fixed-record legacy binary ACF in this install reports.
pub const LEGACY_BINARY_SIZE: u64 = 2_582_001;

/// Recognize the legacy binary ACF variant, which this port detects but does not model.
///
/// Measured on the shipped install (X-Plane 10.51r2, 59 `.acf` files total):
///
/// * 40 are the text format this module parses — line 1 is `I` or `M`.
/// * 19 are not text at all. All 19 sit under `Extra Aircraft/` — 4 in
///   `Aircraft From Previous Versions/`, 15 in `Experimental/`. 17 of them are exactly
///   [`LEGACY_BINARY_SIZE`] bytes and start `69 E8 03 00 00`; two are a few KB larger.
///   None of the 19 contains a `0x0A` in its first 64 bytes.
/// * Inside them, printable runs sit at offsets ≡ 1 (mod 4) (285 of 287 in one sample), i.e.
///   the record stream is 4-byte aligned from offset 1, not offset 0. Bytes 1..5 read as the
///   little-endian u32 `1000`, and offsets 10 and 14 read as the little-endian f32 `1.0`.
/// * Some `char[]` fields are stored with each 4-byte word byte-reversed and some are not: in
///   one sample `DeathTrap.acf` appears as `taeDarThca.p…f` while a sibling file's
///   `B-1B Lancer4.acf` appears verbatim.
///
/// That is enough to tell "a different serialization" from "a broken text ACF", which is what
/// the content checker needs. It is not a model of the format: the field layout, the swap rule,
/// and whether the original even loads these files are all still unmeasured — see
/// `docs/COMPATIBILITY.md`.
pub fn looks_like_legacy_binary(head: &[u8], len: u64) -> bool {
    len >= 2_000_000 && !head.iter().take(64).any(|b| *b == b'\n')
}

/// [`looks_like_legacy_binary`] for a file on disk.
///
/// A read failure is reported as "not legacy binary" rather than as an error: the caller has
/// already failed to parse the file, and misclassifying an I/O problem as a format problem
/// would hide it.
pub fn is_legacy_binary_file(path: impl AsRef<std::path::Path>) -> bool {
    let path = path.as_ref();
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(bytes) = std::fs::File::open(path).and_then(|mut f| {
        use std::io::Read;
        let mut buf = vec![0u8; 64];
        let n = f.read(&mut buf)?;
        buf.truncate(n);
        Ok(buf)
    }) else {
        return false;
    };
    looks_like_legacy_binary(&bytes, meta.len())
}
