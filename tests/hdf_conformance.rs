//! Independent h5py/NumPy oracles generated 2026-10-18 from immutable real files.
//! Real tests are explicitly ignored by default; requested qualification fails if absent.
#![cfg(all(feature = "hdf", feature = "results"))]
use ras_commander_fast_hdf::hdf::{NumericValues, ReadOnlyHdfSession, Slice};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
fn bytes(values: &NumericValues) -> Vec<u8> {
    macro_rules! b {
        ($v:expr) => {
            $v.iter().flat_map(|x| x.to_le_bytes()).collect()
        };
    }
    match values {
        NumericValues::I8(v) => b!(v),
        NumericValues::I16(v) => b!(v),
        NumericValues::I32(v) => b!(v),
        NumericValues::I64(v) => b!(v),
        NumericValues::U8(v) => v.clone(),
        NumericValues::U16(v) => b!(v),
        NumericValues::U32(v) => b!(v),
        NumericValues::U64(v) => b!(v),
        NumericValues::F32(v) => b!(v),
        NumericValues::F64(v) => b!(v),
        _ => panic!("new numeric variant requires qualification"),
    }
}
fn hex(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn fixture(name: &str, expected: &str) -> PathBuf {
    let p = PathBuf::from(
        std::env::var_os("RAS_NATIVE_FIXTURES")
            .expect("set RAS_NATIVE_FIXTURES to immutable5/6/7 fixture directory"),
    )
    .join(name);
    assert_eq!(file_sha256(&p), expected);
    p
}
/// Streamed SHA-256, so multi-GB fixtures are not read into memory.
fn file_sha256(path: &std::path::Path) -> String {
    use std::io::Read;
    let mut file = std::fs::File::open(path).expect("real fixture required");
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1 << 22];
    loop {
        let n = file.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    format!("{:x}", hash.finalize())
}

#[test]
#[ignore = "requires real5/6/7 fixture directory; run with RAS_NATIVE_FIXTURES"]
fn real_hdf_0() {
    let p = fixture(
        "AustinOyster.p03.hdf",
        "fdaff5045ad36bf5d424489ff64b3d86a1e0e3ef92a628554488f009047cad92",
    );
    for threads in [1, 4] {
        let s = ReadOnlyHdfSession::with_options(&p, threads, true).unwrap();
        let path="Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/2D Flow Areas/Perimeter 1/Water Surface";
        let cap = s.numeric_capability(path).unwrap();
        assert_eq!(cap.shape, vec![73, 1882841]);
        let a = s
            .read_numeric(path, Some(&[Slice::new(0, 13, 3), Slice::new(0, 19, 2)]))
            .unwrap();
        assert_eq!(a.shape, vec![5, 10]);
        assert_eq!(
            hex(&bytes(&a.values)),
            "de7fe9152836b58d102d81226a0631931ec6b0ad68ba74b67a1bda9638181786"
        );
        let a = s
            .read_numeric(
                "Geometry/2D Flow Areas/Perimeter 1/FacePoints Coordinate",
                None,
            )
            .unwrap();
        assert_eq!(
            hex(&bytes(&a.values)),
            "5171f06a386c5acbe430bc51b4c58d6dba4b4e4901cde9abcded3dd8992eecc2"
        );
        let reduced = s
            .reduce_temporal(
                path,
                ras_commander_fast_hdf::results::ReductionRequest::default(),
            )
            .unwrap();
        let mut out = Vec::new();
        for v in reduced.values.maximum {
            out.extend(v.to_le_bytes());
        }
        for v in reduced.values.peak_index {
            out.extend(v.to_le_bytes());
        }
        for v in reduced.values.finite_count {
            out.extend(v.to_le_bytes());
        }
        assert_eq!(
            hex(&out),
            "deb83a336ca4c26b973c24a352566c14ba11e0cbd8be63eac8ad12473653777a"
        );
    }
    assert_eq!(
        file_sha256(&p),
        "fdaff5045ad36bf5d424489ff64b3d86a1e0e3ef92a628554488f009047cad92"
    );
}

#[test]
#[ignore = "requires real5/6/7 fixture directory; run with RAS_NATIVE_FIXTURES"]
fn real_hdf_1() {
    let p = fixture(
        "BaldEagleDamBrk.p07.hdf",
        "07cad3930a679891f87bbface1ab749b82fd7d7eae6aaac3b5ff041c44280984",
    );
    for threads in [1, 4] {
        let s = ReadOnlyHdfSession::with_options(&p, threads, true).unwrap();
        let path="Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/2D Flow Areas/BaldEagleCr/Water Surface";
        let cap = s.numeric_capability(path).unwrap();
        assert_eq!(cap.shape, vec![865, 89879]);
        let a = s
            .read_numeric(path, Some(&[Slice::new(0, 13, 3), Slice::new(0, 19, 2)]))
            .unwrap();
        assert_eq!(a.shape, vec![5, 10]);
        assert_eq!(
            hex(&bytes(&a.values)),
            "e09e4a62595ca578f9963c8708398cf039c108fcd68401b52d63b2fde1a90bb1"
        );
        let a = s
            .read_numeric(
                "Geometry/2D Flow Areas/BaldEagleCr/FacePoints Coordinate",
                None,
            )
            .unwrap();
        assert_eq!(
            hex(&bytes(&a.values)),
            "963b3940e7b0e5b9afc826a2958b94e48bb63f73081c0ab7a2c26ad1780c6df5"
        );
        let reduced = s
            .reduce_temporal(
                path,
                ras_commander_fast_hdf::results::ReductionRequest::default(),
            )
            .unwrap();
        let mut out = Vec::new();
        for v in reduced.values.maximum {
            out.extend(v.to_le_bytes());
        }
        for v in reduced.values.peak_index {
            out.extend(v.to_le_bytes());
        }
        for v in reduced.values.finite_count {
            out.extend(v.to_le_bytes());
        }
        assert_eq!(
            hex(&out),
            "713bdf1a90b643a8895bc6048b8f8b3374d31eed9deae72287fc065b3e8b3046"
        );
    }
    assert_eq!(
        file_sha256(&p),
        "07cad3930a679891f87bbface1ab749b82fd7d7eae6aaac3b5ff041c44280984"
    );
}

#[test]
#[ignore = "requires real5/6/7 fixture directory; run with RAS_NATIVE_FIXTURES"]
fn real_hdf_2() {
    let p = fixture(
        "BaldEagleDamBrk.p18.hdf",
        "1d0063a84fd131ac95ffaa2b3f6647f2ef51c7f38f23c43e8ae6c698bfc95f0b",
    );
    for threads in [1, 4] {
        let s = ReadOnlyHdfSession::with_options(&p, threads, true).unwrap();
        let path="Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/2D Flow Areas/BaldEagleCr/Water Surface";
        let cap = s.numeric_capability(path).unwrap();
        assert_eq!(cap.shape, vec![865, 3947]);
        let a = s
            .read_numeric(path, Some(&[Slice::new(0, 13, 3), Slice::new(0, 19, 2)]))
            .unwrap();
        assert_eq!(a.shape, vec![5, 10]);
        assert_eq!(
            hex(&bytes(&a.values)),
            "b35cd54d33d09f1cee8aa973af91f00a81854a74387c8bfb9ebee992ab70e768"
        );
        let a = s
            .read_numeric(
                "Geometry/2D Flow Areas/BaldEagleCr/FacePoints Coordinate",
                None,
            )
            .unwrap();
        assert_eq!(
            hex(&bytes(&a.values)),
            "c4cd5a9d26d9412e2fc4d22e5884f228e19f16415aba26e9e9c6e427fd0d66ee"
        );
        let reduced = s
            .reduce_temporal(
                path,
                ras_commander_fast_hdf::results::ReductionRequest::default(),
            )
            .unwrap();
        let mut out = Vec::new();
        for v in reduced.values.maximum {
            out.extend(v.to_le_bytes());
        }
        for v in reduced.values.peak_index {
            out.extend(v.to_le_bytes());
        }
        for v in reduced.values.finite_count {
            out.extend(v.to_le_bytes());
        }
        assert_eq!(
            hex(&out),
            "52577e8698225dbb01278a528add988c1ad9df0bcc94b18e17ea42332d0747a2"
        );
        let path="Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/2D Flow Areas/Upper 2D Area/Water Surface";
        let cap = s.numeric_capability(path).unwrap();
        assert_eq!(cap.shape, vec![865, 1251]);
        let a = s
            .read_numeric(path, Some(&[Slice::new(0, 13, 3), Slice::new(0, 19, 2)]))
            .unwrap();
        assert_eq!(a.shape, vec![5, 10]);
        assert_eq!(
            hex(&bytes(&a.values)),
            "b9cdd0e177f3a8a4e9338f905eabd4a28b21d94e77109953d0a127862b03b577"
        );
        let a = s
            .read_numeric(
                "Geometry/2D Flow Areas/Upper 2D Area/FacePoints Coordinate",
                None,
            )
            .unwrap();
        assert_eq!(
            hex(&bytes(&a.values)),
            "004178a604c943cdadbf649acf625ffc8545b0b059bfab88863abe2bd55389a7"
        );
        let reduced = s
            .reduce_temporal(
                path,
                ras_commander_fast_hdf::results::ReductionRequest::default(),
            )
            .unwrap();
        let mut out = Vec::new();
        for v in reduced.values.maximum {
            out.extend(v.to_le_bytes());
        }
        for v in reduced.values.peak_index {
            out.extend(v.to_le_bytes());
        }
        for v in reduced.values.finite_count {
            out.extend(v.to_le_bytes());
        }
        assert_eq!(
            hex(&out),
            "6241940644f0bdc57b1e489c2d09aecd69c98ffb21fe518f24413014e50fc66f"
        );
    }
    assert_eq!(
        file_sha256(&p),
        "1d0063a84fd131ac95ffaa2b3f6647f2ef51c7f38f23c43e8ae6c698bfc95f0b"
    );
}

#[test]
fn io_and_corrupt_are_distinct() {
    use ras_commander_fast_hdf::ErrorKind;
    let root = std::env::temp_dir().join(format!("ras-native-corrupt-{}", std::process::id()));
    assert!(
        matches!(ReadOnlyHdfSession::open(root.join("absent.hdf")),Err(e) if e.kind==ErrorKind::FileNotFound)
    );
    std::fs::write(&root, b"not an HDF file").unwrap();
    assert!(matches!(ReadOnlyHdfSession::open(&root),Err(e) if e.kind==ErrorKind::Corrupt));
    std::fs::remove_file(root).unwrap();
}

#[test]
fn numeric_encoding_matrix_exact_or_explicit_rejection() {
    use ras_commander_fast_hdf::ErrorKind;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hdf_numeric");
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/hdf_numeric/manifest.json")).unwrap();
    for fixture in manifest["fixtures"].as_array().unwrap() {
        let path = root.join(fixture["file"].as_str().unwrap());
        assert_eq!(
            hex(&std::fs::read(&path).unwrap()),
            fixture["sha256"].as_str().unwrap()
        );
        for threads in [1, 4] {
            for parallel in [false, true] {
                let s = ReadOnlyHdfSession::with_options(&path, threads, parallel).unwrap();
                for case in fixture["cases"].as_array().unwrap() {
                    let path = case["path"].as_str().unwrap();
                    let verdict = case["verdict"].as_str().unwrap();
                    match s.read_numeric(path, None) {
                        Ok(array) => {
                            assert_ne!(verdict, "reject", "{path}");
                            let shape: Vec<u64> = case["shape"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|v| v.as_u64().unwrap())
                                .collect();
                            assert_eq!(array.shape, shape, "{path}");
                            assert_eq!(
                                hex(&bytes(&array.values)),
                                case["sha256"].as_str().unwrap(),
                                "{path}"
                            );
                            assert!(s.numeric_capability(path).unwrap().kind.is_some());
                        }
                        Err(e) => {
                            assert_ne!(verdict, "exact", "{path}: {e}");
                            assert_eq!(e.kind, ErrorKind::Unsupported, "{path}: {e}");
                        }
                    }
                }
                // Typed precision, request validation, ordered batch, missing vs unsupported.
                let a = s.read_numeric("int/u64_bounds", None).unwrap();
                let NumericValues::U64(v) = a.values else {
                    panic!("uint64 changed type")
                };
                assert_eq!(v[3..6], [1 << 53, (1 << 53) + 1, 1 << 63]);
                assert_eq!(*v.last().unwrap(), u64::MAX);
                assert!(
                    matches!(s.read_numeric("int/missing",None),Err(e) if e.kind==ErrorKind::NotFound)
                );
                assert!(
                    matches!(s.read_numeric("shape/rank3_i2",Some(&[Slice::new(0,4,0)])),Err(e) if e.kind==ErrorKind::InvalidRequest)
                );
                let batch = s
                    .read_numeric_batch(&["int/u64_bounds", "int/u64_bounds"])
                    .unwrap();
                assert_eq!(batch.len(), 2);
                assert_eq!(batch[0], batch[1]);
                assert!(
                    matches!(s.read_numeric_batch(&["int/u64_bounds","class/compound"]),Err(e) if e.kind==ErrorKind::Unsupported)
                );
            }
        }
    }
}
#[test]
fn selected_values_outlive_session() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hdf_numeric/numeric_types_v2.h5");
    let a = {
        let s = ReadOnlyHdfSession::open(path).unwrap();
        s.read_numeric("int/u64_bounds", Some(&[Slice::new(3, 8, 2)]))
            .unwrap()
    };
    assert_eq!(a.shape, vec![3]);
    let NumericValues::U64(v) = a.values else {
        panic!("wrong type")
    };
    assert_eq!(v, vec![1 << 53, 1 << 63, u64::MAX]);
}

