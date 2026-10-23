#![cfg(feature = "hdf")]
//! Structure result families: env-gated real-file oracle comparison (h5py, bit-exact) and
//! synthetic malformed fixtures (generator tests/fixtures/structures/make_fixtures.py).
use ras_commander_rust::hdf::structures::{ColumnLabelSource, StructureFamilyKind as K};
use ras_commander_rust::hdf::DEFAULT_MAX_BYTES;
use ras_commander_rust::hdf::{NumericKind, NumericValues, ReadOnlyHdfSession, Slice};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const MAX: usize = 64 << 20;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn le_bytes_and_bits(v: &NumericValues) -> (Vec<u8>, Vec<u64>) {
    match v {
        NumericValues::F32(v) => (
            v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            v.iter().take(8).map(|x| u64::from(x.to_bits())).collect(),
        ),
        NumericValues::F64(v) => (
            v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            v.iter().take(8).map(|x| x.to_bits()).collect(),
        ),
        other => panic!("unexpected dtype {other:?}"),
    }
}
/// h5py normalizes string padding on read (the real files are space padded), so the stored
/// payload and the oracle bytes are compared per element after trimming trailing NUL/space.
fn same_padded(
    raw: &[u8],
    oracle_hex: &serde_json::Value,
    itemsize: &serde_json::Value,
    ctx: &str,
) {
    let h = oracle_hex.as_str().unwrap();
    let want: Vec<u8> = (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
        .collect();
    let width = usize::try_from(itemsize.as_u64().unwrap()).unwrap();
    assert_eq!(raw.len(), want.len(), "{ctx} payload length");
    let trim = |b: &[u8]| {
        b.iter()
            .rposition(|c| *c != 0 && *c != b' ')
            .map_or(0, |i| i + 1)
    };
    for (a, b) in raw.chunks(width).zip(want.chunks(width)) {
        assert_eq!(&a[..trim(a)], &b[..trim(b)], "{ctx} element bytes");
    }
}
fn kind_of(k: &str) -> K {
    match k {
        "SA 2D Area Conn" => K::Sa2dAreaConnection,
        "Lateral Structures" => K::LateralStructure,
        "Storage Areas" => K::StorageArea,
        other => panic!("family {other}"),
    }
}
fn numeric(dtype: &str) -> NumericKind {
    match dtype {
        "float32" => NumericKind::F32,
        "float64" => NumericKind::F64,
        other => panic!("dtype {other}"),
    }
}
fn slice(v: &serde_json::Value) -> [Slice; 2] {
    let g = |k: &str| {
        let a = v[k].as_array().unwrap();
        Slice::new(
            a[0].as_u64().unwrap(),
            a[1].as_u64().unwrap(),
            a[2].as_u64().unwrap(),
        )
    };
    [g("rows"), g("cols")]
}
fn check_selection(
    s: &ReadOnlyHdfSession,
    kind: K,
    structure: Option<&str>,
    rec: &serde_json::Value,
    ctx: &str,
) {
    let Some(sel) = rec.get("selection") else {
        return;
    };
    let [rows, cols] = slice(sel);
    let rel = rec["relative"].as_str().unwrap();
    let got = s
        .read_structure_series(kind, structure, rel, rows, cols, MAX)
        .unwrap_or_else(|e| panic!("{ctx} {rel}: {e}"));
    let want_shape: Vec<u64> = sel["shape"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap())
        .collect();
    assert_eq!(got.shape, want_shape, "{ctx} {rel} shape");
    let (bytes, bits) = le_bytes_and_bits(&got.values);
    assert_eq!(
        hex(&Sha256::digest(&bytes)),
        sel["sha256_le"].as_str().unwrap(),
        "{ctx} {rel} sha"
    );
    let want_bits: Vec<u64> = sel["first_bits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap())
        .collect();
    assert_eq!(bits, want_bits, "{ctx} {rel} first IEEE bits");
}

fn check_series(
    s: &ReadOnlyHdfSession,
    kind: K,
    structure: Option<&str>,
    got: &ras_commander_rust::hdf::structures::StructureSeries,
    rec: &serde_json::Value,
    ctx: &str,
) {
    let rel = rec["relative"].as_str().unwrap();
    let ctx = format!("{ctx} {rel}");
    let shape: Vec<u64> = rec["shape"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap())
        .collect();
    assert_eq!(got.shape, shape, "{ctx} shape");
    if rec["dtype"].as_str().unwrap().starts_with("float") {
        assert_eq!(
            got.kind,
            Some(numeric(rec["dtype"].as_str().unwrap())),
            "{ctx}"
        );
        assert!(got.capability.is_supported(), "{ctx}");
    }
    let labels = &rec["labels"];
    match (
        got.columns.value(),
        got.columns.is_unavailable(),
        labels["state"].as_str().unwrap(),
    ) {
        (None, true, "absent") => {}
        (Some(set), _, "declared") => {
            let (source, got_labels) = (&set.source, &set.labels);
            let raw_bytes = got
                .columns
                .raw_bytes()
                .expect("declared labels keep raw bytes");
            let want_source = if labels["source"] == "VariableUnit" {
                ColumnLabelSource::VariableUnit
            } else {
                ColumnLabelSource::Connections
            };
            assert_eq!(*source, want_source, "{ctx}");
            same_padded(raw_bytes, &labels["hex"], &labels["itemsize"], &ctx);
            let names: Vec<&str> = labels["names"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap())
                .collect();
            assert_eq!(
                got_labels
                    .iter()
                    .map(|l| l.name.as_str())
                    .collect::<Vec<_>>(),
                names,
                "{ctx} names"
            );
            if let Some(units) = labels["units"].as_array() {
                let units: Vec<&str> = units.iter().map(|x| x.as_str().unwrap()).collect();
                assert_eq!(
                    got_labels
                        .iter()
                        .map(|l| l.unit.as_deref().unwrap())
                        .collect::<Vec<_>>(),
                    units,
                    "{ctx} units"
                );
            }
        }
        (_, _, want) => panic!("{ctx}: labels {:?} vs oracle {want}", got.columns),
    }
    if structure.is_some() {
        let units = &got.units.units;
        match (
            units.value(),
            units.is_unavailable(),
            rec["units_attr"].as_object(),
        ) {
            (None, true, None) => {}
            (Some(text), _, Some(u)) => {
                assert_eq!(Some(text.as_str()), u["text"].as_str(), "{ctx}");
                let raw_bytes = units.raw_bytes().expect("declared units keep raw bytes");
                same_padded(raw_bytes, &u["hex"], &u["itemsize"], &ctx);
            }
            (_, _, want) => panic!("{ctx}: units {units:?} vs oracle {want:?}"),
        }
    }
    check_selection(s, kind, structure, rec, &ctx);
}

fn check_identity(
    got: &ras_commander_rust::hdf::structures::GeometryIdentity,
    kind: K,
    want: &serde_json::Value,
    ctx: &str,
) {
    match kind {
        K::StorageArea => {
            assert_eq!(got.id.row, want["row"].as_u64().unwrap(), "{ctx}");
            assert_eq!(got.name, want["name"].as_str().unwrap(), "{ctx}");
        }
        K::LateralStructure => {
            assert_eq!(got.id.row, want["row"].as_u64().unwrap(), "{ctx}");
            assert_eq!(got.river, want["river"].as_str().unwrap(), "{ctx}");
            assert_eq!(got.reach, want["reach"].as_str().unwrap(), "{ctx}");
            assert_eq!(got.station, want["station"].as_str().unwrap(), "{ctx}");
        }
        _ => {
            let rows = want["rows"].as_array().unwrap();
            assert_eq!(rows.len(), 1, "{ctx} oracle must find exactly one row");
            assert_eq!(got.id.row, rows[0].as_u64().unwrap(), "{ctx}");
            assert_eq!(
                got.connection,
                want["connection"].as_str().unwrap(),
                "{ctx}"
            );
            assert_eq!(got.upstream, want["upstream"].as_str().unwrap(), "{ctx}");
            assert_eq!(
                got.downstream,
                want["downstream"].as_str().unwrap(),
                "{ctx}"
            );
        }
    }
}
fn strs(v: &serde_json::Value) -> Vec<&str> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect()
}

fn check_structure(
    s: &ReadOnlyHdfSession,
    kind: K,
    got: &ras_commander_rust::hdf::structures::StructureDescriptor,
    want: &serde_json::Value,
    geometry_id: &serde_json::Value,
    ctx: &str,
) {
    let sctx = format!("{ctx} / {}", got.name);
    match (
        got.node_pointer.value(),
        got.node_pointer.is_unavailable(),
        want["node_pointer"].as_i64(),
    ) {
        (Some(a), _, Some(b)) => assert_eq!(*a, b, "{sctx}"),
        (None, true, None) => {}
        (_, _, w) => panic!("{sctx}: node pointer {:?} vs {w:?}", got.node_pointer),
    }
    check_identity(&got.identity, kind, geometry_id, &sctx);
    let mut have: Vec<&str> = got.series.iter().map(|x| x.relative.as_str()).collect();
    let mut exp: Vec<&str> = want["series"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["relative"].as_str().unwrap())
        .collect();
    have.sort_unstable();
    exp.sort_unstable();
    assert_eq!(have, exp, "{sctx} series set");
    let mut statics = got.static_datasets.clone();
    statics.sort();
    assert_eq!(
        statics.iter().map(String::as_str).collect::<Vec<_>>(),
        strs(&want["static_datasets"]),
        "{sctx} static datasets"
    );
    for w in want["series"].as_array().unwrap() {
        let g = got
            .series
            .iter()
            .find(|x| x.relative == w["relative"].as_str().unwrap())
            .unwrap();
        check_series(s, kind, Some(&got.name), g, w, &sctx);
    }
}

/// RAS_NATIVE_FIXTURES = fixture directory; RAS_STRUCT_ORACLE = JSON from
/// tests/fixtures/structures/oracle_real_structures.py (run on the same fixtures).
#[test]
#[ignore = "requires real fixtures (RAS_NATIVE_FIXTURES) and h5py oracle JSON (RAS_STRUCT_ORACLE)"]
fn real_structure_families_match_h5py_oracle_bit_exactly() {
    use ras_commander_rust::{Capability, ErrorKind};
    let root = PathBuf::from(std::env::var_os("RAS_NATIVE_FIXTURES").expect("RAS_NATIVE_FIXTURES"));
    let oracle: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("RAS_STRUCT_ORACLE").expect("RAS_STRUCT_ORACLE")).unwrap(),
    )
    .unwrap();
    let mut compared = 0;
    for (file, entry) in oracle["files"].as_object().unwrap() {
        let s = ReadOnlyHdfSession::open(root.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        let caps = s.plan_capabilities(DEFAULT_MAX_BYTES).unwrap();
        for (fam, rec) in entry["families"].as_object().unwrap() {
            let kind = kind_of(fam);
            let ctx = format!("{file} {fam}");
            let cap = caps.structures.iter().find(|c| c.kind == kind).unwrap();
            if !rec["present"].as_bool().unwrap() {
                assert!(
                    matches!(cap.status, Capability::Unavailable { .. }),
                    "{ctx}"
                );
                let e = s.structure_inventory(kind, MAX).unwrap_err();
                assert_eq!(e.kind, ErrorKind::NotFound, "{ctx}");
                continue;
            }
            assert!(cap.status.is_supported(), "{ctx}: {:?}", cap.status);
            let inv = s
                .structure_inventory(kind, MAX)
                .unwrap_or_else(|e| panic!("{ctx}: {e}"));
            assert_eq!(inv.time_rows, rec["time_rows"].as_u64().unwrap(), "{ctx}");
            let names: Vec<&str> = inv.structures.iter().map(|x| x.name.as_str()).collect();
            assert_eq!(names, strs(&rec["group_names"]), "{ctx} structure names");
            for (got, want) in inv
                .structures
                .iter()
                .zip(rec["structures"].as_array().unwrap())
            {
                let id = &rec["geometry"]["ids"][&got.name];
                check_structure(&s, kind, got, want, id, &ctx);
                compared += 1;
            }
            let fam_names: Vec<&str> = inv
                .family_series
                .iter()
                .map(|x| x.relative.as_str())
                .collect();
            assert_eq!(
                fam_names,
                strs(&rec["family_dataset_names"]),
                "{ctx} family datasets"
            );
            for (g, w) in inv
                .family_series
                .iter()
                .zip(rec["family_series"].as_array().unwrap())
            {
                check_series(&s, kind, None, g, w, &ctx);
            }
        }
        let agg = caps
            .families
            .iter()
            .find(|f| f.family == ras_commander_rust::hdf::xs1d::ResultFamily::Structures)
            .unwrap();
        assert!(agg.status.is_supported(), "{file}: {:?}", agg.status);
    }
    assert!(compared > 0);
}
