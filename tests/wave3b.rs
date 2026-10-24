#![cfg(feature = "hdf")]
use ras_commander_fast_hdf::{
    hdf::{
        families::{ReadFamily, READ_FAMILIES},
        stored::StoredValues,
        NumericValues, ReadOnlyHdfSession, Slice,
    },
    Capability, UnsupportedCode,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::PathBuf};
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
#[ignore = "requires relocatable RAS_WAVE3B_ORACLE and RAS_WAVE3B_FIXTURE_DIR"]
fn all_environmental_reads_match_h5py() {
    use ras_commander_fast_hdf::hdf::environment::ClassId;
    let m = std::env::var("RAS_WAVE3B_ORACLE").expect("RAS_WAVE3B_ORACLE required");
    let root = PathBuf::from(
        std::env::var("RAS_WAVE3B_FIXTURE_DIR").expect("RAS_WAVE3B_FIXTURE_DIR required"),
    );
    let j: Value = serde_json::from_slice(&std::fs::read(m).unwrap()).unwrap();
    let mut checked = 0;
    assert_eq!(
        j["cases"].as_array().unwrap().len(),
        36,
        "complete qualification corpus required"
    );
    for c in j["cases"].as_array().unwrap() {
        let path = root.join(c["name"].as_str().unwrap());
        assert_eq!(
            sha(std::fs::read(&path).unwrap()),
            c["sha256"].as_str().unwrap()
        );
        let session = ReadOnlyHdfSession::open(&path).unwrap();
        for (name, e) in c["families"].as_object().unwrap() {
            let f = family(name);
            let actual = session
                .read_family_inventory(f, 128 << 20)
                .unwrap_or_else(|e| panic!("{} {name}: {e:?}", path.display()));
            assert_eq!(
                actual.capability.as_str(),
                e["status"].as_str().unwrap(),
                "{} {name}",
                path.display()
            );
            assert_eq!(
                actual
                    .datasets
                    .iter()
                    .map(|d| d.stored.path.as_str())
                    .collect::<BTreeSet<_>>(),
                e["datasets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|d| d["path"].as_str().unwrap())
                    .collect::<BTreeSet<_>>(),
                "{} {name}",
                path.display()
            );
            for d in e["datasets"].as_array().unwrap() {
                let p = d["path"].as_str().unwrap();
                let a = actual.datasets.iter().find(|a| a.stored.path == p).unwrap();
                assert_eq!(
                    a.stored.capability.as_str(),
                    d["expected_status"].as_str().unwrap(),
                    "{} {p}: {:?}",
                    path.display(),
                    a.stored.capability
                );
                assert_eq!(serde_json::json!(a.stored.shape), d["shape"]);
                assert_eq!(a.stored.storage, d["storage"].as_str().unwrap(), "{p}");
                if a.stored.storage == "numeric" {
                    let k = d["kind"].as_str().unwrap();
                    let bits = k[1..].parse::<u32>().unwrap() * 8;
                    assert_eq!(
                        a.stored.kind.unwrap().as_str(),
                        format!("{}{bits}", &k[..1]),
                        "{p}"
                    );
                } else {
                    assert!(a.stored.kind.is_none());
                }

                if name == "encroachments" && p.ends_with("/Profile Names") {
                    assert_eq!(a.domain, "profile");
                    assert_eq!(a.role, "name_axis");
                }
                check_attrs(&a.stored.attributes, &d["attribute_values"]);
                if !a.stored.capability.is_supported() {
                    assert_eq!(
                        a.stored.capability.code().unwrap().as_str(),
                        d["expected_code"].as_str().unwrap()
                    );
                    // A strict domain refusal must not erase a literal stored read.
                    let tail = p
                        .strip_prefix("Results/Unsteady/Output/Output Blocks/")
                        .expect("qualified generic block fallback");
                    let (block, relative) = tail.split_once('/').unwrap();
                    let selection = selections(d);
                    check_read(
                        session
                            .read_output_block(block, relative, selection.as_deref(), 128 << 20)
                            .unwrap(),
                        d,
                    );
                    checked += 1;
                    continue;
                }
                let selection = selections(d);
                check_read(
                    session
                        .read_family_dataset(f, p, selection.as_deref(), 128 << 20, 128 << 20)
                        .unwrap_or_else(|e| panic!("{} {p}: {e:?}", path.display())),
                    d,
                );
                checked += 1;
            }
            for (p, attrs) in e["groups"].as_object().unwrap() {
                let a = actual
                    .groups
                    .iter()
                    .find(|a| &a.path == p)
                    .unwrap_or_else(|| panic!("{} missing group {p}", path.display()));
                check_attrs(&a.attributes, attrs);
            }
            if let Some(e) = e.get("classification") {
                let catalogue = session.classification_catalogue(f, 128 << 20).unwrap();
                let a = catalogue.value().unwrap();
                assert_eq!(a.entries.len(), e["links"].as_array().unwrap().len());
                for (a, e) in a.entries.iter().zip(e["links"].as_array().unwrap()) {
                    assert_eq!(a.name, e["name"].as_str().unwrap());
                    assert_eq!(a.capability.as_str(), e["status"].as_str().unwrap());
                    let id = match a.pixel_id {
                        Some(ClassId::Signed(v)) => serde_json::json!(v),
                        Some(ClassId::Unsigned(v)) => serde_json::json!(v),
                        None => Value::Null,
                        _ => panic!("unhandled class ID"),
                    };
                    assert_eq!(id, e["pixel_id"]);
                    assert_eq!(
                        serde_json::json!(a
                            .variable_rows
                            .iter()
                            .map(|r| r.row)
                            .collect::<Vec<_>>()),
                        e["rows"]
                    );
                }
                assert_eq!(
                    serde_json::json!(a
                        .unmatched_variables
                        .iter()
                        .map(|r| r.row)
                        .collect::<Vec<_>>()),
                    e["unmatched"]
                );
            }
        }
        println!("PASS {}", path.display());
    }
    assert_eq!(checked, 705, "complete payload evidence required");
    println!("W3B exact payload checks {checked}");
}

#[test]
fn real_class_table_excerpt_retains_ambiguity_and_unmatched_rows() {
    use ras_commander_fast_hdf::ErrorKind;
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wave3b/ambiguous_classes.h5");
    let session = ReadOnlyHdfSession::open(path).unwrap();
    let report = session
        .classification_catalogue(ReadFamily::LandCover, 1 << 20)
        .unwrap();
    let map = report.value().unwrap();
    assert_eq!(map.entries.len(), 2);
    assert_eq!(map.entries[0].name, "duplicate");
    assert!(matches!(
        map.entries[0].capability,
        Capability::Unsupported {
            code: UnsupportedCode::IdentityLayout,
            ..
        }
    ));
    assert_eq!(
        map.entries[0]
            .variable_rows
            .iter()
            .map(|id| id.row)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert!(matches!(
        map.entries[1].capability,
        Capability::Unavailable { .. }
    ));
    assert_eq!(
        map.unmatched_variables
            .iter()
            .map(|id| id.row)
            .collect::<Vec<_>>(),
        [2]
    );
    let err = session
        .classification_catalogue(ReadFamily::LandCover, 0)
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(err.code, Some(UnsupportedCode::ResourceLimit));
    assert_eq!(
        session
            .classification_catalogue(ReadFamily::Terrain, 1 << 20)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        session
            .read_family_dataset(
                ReadFamily::LandCover,
                "../Variables",
                None,
                1 << 20,
                1 << 20
            )
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
}

#[test]
fn scalar_sediment_time_is_corrupt_without_panicking() {
    use ras_commander_fast_hdf::ErrorKind;
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wave3b/scalar_sediment_time.h5");
    let s = ReadOnlyHdfSession::open(path).unwrap();
    let inv = s
        .read_family_inventory(ReadFamily::Sediment, 1 << 20)
        .unwrap();
    let result = inv
        .datasets
        .iter()
        .find(|d| d.stored.shape == [2, 1])
        .unwrap();
    assert!(matches!(
        result.stored.capability,
        Capability::Corrupt { .. }
    ));
    assert_eq!(
        s.read_family_dataset(
            ReadFamily::Sediment,
            &result.stored.path,
            None,
            1 << 20,
            1 << 20
        )
        .unwrap_err()
        .kind,
        ErrorKind::Corrupt
    );
}