#[test]
fn huge_single_element_strides_and_thread_bounds_are_checked() {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hdf_numeric/numeric_types_v2.h5");
    assert!(
        matches!(ReadOnlyHdfSession::with_options(&p,usize::MAX,true),Err(e) if e.kind==ras_commander_fast_hdf::ErrorKind::InvalidRequest)
    );
    for parallel in [false, true] {
        let s = ReadOnlyHdfSession::with_options(&p, 2, parallel).unwrap();
        let a = s
            .read_numeric(
                "shape/chunked_2d_f4",
                Some(&[Slice::new(1, 37, u64::MAX), Slice::new(1, 23, u64::MAX)]),
            )
            .unwrap();
        let b = s
            .read_numeric(
                "shape/chunked_2d_f4",
                Some(&[Slice::new(1, 2, 1), Slice::new(1, 2, 1)]),
            )
            .unwrap();
        assert_eq!(a, b);
    }
}

#[test]
fn raw_metadata_retains_payload_and_capability_encoding() {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hdf_numeric/numeric_types_v1.h5");
    let s = ReadOnlyHdfSession::open(&p).unwrap();
    let attrs = s.dataset_attributes("header/continued_u4").unwrap();
    assert_eq!(attrs.len(), 60);
    assert!(attrs
        .iter()
        .all(|a| a.shape == vec![8] && a.raw_bytes.len() == 64 && a.strings.is_none()));
    let c = s
        .numeric_capability("encoding/i32_precision24_offset4")
        .unwrap();
    let enc = c.encoding.unwrap();
    assert_eq!(enc.bit_offset, Some(4));
    assert_eq!(enc.bit_precision, Some(24));
    assert!(c.kind.is_none());
    let c = s.numeric_capability("float/f8_special").unwrap();
    let enc = c.encoding.unwrap();
    assert_eq!(enc.float.unwrap().exponent_bias, 1023);
    assert!(c.kind.is_some());
}
#[test]
#[ignore = "requires immutable real5/6/7 fixtures"]
fn real_version_and_time_string_metadata() {
    for (name, sha, version, rows) in [
        (
            "AustinOyster.p03.hdf",
            "fdaff5045ad36bf5d424489ff64b3d86a1e0e3ef92a628554488f009047cad92",
            "5.0.7",
            73,
        ),
        (
            "BaldEagleDamBrk.p07.hdf",
            "07cad3930a679891f87bbface1ab749b82fd7d7eae6aaac3b5ff041c44280984",
            "6.6",
            865,
        ),
        (
            "BaldEagleDamBrk.p18.hdf",
            "1d0063a84fd131ac95ffaa2b3f6647f2ef51c7f38f23c43e8ae6c698bfc95f0b",
            "7.0",
            865,
        ),
    ] {
        let p = fixture(name, sha);
        let s = ReadOnlyHdfSession::open(&p).unwrap();
        let attrs = s.group_attributes("/").unwrap();
        let ver = attrs
            .iter()
            .find(|a| a.name == "File Version")
            .expect("stored File Version");
        let strings = ver.strings.as_ref().expect("fixedstring decoded");
        assert!(strings[0].contains(version));
        let times=s.read_strings("Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/Time Date Stamp").unwrap();
        assert_eq!(times.len(), rows, "{name}");
        assert!(times.iter().all(|t| !t.is_empty()));
    }
}

