//! The engine's read policy (src/hdf/reader.rs) refuses exactly what the
//! reader the engine embedded up to 0.2.3 refused with its patch 0008, with the
//! same error kind, code, subject and message. Extensible-array chunk indexes
//! past their index block (formerly refused by patch 0014) read as h5py reads
//! them. Fixtures and their
//! generator: tests/fixtures/reader_policy.
use super::*;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reader_policy")
        .join(name)
}

/// (kind, code, subject, message) of a refusal.
type Shape = (ErrorKind, Option<UnsupportedCode>, Option<String>, String);

fn shape(e: Error) -> Shape {
    (e.kind, e.code, e.subject, e.message)
}

fn unsupported(subject: bool, message: &str) -> Shape {
    (
        ErrorKind::Unsupported,
        Some(UnsupportedCode::Encoding),
        subject.then(|| "data".to_string()),
        if subject {
            format!("data: {message}")
        } else {
            message.to_string()
        },
    )
}

/// Every read path the engine uses on "data": whole reads (cached, cache
/// bypass, serial and parallel), bounded slices, a slice that covers the
/// whole dataset, and row/column gathers. Metadata stays readable.
fn assert_refused(name: &str, kind: NumericKind, gathers: &[(Slice, &[u64])], message: &str) {
    for threads in [1, 4] {
        let s = ReadOnlyHdfSession::with_options(fixture(name), threads, true).unwrap();
        let capability = s.numeric_capability("data").unwrap();
        assert_eq!(capability.kind, Some(kind), "{name}");
        assert_eq!(capability.capability, Capability::Supported, "{name}");
        assert!(s.dataset_attributes("data").is_ok(), "{name}");
        let [rows, cols] = capability.shape[..] else {
            panic!("{name}: expected 2-D data")
        };
        let whole = unsupported(true, message);
        let part = unsupported(false, message);
        assert_eq!(shape(s.read_numeric("data", None).unwrap_err()), whole);
        let all = [Slice::new(0, rows, 1), Slice::new(0, cols, 1)];
        assert_eq!(
            shape(s.read_numeric("data", Some(&all)).unwrap_err()),
            whole
        );
        let some = [Slice::new(1, rows, 2), Slice::new(0, cols, 3)];
        assert_eq!(
            shape(s.read_numeric("data", Some(&some)).unwrap_err()),
            part
        );
        for &(rows, columns) in gathers {
            assert_eq!(
                shape(s.gather_numeric("data", kind, rows, columns).unwrap_err()),
                part,
                "{name}: gather {rows:?} {columns:?}"
            );
        }
        // Selections of no elements read no chunk and are not refused.
        let empty = [Slice::new(1, 1, 1), Slice::new(0, cols, 1)];
        assert_eq!(
            s.read_numeric("data", Some(&empty)).unwrap().shape,
            vec![0, cols]
        );
        let none = s
            .gather_numeric("data", kind, Slice::new(1, 1, 1), &[0])
            .unwrap()
            .unwrap();
        assert_eq!(none.shape, vec![0, 1]);
        // A whole read larger than the chunk cache bypasses it.
        let bypass = ReadOnlyHdfSession::with_cache_options(fixture(name), threads, true, 0, 1)
            .unwrap()
            .read_numeric("data", None)
            .unwrap_err();
        assert_eq!(shape(bypass), whole);
    }
}

#[test]
fn unbounded_intermediate_filters_are_refused() {
    let gathers: &[(Slice, &[u64])] = &[
        (Slice::new(1, 5, 2), &[0, 7, 3]),
        (Slice::new(0, 6, 1), &[7]),
    ];
    for (name, id) in [
        ("filter_lz4_then_deflate.h5", 32004),
        ("filter_unknown_then_deflate.h5", 40000),
    ] {
        assert_refused(
            name,
            NumericKind::F64,
            gathers,
            &format!(
                "unsupported filter: filter {id} has no qualified encoded-size bound in a bounded pipeline"
            ),
        );
    }
}

#[test]
fn a_final_lz4_stage_is_decoded_not_refused() {
    // LZ4 is decoded first, bounded by the chunk size; the chunk is not LZ4
    // data, so the read fails as corrupt (not unsupported).
    let s = ReadOnlyHdfSession::open(fixture("filter_deflate_then_lz4.h5")).unwrap();
    let e = s.read_numeric("data", None).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Corrupt, "{e:?}");
    assert!(
        e.message.starts_with("data: decompression error: LZ4"),
        "{e:?}"
    );
}

/// The SHA-256 h5py gave for every value of `name` (`manifest.json`, written by
/// `make_fixtures.py`, which also checks the values against their formula).
fn h5py_sha256(name: &str) -> String {
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("manifest.json")).unwrap()).unwrap();
    manifest["h5py_values_sha256"][name]
        .as_str()
        .unwrap()
        .to_string()
}

