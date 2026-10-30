#![cfg(feature = "hdf")]
//! `hec_ras_file_type_of_bytes` is `from_bytes` then `hec_ras_file_type`, on every committed
//! HDF5 fixture and on the placements a prefix does not hold (`tests/fixtures/file_type`,
//! generator `tests/generate_file_type_fixtures.py`).
use ras_commander_fast_hdf::hdf::metadata::StoredFileType;
use ras_commander_fast_hdf::hdf::ReadOnlyHdfSession;
use ras_commander_fast_hdf::Result;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(&path, out);
        } else if path
            .extension()
            .is_some_and(|x| x == "h5" || x == "hdf" || x == "hdf5")
        {
            out.push(path);
        }
    }
}

fn whole(data: &[u8], name: &str) -> Result<StoredFileType> {
    ReadOnlyHdfSession::from_bytes(data.to_vec(), name, 1, false)
        .and_then(|s| s.hec_ras_file_type())
}

/// Same verdict, or the same error kind, code and message.
fn same(a: &Result<StoredFileType>, b: &Result<StoredFileType>) -> bool {
    match (a, b) {
        (Ok(a), Ok(b)) => a == b,
        (Err(a), Err(b)) => a.kind == b.kind && a.code == b.code && a.to_string() == b.to_string(),
        _ => false,
    }
}

#[test]
fn file_type_of_bytes_is_the_session_file_type_on_every_fixture() {
    let mut all = Vec::new();
    files(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        &mut all,
    );
    assert!(all.len() > 100, "{} fixtures", all.len());
    let mut hec_ras = 0;
    for path in &all {
        let data = std::fs::read(path).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        let sniffed = ReadOnlyHdfSession::hec_ras_file_type_of_bytes(&data, name);
        let full = whole(&data, name);
        assert!(
            same(&sniffed, &full),
            "{}: {sniffed:?} vs {full:?}",
            path.display()
        );
        hec_ras += usize::from(matches!(sniffed, Ok(StoredFileType::HecRas(_))));
    }
    assert!(hec_ras > 0);
}

#[test]
fn file_type_fixtures_match_their_provenance() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/file_type");
    let provenance: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("provenance.json")).unwrap()).unwrap();
    let listed = provenance["files"].as_object().unwrap();
    assert_eq!(listed.len(), 3);
    for (file, sha) in listed {
        let bytes = std::fs::read(dir.join(file)).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            sha.as_str().unwrap(),
            "{file}"
        );
    }
}

#[test]
fn bytes_that_are_not_whole_hdf5_files_are_refused_as_from_bytes_refuses_them() {
    for data in [&b""[..], b"not an hdf5 file", &[0u8; 70_000][..]] {
        let sniffed = ReadOnlyHdfSession::hec_ras_file_type_of_bytes(data, "upload.bin");
        assert!(sniffed.is_err());
        assert!(same(&sniffed, &whole(data, "upload.bin")), "{sniffed:?}");
    }
    // A truncated file: whatever the reader reports for those bytes whole.
    let data = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/file_type/late_dense_attributes.h5"),
    )
    .unwrap();
    for cut in [100, 4096, 70_000, data.len() - 1] {
        let sniffed = ReadOnlyHdfSession::hec_ras_file_type_of_bytes(&data[..cut], "cut.h5");
        let full = whole(&data[..cut], "cut.h5");
        assert!(same(&sniffed, &full), "cut {cut}: {sniffed:?} vs {full:?}");
    }
}
