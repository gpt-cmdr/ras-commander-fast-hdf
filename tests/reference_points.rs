#![cfg(feature = "hdf")]
use ras_commander_fast_hdf::{
    hdf::{
        families::{ReadFamily, READ_FAMILIES},
        stored::StoredValues,
        NumericValues, ReadOnlyHdfSession, Slice,
    },
    Capability, ErrorKind, UnsupportedCode,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
fn family(name: &str) -> ReadFamily {
    *READ_FAMILIES.iter().find(|f| f.as_str() == name).unwrap()
}
fn sha(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}
fn numeric_bytes(v: NumericValues) -> Vec<u8> {
    macro_rules! bytes {
        ($v:expr) => {
            $v.into_iter().flat_map(|v| v.to_le_bytes()).collect()
        };
    }
    match v {
        NumericValues::I8(v) => bytes!(v),
        NumericValues::U8(v) => v,
        NumericValues::I16(v) => bytes!(v),
        NumericValues::U16(v) => bytes!(v),
        NumericValues::I32(v) => bytes!(v),
        NumericValues::U32(v) => bytes!(v),
        NumericValues::I64(v) => bytes!(v),
        NumericValues::U64(v) => bytes!(v),
        NumericValues::F32(v) => bytes!(v),
        NumericValues::F64(v) => bytes!(v),
        _ => panic!("unhandled numeric kind"),
    }
}
fn selections(d: &Value) -> Option<Vec<Slice>> {
    d["selection"].as_array().map(|v| {
        v.iter()
            .map(|s| {
                Slice::new(
                    s[0].as_u64().unwrap(),
                    s[1].as_u64().unwrap(),
                    s[2].as_u64().unwrap(),
                )
            })
            .collect()
    })
}
fn check_attrs(actual: &[ras_commander_fast_hdf::hdf::metadata::TypedAttribute], expected: &Value) {
    use ras_commander_fast_hdf::hdf::metadata::MetadataValue;
    for e in expected.as_array().unwrap() {
        let a = actual
            .iter()
            .find(|a| a.name == e["name"].as_str().unwrap())
            .unwrap();
        let v = match &a.value {
            MetadataValue::Signed(v) => serde_json::json!(v),
            MetadataValue::Unsigned(v) => serde_json::json!(v),
            MetadataValue::Float(v) => serde_json::json!(v
                .iter()
                .map(|v| format!("{:016x}", v.to_bits()))
                .collect::<Vec<_>>()),
            MetadataValue::Strings(v) => serde_json::json!(v),
            _ => panic!("expected qualified attribute {}: {:?}", a.name, a.value),
        };
        assert_eq!(v, e["values"], "attribute {}", a.name);
    }
}
fn check_read(actual: StoredValues, d: &Value) {
    match actual {
        StoredValues::Numeric(a) => {
            assert_eq!(
                serde_json::json!(a.shape),
                d["selected_shape"],
                "{}",
                d["path"]
            );
            assert_eq!(
                sha(numeric_bytes(a.values)),
                d["sha256"].as_str().unwrap(),
                "{}",
                d["path"]
            );
        }
        StoredValues::Strings { shape, values } => {
            assert_eq!(serde_json::json!(shape), d["shape"]);
            assert_eq!(serde_json::json!(values), d["values"], "{}", d["path"]);
        }
        StoredValues::Compound(t) => {
            if let Some(fields) = d["field_bytes"].as_array() {
                let width = d["itemsize"].as_u64().unwrap() as usize;
                for e in fields {
                    let name = e["name"].as_str().unwrap();
                    let column = t.table.columns.iter().find(|c| c.name == name).unwrap();
                    assert_eq!(u64::from(column.byte_offset), e["offset"].as_u64().unwrap());
                    let start = e["offset"].as_u64().unwrap() as usize;
                    let size = e["width"].as_u64().unwrap() as usize;
                    let bytes = t
                        .raw_bytes
                        .chunks_exact(width)
                        .flat_map(|row| row[start..start + size].iter().copied())
                        .collect::<Vec<_>>();
                    assert_eq!(
                        sha(bytes),
                        e["sha256"].as_str().unwrap(),
                        "{} member {name}",
                        d["path"]
                    );
                }
            } else {
                assert_eq!(sha(&t.raw_bytes), d["sha256"].as_str().unwrap());
            }
            assert_eq!(
                serde_json::json!(t.table.columns.iter().map(|c| &c.name).collect::<Vec<_>>()),
                d["columns"]
            );
        }
        StoredValues::Bitfield { shape, raw_bytes } => {
            assert_eq!(serde_json::json!(shape), d["shape"]);
            assert_eq!(sha(raw_bytes), d["sha256"].as_str().unwrap());
        }
        _ => panic!("unhandled storage"),
    }
}
#[test]
#[ignore = "requires RAS_REFERENCE_POINTS_ORACLE and RAS_REFERENCE_POINTS_FIXTURES"]
fn reference_points_match_real_h5py_geometry_series_and_identities() {
    use ras_commander_fast_hdf::hdf::{lines::GeometryFamily, xs1d::ResultFamily};
    let file =
        std::env::var("RAS_REFERENCE_POINTS_ORACLE").expect("RAS_REFERENCE_POINTS_ORACLE required");
    let root = PathBuf::from(
        std::env::var("RAS_REFERENCE_POINTS_FIXTURES")
            .expect("RAS_REFERENCE_POINTS_FIXTURES required"),
    );
    let oracle: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    let mut checked = 0;
    let mut line_checked = 0;
    for c in oracle["cases"].as_array().unwrap() {
        let path = root.join(c["name"].as_str().unwrap());
        assert_eq!(
            sha(std::fs::read(&path).unwrap()),
            c["sha256"].as_str().unwrap()
        );
        let s = ReadOnlyHdfSession::open(&path).unwrap();
        let reported = s.reference_point_geometry(64 << 20).unwrap();
        let g = reported.value().unwrap();
        assert!(s
            .geometry_family_capability(GeometryFamily::ReferencePoints, 64 << 20)
            .unwrap()
            .is_supported());
        check_read(
            StoredValues::Compound(g.attributes.clone()),
            &c["geometry"]["attributes"],
        );
        check_read(
            StoredValues::Numeric(g.points.clone()),
            &c["geometry"]["points"],
        );
        checked += 2;
        for (i, (feature, want)) in g
            .features
            .iter()
            .zip(c["geometry"]["features"].as_array().unwrap())
            .enumerate()
        {
            assert_eq!(feature.name, want["name"].as_str().unwrap());
            assert_eq!(feature.area, want["area"].as_str().unwrap());
            assert_eq!(
                i64::from(feature.cell_index),
                want["cell"].as_i64().unwrap()
            );
            assert_eq!(feature.point_row, i as u64);
            assert_eq!(feature.id.row, i as u64);
            assert_eq!(feature.id.table, "Geometry/Reference Points/Attributes");
            assert_eq!(feature.id.source, path);
        }
        assert_eq!(
            g.features.len(),
            c["geometry"]["features"].as_array().unwrap().len()
        );
        let layers = s.layer_inventory(64 << 20).unwrap();
        let point = layers
            .iter()
            .find(|l| l.family == "geometry" && l.name == "reference_points")
            .unwrap();
        assert!(point.capability.is_supported());
        assert_eq!(point.geometry_type, "point");
        assert_eq!(point.row_count, Some(3));
        let f = family("reference_point_results");
        let inv = s.read_family_inventory(f, 64 << 20).unwrap();
        let expected = c["series"].as_array().unwrap();
        assert_eq!(
            inv.datasets
                .iter()
                .map(|d| d.stored.path.as_str())
                .collect::<BTreeSet<_>>(),
            expected
                .iter()
                .map(|d| d["path"].as_str().unwrap())
                .collect()
        );
        if expected.is_empty() {
            assert!(matches!(inv.capability, Capability::Unavailable { .. }));
        } else {
            assert!(inv.capability.is_supported());
            assert!(s
                .plan_capabilities(64 << 20)
                .unwrap()
                .families
                .iter()
                .find(|f| f.family == ResultFamily::ReferencePoints)
                .unwrap()
                .status
                .is_supported());
        }
        if c["direction_present"] == true {
            let evidence = &c["direction_evidence"];
            assert_eq!(evidence["nonzero_samples"], 290);
            assert!(evidence["max_xy_degrees_error"].as_f64().unwrap() < 1e-4);
            assert!(evidence["max_north_degrees_error"].as_f64().unwrap() > 1.0);
        }
        assert_eq!(
            inv.datasets
                .iter()
                .filter(|d| d.stored.path.ends_with("/Direction"))
                .count(),
            if c["direction_present"] == true { 2 } else { 0 }
        );
        let line_family = ReadFamily::ReferenceLines;
        let lines = s.read_family_inventory(line_family, 64 << 20).unwrap();
        let expected_lines = c["lines"].as_array().unwrap();
        assert_eq!(
            lines
                .datasets
                .iter()
                .map(|d| d.stored.path.as_str())
                .collect::<BTreeSet<_>>(),
            expected_lines
                .iter()
                .map(|d| d["path"].as_str().unwrap())
                .collect()
        );
        for d in expected_lines {
            let path = d["path"].as_str().unwrap();
            let actual = lines
                .datasets
                .iter()
                .find(|v| v.stored.path == path)
                .unwrap();
            assert!(
                actual.stored.capability.is_supported(),
                "{path}: {:?}",
                actual.stored.capability
            );
            assert_eq!(actual.role, d["role"].as_str().unwrap());
            if !path.ends_with("/Name") {
                assert_eq!(serde_json::json!(actual.names), d["names"]);
                assert_eq!(serde_json::json!(actual.identity_table), d["name_table"]);
                assert_eq!(actual.domain, "reference_line");
            }
            check_attrs(&actual.stored.attributes, &d["attribute_values"]);
            let select = selections(d);
            check_read(
                s.read_family_dataset(line_family, path, select.as_deref(), 64 << 20, 64 << 20)
                    .unwrap(),
                d,
            );
            line_checked += 1;
        }
        for d in expected {
            let path = d["path"].as_str().unwrap();
            let a = inv.datasets.iter().find(|a| a.stored.path == path).unwrap();
            assert!(
                a.stored.capability.is_supported(),
                "{path}: {:?}",
                a.stored.capability
            );
            assert_eq!(a.domain, "reference_point");
            assert_eq!(a.role, d["role"].as_str().unwrap());
            assert_eq!(serde_json::json!(a.time_group), d["time_group"]);
            assert_eq!(serde_json::json!(a.identity_table), d["name_table"]);
            assert_eq!(serde_json::json!(a.names), d["names"]);
            check_attrs(&a.stored.attributes, &d["attribute_values"]);
            assert_eq!(
                a.geometry_locations.len(),
                d["geometry_rows"].as_array().unwrap().len()
            );
            for (i, (loc, geom)) in a.locations.iter().zip(&a.geometry_locations).enumerate() {
                assert_eq!(loc.table, d["name_table"].as_str().unwrap());
                assert_eq!(loc.row, i as u64);
                assert_eq!(loc.source, s.path());
                assert_eq!(geom.table, "Geometry/Reference Points/Attributes");
                assert_eq!(geom.row, d["geometry_rows"][i].as_u64().unwrap());
                assert_eq!(geom.source, s.path());
            }
            let select = selections(d);
            check_read(
                s.read_family_dataset(f, path, select.as_deref(), 64 << 20, 64 << 20)
                    .unwrap(),
                d,
            );
            checked += 1;
        }
        assert_eq!(
            s.read_family_inventory(ReadFamily::ReferenceSummaries, 64 << 20)
                .unwrap()
                .capability
                .code(),
            Some(UnsupportedCode::Schema)
        );
        println!("PASS reference points {}", path.display());
    }
    assert_eq!(oracle["cases"].as_array().unwrap().len(), 4);
    assert_eq!(checked, 30);
    assert_eq!(line_checked, 72);
    println!("reference-line full payload checks {line_checked}");
    println!("reference-point full payload checks {checked}");
}
fn local(name: &str) -> ReadOnlyHdfSession {
    ReadOnlyHdfSession::open(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/reference_points")
            .join(name),
    )
    .unwrap()
}
const ROOT: &str =
    "Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/Reference Points";
#[test]
fn point_name_permutation_is_linked_not_assumed_and_reads_are_bounded() {
    let s = local("reordered.h5");
    let inv = s
        .read_family_inventory(ReadFamily::ReferencePoints, 1 << 20)
        .unwrap();
    for d in &inv.datasets {
        assert!(
            d.stored.capability.is_supported(),
            "{:?}",
            d.stored.capability
        );
        assert_eq!(
            d.geometry_locations
                .iter()
                .map(|id| id.row)
                .collect::<Vec<_>>(),
            [2, 0, 1]
        );
    }
    let path = format!("{ROOT}/Direction");
    let err = s
        .read_family_dataset(ReadFamily::ReferencePoints, &path, None, 1 << 20, 0)
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(err.code, Some(UnsupportedCode::ResourceLimit));
    let empty = [Slice::new(0, 0, 1), Slice::new(0, 3, 1)];
    let StoredValues::Numeric(a) = s
        .read_family_dataset(ReadFamily::ReferencePoints, &path, Some(&empty), 1 << 20, 0)
        .unwrap()
    else {
        panic!("numeric")
    };
    assert_eq!(a.shape, [0, 3]);
    assert_eq!(
        s.reference_point_geometry(0).unwrap_err().code,
        Some(UnsupportedCode::ResourceLimit)
    );
    assert_eq!(s.read_family_dataset(ReadFamily::ReferencePoints,"Results/Unsteady/Output/Output Blocks/Base Output/Summary Output/Reference Points/Maximum Water Surface",None,1<<20,1<<20).unwrap_err().code,Some(UnsupportedCode::Schema));
}
#[test]
fn malformed_point_layouts_are_isolated_and_never_relabelled() {
    for name in [
        "bad_cells.h5",
        "bad_areas.h5",
        "bad_units.h5",
        "bad_name.h5",
        "bad_points.h5",
        "bad_time.h5",
        "duplicate_geometry.h5",
    ] {
        let s = local(name);
        let inv = s
            .read_family_inventory(ReadFamily::ReferencePoints, 1 << 20)
            .unwrap();
        let d = inv
            .datasets
            .iter()
            .find(|d| d.stored.path.ends_with("Direction"))
            .unwrap();
        assert!(!d.stored.capability.is_supported(), "{name}");
        let e = s
            .read_family_dataset(
                ReadFamily::ReferencePoints,
                &format!("{ROOT}/Direction"),
                None,
                1 << 20,
                1 << 20,
            )
            .unwrap_err();
        if matches!(
            name,
            "bad_units.h5" | "bad_name.h5" | "duplicate_geometry.h5"
        ) {
            assert_eq!(e.kind, ErrorKind::Unsupported, "{name}");
        } else {
            assert_eq!(e.kind, ErrorKind::Corrupt, "{name}");
        }
        if matches!(name, "bad_cells.h5" | "bad_areas.h5" | "bad_units.h5") {
            assert!(s
                .read_family_dataset(
                    ReadFamily::ReferencePoints,
                    &format!("{ROOT}/Water Surface"),
                    None,
                    1 << 20,
                    1 << 20
                )
                .is_ok());
        }
    }
    let s = local("unknown_variable.h5");
    assert_eq!(
        s.read_family_dataset(
            ReadFamily::ReferencePoints,
            &format!("{ROOT}/Flow"),
            None,
            1 << 20,
            1 << 20
        )
        .unwrap_err()
        .code,
        Some(UnsupportedCode::Schema)
    );
}
