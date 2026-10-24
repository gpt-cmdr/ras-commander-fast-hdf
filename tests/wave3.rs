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
#[ignore = "requires relocatable RAS_WAVE3_ORACLE and RAS_WAVE3_FIXTURE_DIR"]
fn all_wave3_families_match_frozen_h5py() {
    let manifest =
        std::env::var("RAS_WAVE3_ORACLE").expect("RAS_WAVE3_ORACLE must name the frozen manifest");
    let root = PathBuf::from(
        std::env::var("RAS_WAVE3_FIXTURE_DIR").expect("RAS_WAVE3_FIXTURE_DIR required"),
    );
    let oracle: Value = serde_json::from_slice(&std::fs::read(manifest).unwrap()).unwrap();
    let mut checked = 0;
    assert_eq!(
        oracle["cases"].as_array().unwrap().len(),
        9,
        "complete qualification corpus required"
    );
    for c in oracle["cases"].as_array().unwrap() {
        let path = root.join(c["name"].as_str().unwrap());
        assert_eq!(
            sha(std::fs::read(&path).unwrap()),
            c["sha256"].as_str().unwrap()
        );
        let s = ReadOnlyHdfSession::open(&path).unwrap();
        for (name, expected) in c["families"].as_object().unwrap() {
            let f = family(name);
            let inventory = s.read_family_inventory(f, 64 << 20).unwrap();
            assert_eq!(
                inventory.capability.as_str(),
                expected["status"].as_str().unwrap(),
                "{} {name}",
                path.display()
            );
            let actual: BTreeSet<_> = inventory
                .datasets
                .iter()
                .map(|d| d.stored.path.as_str())
                .collect();
            let want: BTreeSet<_> = expected["datasets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| d["path"].as_str().unwrap())
                .collect();
            assert_eq!(actual, want, "{} {name}", path.display());
            if expected["coverage"] == "trivially_absent" {
                assert!(inventory.datasets.is_empty());
                println!("TRIVIALLY ABSENT {} {name}", path.display());
            }
            for d in expected["datasets"].as_array().unwrap() {
                let p = d["path"].as_str().unwrap();
                let a = inventory
                    .datasets
                    .iter()
                    .find(|a| a.stored.path == p)
                    .unwrap();
                assert_eq!(serde_json::json!(a.stored.shape), d["shape"]);
                assert_eq!(
                    a.stored.capability.as_str(),
                    d["expected_status"].as_str().unwrap(),
                    "{} {p}: {:?}",
                    path.display(),
                    a.stored.capability
                );
                assert_eq!(
                    serde_json::json!(a
                        .stored
                        .attributes
                        .iter()
                        .map(|a| &a.name)
                        .collect::<BTreeSet<_>>()),
                    d["attrs"]
                );
                check_attrs(&a.stored.attributes, &d["attribute_values"]);
                if d["expected_status"] == "unsupported" {
                    assert_eq!(
                        a.stored.capability.code().unwrap().as_str(),
                        d["expected_code"].as_str().unwrap()
                    );
                    if d["generic_numeric_fallback"] == true {
                        let err = s
                            .read_family_dataset(f, p, None, 64 << 20, 64 << 20)
                            .unwrap_err();
                        assert_eq!(err.code, Some(UnsupportedCode::IdentityLayout));
                        let selection = selections(d);
                        check_read(
                            StoredValues::Numeric(s.read_numeric(p, selection.as_deref()).unwrap()),
                            d,
                        );
                        checked += 1;
                    }
                    continue;
                }
                if let Some(ids) = d.get("id_values") {
                    let actual = a
                        .stored_location_ids
                        .as_ref()
                        .expect("stored ID values required separately from row positions");
                    assert!(
                        matches!(&actual.values, NumericValues::I32(_)),
                        "fixture stores i32 IDs"
                    );
                    check_read(StoredValues::Numeric(actual.clone()), ids);
                }
                if let Some(role) = d["role"].as_str() {
                    assert_eq!(a.role, role, "{p}");
                }
                if d.get("time_row").is_some() {
                    assert_eq!(serde_json::json!(a.time_of_extreme_row), d["time_row"]);
                }
                if d.get("names").is_some() {
                    assert_eq!(serde_json::json!(a.names), d["names"]);
                    assert_eq!(a.locations.len(), a.names.len());
                }
                if d.get("domain").is_some() {
                    assert_eq!(a.domain, d["domain"].as_str().unwrap(), "{p}");
                    assert_eq!(
                        serde_json::json!(a.identity_table),
                        d["identity_table"],
                        "{p}"
                    );
                    assert_eq!(
                        a.locations.len() as u64,
                        d["location_count"].as_u64().unwrap(),
                        "{p}"
                    );
                    for (row, id) in a.locations.iter().enumerate() {
                        assert_eq!(id.row, row as u64);
                        assert_eq!(Some(&id.table), a.identity_table.as_ref());
                        assert_eq!(id.source, s.path());
                    }
                }
                if let Some(rows) = d["time_rows"].as_u64() {
                    assert_eq!(serde_json::json!(a.time_group), d["time_group"], "{p}");
                    if let Some(group) = a.time_group.as_ref() {
                        assert_eq!(
                            s.read_numeric(&format!("{group}/Time"), None)
                                .unwrap()
                                .shape,
                            [rows]
                        );
                    }
                }
                let selection = selections(d);
                check_read(
                    s.read_family_dataset(f, p, selection.as_deref(), 64 << 20, 64 << 20)
                        .unwrap(),
                    d,
                );
                if let Some(samples) = d["additional_samples"].as_array() {
                    for sample in samples {
                        let mut record = d.clone();
                        for key in ["selection", "selected_shape", "sha256"] {
                            record[key] = sample[key].clone();
                        }
                        let selection = selections(&record);
                        check_read(
                            s.read_family_dataset(f, p, selection.as_deref(), 64 << 20, 64 << 20)
                                .unwrap(),
                            &record,
                        );
                    }
                }
                checked += 1;
            }
            if f == ReadFamily::PlanMetadata {
                for (path, attrs) in expected["groups"].as_object().unwrap() {
                    let g = inventory.groups.iter().find(|g| &g.path == path).unwrap();
                    check_attrs(&g.attributes, attrs);
                }
                assert!(inventory.groups.iter().any(|g| g.path == "/"));
            }
        }
        let blocks = s.output_block_inventory(64 << 20).unwrap();
        assert_eq!(blocks.len(), c["blocks"].as_array().unwrap().len());
        for b in c["blocks"].as_array().unwrap() {
            let name = b["name"].as_str().unwrap();
            let a = blocks.iter().find(|a| a.name == name).unwrap();
            assert_eq!(
                a.datasets.len(),
                b["datasets"].as_array().unwrap().len(),
                "{} {}",
                path.display(),
                name
            );
            for d in b["datasets"].as_array().unwrap() {
                let p = d["path"].as_str().unwrap();
                let v = a.datasets.iter().find(|a| a.path == p).unwrap();
                assert_eq!(
                    v.capability.as_str(),
                    d["expected_status"].as_str().unwrap(),
                    "{p}"
                );
                if !v.capability.is_supported() {
                    assert_eq!(
                        v.capability.code().unwrap().as_str(),
                        d["expected_code"].as_str().unwrap()
                    );
                    continue;
                }
                let relative = p.strip_prefix(&(a.path.clone() + "/")).unwrap();
                let selection = selections(d);
                check_read(
                    s.read_output_block(name, relative, selection.as_deref(), 64 << 20)
                        .unwrap_or_else(|e| panic!("{} {p}: {e:?}", path.display())),
                    d,
                );
                if let Some(samples) = d["additional_samples"].as_array() {
                    for sample in samples {
                        let mut record = d.clone();
                        for key in ["selection", "selected_shape", "sha256"] {
                            record[key] = sample[key].clone();
                        }
                        let selection = selections(&record);
                        check_read(
                            s.read_output_block(name, relative, selection.as_deref(), 64 << 20)
                                .unwrap(),
                            &record,
                        );
                    }
                }
                checked += 1;
            }
        }
        {
            let f = ReadFamily::PipeRunSummary;
            assert!(matches!(
                s.read_family_inventory(f, 64 << 20).unwrap().capability,
                Capability::Unsupported {
                    code: UnsupportedCode::Schema,
                    ..
                }
            ));
        }
        println!("PASS {} family datasets + output blocks", path.display());
    }
    assert_eq!(checked, 7970, "complete payload evidence required");
    println!("exact owned payload checks: {checked}");
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wave3")
        .join(name)
}
#[test]
fn real_excerpt_bounds_budget_and_corruption() {
    let s = ReadOnlyHdfSession::open(fixture("reference_excerpt.h5")).unwrap();
    let path="Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/Reference Lines/Flow";
    let inv = s
        .read_family_inventory(ReadFamily::ReferenceLines, 1 << 20)
        .unwrap();
    assert!(inv
        .datasets
        .iter()
        .all(|d| d.stored.capability.is_supported()));
    let err = s
        .read_family_dataset(ReadFamily::ReferenceLines, path, None, 1 << 20, 0)
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(err.code, Some(UnsupportedCode::ResourceLimit));
    let empty = [Slice::new(0, 0, 1), Slice::new(0, 1, 1)];
    let StoredValues::Numeric(a) = s
        .read_family_dataset(ReadFamily::ReferenceLines, path, Some(&empty), 1 << 20, 0)
        .unwrap()
    else {
        panic!("numeric required")
    };
    assert_eq!(a.shape, [0, 1]);
    assert_eq!(
        s.read_family_dataset(
            ReadFamily::ReferenceLines,
            path,
            Some(&[Slice::new(0, 1000, 1), Slice::new(0, 1, 1)]),
            1 << 20,
            1 << 20
        )
        .unwrap_err()
        .kind,
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        s.read_family_dataset(
            ReadFamily::ReferenceLines,
            "../Flow",
            None,
            1 << 20,
            1 << 20
        )
        .unwrap_err()
        .kind,
        ErrorKind::InvalidRequest
    );
    let s = ReadOnlyHdfSession::open(fixture("reference_bad_time.h5")).unwrap();
    assert!(matches!(
        s.read_family_inventory(ReadFamily::ReferenceLines, 1 << 20)
            .unwrap()
            .datasets
            .iter()
            .find(|d| d.stored.path == path)
            .unwrap()
            .stored
            .capability,
        Capability::Corrupt { .. }
    ));
    assert_eq!(
        s.read_family_dataset(ReadFamily::ReferenceLines, path, None, 1 << 20, 1 << 20)
            .unwrap_err()
            .kind,
        ErrorKind::Corrupt
    );
}

#[test]
fn pipe_ranges_are_checked_before_geometry_reads() {
    let s = ReadOnlyHdfSession::open(fixture("pipe_bad_ranges.h5")).unwrap();
    let path = "Geometry/Pipe Conduits/Polyline Points";
    let inv = s
        .read_family_inventory(ReadFamily::PipeGeometry, 1 << 20)
        .unwrap();
    assert!(matches!(
        inv.datasets
            .iter()
            .find(|d| d.stored.path == path)
            .unwrap()
            .stored
            .capability,
        Capability::Corrupt { .. }
    ));
    assert_eq!(
        s.read_family_dataset(ReadFamily::PipeGeometry, path, None, 1 << 20, 1 << 20)
            .unwrap_err()
            .kind,
        ErrorKind::Corrupt
    );
    assert_eq!(
        s.read_output_block("..", "x", None, 1024).unwrap_err().kind,
        ErrorKind::InvalidRequest
    );
}
