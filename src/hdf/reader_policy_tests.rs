//! The engine's read policies (src/hdf/reader.rs) refuse exactly what the
//! reader the engine embedded up to 0.2.3 refused (its patches 0008 and 0014),
//! with the same error kind, code, subject and message. Fixtures and their
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

#[test]
fn extensible_arrays_past_the_index_block_are_refused() {
    let gathers: &[(Slice, &[u64])] = &[
        (Slice::new(35, 50, 1), &[0, 100]),
        (Slice::new(0, 4, 1), &[3, 1]),
        (Slice::new(2, 50, 7), &[100, 0, 50, 50]),
    ];
    assert_refused(
        "ea_latest.h5",
        NumericKind::F32,
        gathers,
        "unsupported non-inline extensible-array chunk index (unqualified block mapping)",
    );
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
