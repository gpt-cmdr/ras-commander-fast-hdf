#![cfg(feature = "hdf")]
use ras_commander_fast_hdf::hdf::{DecodeBudget, NumericArray, NumericValues, ReadOnlyHdfSession};
use std::{path::Path, sync::Arc, time::Duration};

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hdf_numeric/numeric_types_v1.h5")
}
fn bits(a: &NumericArray) -> Vec<u64> {
    macro_rules! integers { ($($v:ident),*) => { match &a.values {
        $(NumericValues::$v(v) => v.iter().map(|&n| n as u64).collect(),)*
        NumericValues::F32(v) => v.iter().map(|n| u64::from(n.to_bits())).collect(),
        NumericValues::F64(v) => v.iter().map(|n| n.to_bits()).collect(),
        _ => panic!("future numeric kind"),
    } }; }
    integers!(I8, I16, I32, I64, U8, U16, U32, U64)
}
fn poison(a: &mut NumericArray) {
    macro_rules! fill { ($($variant:ident),*) => { match &mut a.values {
        $(NumericValues::$variant(v) => v.fill(17 as _),)*
        _ => panic!("future numeric kind"),
    } }; }
    fill!(I8, I16, I32, I64, U8, U16, U32, U64, F32, F64)
}
fn address(a: &NumericArray) -> usize {
    macro_rules! pointer { ($($v:ident),*) => { match &a.values {
        $(NumericValues::$v(v) => v.as_ptr() as usize,)*
        _ => panic!("future numeric kind"),
    } }; }
    pointer!(I8, I16, I32, I64, U8, U16, U32, U64, F32, F64)
}
#[test]
fn fills_existing_allocations_with_exact_values_and_retains_independent_outputs() {
    let s = ReadOnlyHdfSession::open(fixture()).unwrap();
    for path in [
        "int/i1_bounds",
        "int/i2_bounds",
        "int/i4_bounds",
        "int/i64_bounds",
        "int/u1_bounds",
        "int/u2_bounds",
        "int/u4_bounds",
        "int/u64_bounds",
        "float/f4_special",
        "float/f8_special",
        "shape/chunked_2d_f4",
        "shape/rank3_i2",
        "shape/scalar_f8",
        "shape/empty_1d",
        "shape/empty_2d",
        "filter/gzip_shuffle_f8",
        "fill/chunked_partial_i4",
        "fill/contiguous_unwritten_f8",
    ] {
        let original = s.read_numeric(path, None).unwrap();
        let mut out = original.clone();
        let before = address(&out);
        for _ in 0..3 {
            poison(&mut out);
            s.read_numeric_into(path, &mut out).unwrap();
            assert_eq!(address(&out), before, "{path}");
            assert_eq!(bits(&out), bits(&original), "{path}");
        }
    }
}
#[test]
fn rejects_bad_shape_dtype_length_budget_and_source_without_touching_destination() {
    let s = ReadOnlyHdfSession::open(fixture()).unwrap();
    let mut out = s.read_numeric("int/i4_bounds", None).unwrap();
    let before = bits(&out);
    for path in [
        "missing",
        "class/compound",
        "encoding/i32_precision24_offset4",
        "endian/be_i4",
        "float/f4_special",
    ] {
        assert!(s.read_numeric_into(path, &mut out).is_err());
        assert_eq!(bits(&out), before);
    }
    assert!(s
        .read_numeric_into_bounded("int/i4_bounds", &mut out, 11)
        .is_err());
    assert_eq!(bits(&out), before);
    out.shape = vec![1, 3];
    assert!(s.read_numeric_into("int/i4_bounds", &mut out).is_err());
    assert_eq!(bits(&out), before);
    out.shape = vec![3];
    if let NumericValues::I32(v) = &mut out.values {
        v.pop();
    }
    let before = bits(&out);
    assert!(s.read_numeric_into("int/i4_bounds", &mut out).is_err());
    assert_eq!(bits(&out), before);
}
#[test]
fn decode_reservation_precedes_writes_and_null_is_never_a_scalar() {
    let s = ReadOnlyHdfSession::open(fixture()).unwrap();
    let mut out = s.read_numeric("filter/gzip_shuffle_f8", None).unwrap();
    let before = bits(&out);
    s.set_decode_budget(Arc::new(DecodeBudget::new(1, Duration::ZERO).unwrap()));
    assert!(s
        .read_numeric_into("filter/gzip_shuffle_f8", &mut out)
        .is_err());
    assert_eq!(bits(&out), before);
    let null = ReadOnlyHdfSession::open(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dataspace/dataspace.h5"),
    )
    .unwrap();
    let mut scalar = ReadOnlyHdfSession::open(fixture())
        .unwrap()
        .read_numeric("shape/scalar_f8", None)
        .unwrap();
    let before = bits(&scalar);
    let error = null
        .read_numeric_into("datasets/null_f4", &mut scalar)
        .unwrap_err();
    assert_eq!(
        error.code,
        Some(ras_commander_fast_hdf::UnsupportedCode::Schema)
    );
    assert_eq!(bits(&scalar), before);
}

#[test]
fn policy_refusal_preserves_output_and_late_checksum_failure_does_not_poison_reuse() {
    let reference = ReadOnlyHdfSession::open(fixture()).unwrap();
    let policy = ReadOnlyHdfSession::open(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/reader_policy/filter_lz4_then_deflate.h5"),
    )
    .unwrap();
    let mut out = reference
        .read_numeric("filter/gzip_shuffle_f8", None)
        .unwrap();
    out.shape = policy.numeric_capability("data").unwrap().shape;
    if let NumericValues::F64(v) = &mut out.values {
        v.resize(48, 17.0);
        v.fill(17.0);
    }
    let before = bits(&out);
    let err = policy.read_numeric_into("data", &mut out).unwrap_err();
    assert_eq!(
        err.code,
        Some(ras_commander_fast_hdf::UnsupportedCode::Encoding)
    );
    assert_eq!(bits(&out), before);

    // Alter the last uncompressed Fletcher32 chunk, located by its unique
    // known sample prefix rather than a generator-dependent file offset.
    let mut data = std::fs::read(fixture()).unwrap();
    let pattern = (256i64..264).flat_map(i64::to_le_bytes).collect::<Vec<_>>();
    let offsets = data
        .windows(pattern.len())
        .enumerate()
        .filter_map(|(i, b)| (b == pattern).then_some(i))
        .collect::<Vec<_>>();
    assert_eq!(offsets.len(), 1);
    data[offsets[0]] ^= 1;
    let session = ReadOnlyHdfSession::from_bytes(data, "checksum-failure.h5", 1, false).unwrap();
    let mut out = reference
        .read_numeric("filter/fletcher32_i8", None)
        .unwrap();
    poison(&mut out);
    let pointer = address(&out);
    let error = session
        .read_numeric_into("filter/fletcher32_i8", &mut out)
        .unwrap_err();
    assert_eq!(error.kind, ras_commander_fast_hdf::ErrorKind::Corrupt);
    assert!(error.message.contains("checksum"));
    if let NumericValues::I64(v) = &mut out.values {
        assert_eq!(&v[..64], &(0..64i64).collect::<Vec<_>>());
        v.truncate(6);
    }
    out.shape = vec![6];
    session
        .read_numeric_into("int/i64_bounds", &mut out)
        .unwrap();
    assert_eq!(address(&out), pointer);
    assert_eq!(
        bits(&out),
        bits(&reference.read_numeric("int/i64_bounds", None).unwrap())
    );
}
