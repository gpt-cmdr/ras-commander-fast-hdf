#![cfg(feature = "hdf")]
//! NULL (h5py `Empty`), scalar and zero-length simple dataspaces stay distinct in
//! every typed reader. Synthetic h5py fixture: `generate_dataspace_fixture.py`.
use ras_commander_fast_hdf::{
    hdf::{metadata::MetadataValue, Dataspace, ReadOnlyHdfSession},
    Capability, ErrorKind, UnsupportedCode,
};
fn session() -> ReadOnlyHdfSession {
    ReadOnlyHdfSession::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/dataspace/dataspace.h5"),
    )
    .unwrap()
}
/// (attribute, expected dataspace, legacy shape) for every attribute on /g and /d.
fn expected() -> Vec<(&'static str, Dataspace, Vec<u64>)> {
    vec![
        ("null_f4", Dataspace::Null, vec![0]),
        ("null_i4", Dataspace::Null, vec![0]),
        ("null_str", Dataspace::Null, vec![0]),
        ("null_compound", Dataspace::Null, vec![0]),
        ("null_bitfield", Dataspace::Null, vec![0]),
        ("scalar_f4", Dataspace::Scalar, vec![]),
        ("scalar_str", Dataspace::Scalar, vec![]),
        ("empty_f4", Dataspace::Simple(vec![0]), vec![0]),
        ("empty_str", Dataspace::Simple(vec![0]), vec![0]),
        ("empty_2d_i4", Dataspace::Simple(vec![0, 3]), vec![0, 3]),
        ("one_f4", Dataspace::Simple(vec![1]), vec![1]),
    ]
}
fn check_attributes(attrs: &[ras_commander_fast_hdf::hdf::metadata::TypedAttribute], owner: &str) {
    let cases = expected();
    assert_eq!(attrs.len(), cases.len(), "{owner}");
    for (name, dataspace, shape) in cases {
        let a = attrs.iter().find(|a| a.name == name).unwrap();
        assert_eq!(a.dataspace, dataspace, "{owner} {name}");
        assert_eq!(a.shape, shape, "{owner} {name}");
        assert_eq!(
            a.dataspace.element_count(),
            Some(match name {
                "scalar_f4" | "scalar_str" | "one_f4" => 1,
                _ => 0,
            })
        );
    }
    // NULL and an empty simple array share shape [0] and empty bytes; only dataspace differs.
    let null = attrs.iter().find(|a| a.name == "null_f4").unwrap();
    let empty = attrs.iter().find(|a| a.name == "empty_f4").unwrap();
    assert_eq!(
        (&null.shape, &null.raw_bytes),
        (&empty.shape, &empty.raw_bytes)
    );
    assert_ne!(null.dataspace, empty.dataspace);
    assert_eq!(
        (null.dataspace.as_str(), empty.dataspace.as_str()),
        ("null", "simple")
    );
    let scalar = attrs.iter().find(|a| a.name == "scalar_f4").unwrap();
    assert_eq!(
        (scalar.dataspace.as_str(), &scalar.value),
        ("scalar", &MetadataValue::Float(vec![1.5]))
    );
}
#[test]
fn group_and_dataset_attributes_report_their_dataspace_class() {
    let s = session();
    check_attributes(&s.typed_group_attributes("g").unwrap(), "group");
    check_attributes(
        &s.typed_dataset_attributes("d", 1 << 20)
            .unwrap()
            .into_value()
            .unwrap(),
        "dataset",
    );
}
#[test]
fn numeric_and_decoded_string_readers_refuse_null_instead_of_returning_a_scalar_or_empty() {
    let s = session();
    let null = s.numeric_capability("datasets/null_f4").unwrap();
    assert_eq!(
        (null.shape.clone(), null.dataspace.clone()),
        (vec![], Dataspace::Null)
    );
    assert_eq!(null.capability.code(), Some(UnsupportedCode::Schema));
    let e = s.read_numeric("datasets/null_f4", None).unwrap_err();
    assert_eq!(
        (e.kind, e.code),
        (ErrorKind::Unsupported, Some(UnsupportedCode::Schema))
    );
    let scalar = s.numeric_capability("datasets/scalar_f4").unwrap();
    assert_eq!(
        (scalar.dataspace, scalar.capability),
        (Dataspace::Scalar, Capability::Supported)
    );
    for (p, dims) in [
        ("datasets/empty_f4", vec![0]),
        ("datasets/empty_2d_f4", vec![0, 3]),
    ] {
        let c = s.numeric_capability(p).unwrap();
        assert_eq!(c.dataspace, Dataspace::Simple(dims.clone()));
        assert_eq!(s.read_numeric(p, None).unwrap().shape, dims);
    }
    let e = s.read_strings("datasets/null_str").unwrap_err();
    assert_eq!(e.code, Some(UnsupportedCode::Schema));
    assert_eq!(
        s.read_strings("datasets/empty_str").unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(s.read_strings("datasets/scalar_str").unwrap(), vec!["ab"]);
    // Type mismatches keep their own errors.
    assert_eq!(
        s.read_strings("datasets/null_f4").unwrap_err().kind,
        ErrorKind::InvalidRequest
    );
}
#[test]
fn raw_strings_and_compounds_carry_null_explicitly() {
    let s = session();
    for (p, dataspace, bytes) in [
        ("datasets/null_str", Dataspace::Null, vec![]),
        ("datasets/scalar_str", Dataspace::Scalar, b"ab".to_vec()),
        ("datasets/empty_str", Dataspace::Simple(vec![0]), vec![]),
    ] {
        let r = s
            .raw_fixed_strings(p, 1 << 20)
            .unwrap()
            .into_value()
            .unwrap();
        assert_eq!((r.dataspace, r.raw_bytes), (dataspace, bytes), "{p}");
    }
    for (p, dataspace, rows) in [
        ("datasets/null_compound", Dataspace::Null, 0),
        ("datasets/scalar_compound", Dataspace::Scalar, 1),
        ("datasets/empty_compound", Dataspace::Simple(vec![0]), 0),
    ] {
        let t = s.simple_compound(p, 1 << 20).unwrap();
        assert_eq!(t.dataspace, dataspace, "{p}");
        let MetadataValue::Float(v) = &t.columns[1].value else {
            panic!("{p}")
        };
        assert_eq!(v.len(), rows, "{p}");
    }
}
