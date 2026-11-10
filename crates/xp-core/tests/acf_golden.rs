//! Golden tests for the ACF parser.
//!
//! Every expectation below comes from `tests/golden/acf/c172sp_summary.txt`, which was
//! extracted from the shipped `Cessna_172SP.acf` — not from this parser.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use xp_core::acf::{self, Acf};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden")
}

/// Read a golden file into a key -> value map. `#` lines are comments.
fn read_golden(path: &Path) -> Result<BTreeMap<String, String>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading golden {}", path.display()))?;
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // "key: value" (stats) and "key=value" (verbatim scalars) both appear; a stat value
        // may itself contain '=', so the colon form is matched first.
        let (key, value) = match line.find(": ") {
            Some(sep) => (&line[..sep], line[sep + 2..].trim()),
            None => match line.find('=') {
                Some(sep) => (&line[..sep], line[sep + 1..].trim()),
                None => {
                    return Err(anyhow::anyhow!(
                        "{}: golden line is not key=value: {line:?}",
                        path.display()
                    ))
                }
            },
        };
        let key = key.to_string();
        let value = value.to_string();
        out.insert(key, value);
    }
    if out.is_empty() {
        bail!("golden {} is empty", path.display());
    }
    Ok(out)
}

fn expect(golden: &BTreeMap<String, String>, acf: &Acf, key: &str) -> Result<()> {
    let want = golden
        .get(key)
        .with_context(|| format!("golden has no key {key:?}"))?;
    let got = match acf.text(key) {
        Some(v) => v.to_string(),
        None => bail!("ACF is missing {key:?} (golden wants {want:?})"),
    };
    if &got != want {
        bail!("ACF {key:?} = {got:?}, golden wants {want:?}");
    }
    Ok(())
}

/// The trimmed fixture must parse and keep the exact scalar text it was built from.
#[test]
fn fixture_slice_parses_with_verbatim_values() -> Result<()> {
    let path = golden_dir().join("fixtures/c172sp_slice.acf");
    let acf = Acf::load(&path)?;
    assert_eq!(acf.version, 1004);
    assert_eq!(acf.line_ending_marker, "I");
    assert!(!acf.props.is_empty());

    // verbatim round-trip: the stored value text is exactly what follows `P <key> `
    assert_eq!(acf.text("acf/_Vne_kts"), Some("163.0"));
    assert_eq!(acf.text("acf/_Gpos"), Some("4.500000000"));
    assert_eq!(acf.text("_wing/8/_co_xyz/0,0"), Some("-1.875215173"));
    assert_eq!(acf.text("_cgpt/1/_name"), Some("fuel tank #1"));

    // f32 semantics: the 9-digit decimals are f32 round-trips of short decimals
    assert_eq!(acf.f32("_cgpt/0/_z_ref"), Some(3.3_f32));
    assert_eq!(acf.f32("_wing/8/_chord_for_RIB/0"), Some(5.2_f32));
    assert_eq!(acf.f32("_wing/8/_incidence/0"), Some(3.0_f32));
    assert_eq!(acf.f32("_wing/8/_is_right_mult"), Some(-1.0_f32));

    // integer-shaped scalars are readable as ints and as floats
    assert_eq!(acf.i32("acf/_num_engn"), Some(1));
    assert_eq!(acf.f32_or_int("acf/_num_engn"), Some(1.0_f32));

    // array terminators
    assert_eq!(acf.count("_cgpt"), Some(8));
    assert_eq!(acf.count("_wing"), Some(56));
    assert_eq!(acf.count("_gear"), Some(10));

    // panel groups survive with unknown attributes preserved
    assert!(!acf.panel_2d.instances.is_empty());
    let first = &acf.panel_2d.instances[0];
    assert_eq!((first.id, first.parent, first.ins_type), (0, 0, 350));
    assert_eq!(first.name_line, "eng_rnd_FF eng_rnd_FF");
    assert_eq!(
        first.attrs[0],
        ("POS".to_string(), "192.000000 150.000000".to_string())
    );
    Ok(())
}

/// Key normalization: numeric and comma-paired segments collapse, names do not.
#[test]
fn key_normalization_matches_the_originals_indexing() {
    assert_eq!(
        acf::normalize_key("_wing/8/_chord_piv"),
        "_wing/#/_chord_piv"
    );
    assert_eq!(
        acf::normalize_key("_wing/8/_co_xyz/0,2"),
        "_wing/#/_co_xyz/#,#"
    );
    assert_eq!(acf::normalize_key("acf/_Vne_kts"), "acf/_Vne_kts");
    assert_eq!(
        acf::normalize_key("_engn/0/_exhaust_os_xyz/count"),
        "_engn/#/_exhaust_os_xyz/count"
    );
}

/// The whole shipped C172SP must reproduce the extracted summary golden.
#[test]
fn full_c172sp_matches_golden() -> Result<()> {
    let Some(root) = xp_core::install_root() else {
        eprintln!("SKIP full_c172sp_matches_golden: XP_INSTALL is not set (no coverage claimed)");
        return Ok(());
    };
    let acf_path = root.join("Aircraft/General Aviation/Cessna 172SP/Cessna_172SP.acf");
    if !acf_path.is_file() {
        eprintln!(
            "SKIP full_c172sp_matches_golden: {} missing",
            acf_path.display()
        );
        return Ok(());
    }
    let golden = read_golden(&golden_dir().join("acf/c172sp_summary.txt"))?;
    let acf = Acf::load(&acf_path)?;

    assert_eq!(acf.version, 1004);
    assert_eq!(acf.props.len(), golden["p_lines"].parse::<usize>()?);
    assert_eq!(
        acf.distinct_key_shapes(),
        golden["distinct_normalized_key_shapes"].parse::<usize>()?
    );
    assert_eq!(
        acf.distinct_acf_leaves(),
        golden["distinct_acf_leaves"].parse::<usize>()?
    );
    assert_eq!(
        acf.panel_2d.len(),
        golden["panel_2d_instances"].parse::<usize>()?
    );
    assert_eq!(
        acf.panel_3d.len(),
        golden["panel_3d_instances"].parse::<usize>()?
    );

    let (ints, floats, strs) = acf.value_classes();
    assert_eq!(
        format!("int={ints} float={floats} string={strs}"),
        golden["value_classes"],
        "value classification drifted"
    );

    // every remaining golden entry is a verbatim scalar value
    let skip = [
        "value_classes",
        "shapes",
        "leaves",
        "instances",
        "p_lines",
        "version",
        "lines",
    ];
    let keys: Vec<String> = golden
        .keys()
        .filter(|k| !skip.iter().any(|frag| k.contains(frag)))
        .cloned()
        .collect();
    for key in &keys {
        expect(&golden, &acf, key)?;
    }
    Ok(())
}