fn f32_values(values: &NumericValues) -> &[f32] {
    match values {
        NumericValues::F32(v) => v,
        other => panic!("{other:?}"),
    }
}

#[test]
fn extensible_arrays_past_the_index_block_read_as_h5py_reads_them() {
    use sha2::{Digest, Sha256};
    // ea_latest.h5: 50 chunks (beyond the index block); ea_deep.h5: 9000 chunks
    // (super blocks and paged data blocks). Values are row * cols + col + 1.
    for (name, rows, cols) in [("ea_latest.h5", 50u64, 101u64), ("ea_deep.h5", 9000, 2)] {
        let value = |r: u64, c: u64| (r * cols + c + 1) as f32;
        for threads in [1, 4] {
            let s = ReadOnlyHdfSession::with_options(fixture(name), threads, true).unwrap();
            let full = s.read_numeric("data", None).unwrap();
            assert_eq!(full.shape, vec![rows, cols], "{name}");
            let bytes: Vec<u8> = f32_values(&full.values)
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect();
            let hex: String = Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assert_eq!(
                hex,
                h5py_sha256(name),
                "{name}: every value as h5py reads it"
            );
            let part = s
                .read_numeric(
                    "data",
                    Some(&[Slice::new(rows / 3, rows, 7), Slice::new(1, cols, 2)]),
                )
                .unwrap();
            let want: Vec<f32> = (rows / 3..rows)
                .step_by(7)
                .flat_map(|r| (1..cols).step_by(2).map(move |c| value(r, c)))
                .collect();
            assert_eq!(f32_values(&part.values), want.as_slice(), "{name}: slice");
            let columns = [cols - 1, 0, cols / 2, cols / 2];
            let gathered = s
                .gather_numeric(
                    "data",
                    NumericKind::F32,
                    Slice::new(rows - 40, rows, 3),
                    &columns,
                )
                .unwrap()
                .unwrap();
            let want: Vec<f32> = (rows - 40..rows)
                .step_by(3)
                .flat_map(|r| columns.iter().map(move |&c| value(r, c)))
                .collect();
            assert_eq!(
                f32_values(&gathered.values),
                want.as_slice(),
                "{name}: gather"
            );
        }
    }
}

#[test]
fn extensible_arrays_within_the_index_block_are_read() {
    let expected: Vec<f64> = (0..15).map(f64::from).collect();
    for threads in [1, 4] {
        let s = ReadOnlyHdfSession::with_options(fixture("ea_inline.h5"), threads, true).unwrap();
        let full = s.read_numeric("data", None).unwrap();
        assert_eq!(full.shape, vec![3, 5]);
        assert_eq!(full.values, NumericValues::F64(expected.clone()));
        let part = s
            .read_numeric("data", Some(&[Slice::new(1, 3, 1), Slice::new(1, 5, 2)]))
            .unwrap();
        assert_eq!(part.values, NumericValues::F64(vec![6.0, 8.0, 11.0, 13.0]));
        let gathered = s
            .gather_numeric("data", NumericKind::F64, Slice::new(0, 3, 2), &[4, 0])
            .unwrap()
            .unwrap();
        assert_eq!(
            gathered.values,
            NumericValues::F64(vec![4.0, 0.0, 14.0, 10.0])
        );
    }
}

#[test]
fn malformed_extensible_array_headers_fail_closed() {
    let data = std::fs::read(fixture("ea_inline.h5")).unwrap();
    let at = data
        .windows(4)
        .position(|w| w == b"EAHD")
        .expect("extensible-array header");
    let read = |bytes: Vec<u8>| {
        ReadOnlyHdfSession::from_bytes(bytes, "ea.h5", 1, false)
            .unwrap()
            .read_numeric("data", None)
            .unwrap_err()
    };
    // A changed element count no longer matches the header checksum.
    let mut bad_checksum = data.clone();
    bad_checksum[at + 12 + 5 * 8] ^= 0x40;
    let e = read(bad_checksum);
    assert_eq!(e.kind, ErrorKind::Corrupt, "{e:?}");
    assert!(e.message.contains("checksum mismatch"), "{e:?}");
    let mut bad_version = data.clone();
    bad_version[at + 4] = 1;
    let e = read(bad_version);
    assert_eq!(e.kind, ErrorKind::Unsupported, "{e:?}");
    assert_eq!(
        e.message,
        "data: unsupported extensible array header version 1"
    );
    let mut bad_signature = data;
    bad_signature[at] = b'X';
    let e = read(bad_signature);
    assert_eq!(e.kind, ErrorKind::Corrupt, "{e:?}");
    assert!(e.message.contains("extensible array signature"), "{e:?}");
}