#[test]
fn float64_reduction_budgets_stored_width_and_one_row_floor() {
    use ras_commander_fast_hdf::results::ReductionRequest;
    for name in ["numeric_types_v1.h5", "numeric_types_v2.h5"] {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/hdf_numeric")
            .join(name);
        for threads in [1, 4] {
            let s = ReadOnlyHdfSession::with_options(&p, threads, true).unwrap();
            let out = s
                .reduce_temporal(
                    "filter/gzip_shuffle_f8",
                    ReductionRequest::new().with_max_block_bytes(64),
                )
                .unwrap();
            assert_eq!(out.stored_item_bytes, 8);
            assert_eq!(out.block_rows, 1);
            assert_eq!(out.blocks, 40);
            assert_eq!(out.storage_chunk_bytes, Some(2048));
            for i in 0..50 {
                assert_eq!(out.values.maximum[i], ((1950 + i) as f64 / 7.) as f32);
                assert_eq!(out.values.peak_index[i], 39);
                assert_eq!(out.values.finite_count[i], 40);
            }
            assert!(
                matches!(s.reduce_temporal("filter/gzip_shuffle_f8",ReductionRequest::new().with_max_block_bytes(0)),Err(e) if e.kind==ras_commander_fast_hdf::ErrorKind::InvalidRequest)
            );
        }
    }
}
