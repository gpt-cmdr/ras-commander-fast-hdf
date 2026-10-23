//! Generic dataset string reads prove the stored text encoding before decoding
//! (H4 residual of review MPR01/MPR02). Typed metadata, compound members and
//! `raw_fixed_strings` already refuse variable-length text and reserved string
//! flags; `read_strings` (used by stored family descriptors/values, legacy land-cover
//! `Names`, reference-line `Name` tables and block timestamp checks) previously
//! decoded through the generic reader, which narrows a vlen heap object index to
//! u16, returns an empty string for a missing heap object and normalises reserved
//! padding/charset flags. Fixtures: `tests/generate_generic_string_fixtures.py`.
#![cfg(feature = "hdf")]
use ras_commander_rust::{
    hdf::{families::ReadFamily, ReadOnlyHdfSession},
    ErrorKind, UnsupportedCode,
};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn dir() -> Option<PathBuf> {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/generic_strings");
    if d.is_dir() {
        Some(d)
    } else {
        eprintln!("skipping: {} is absent", d.display());
        None
    }
}

#[test]
fn generic_strings_refuse_unproven_text_encodings() {
    let Some(base) = dir() else { return };
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    for c in cases.as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let path = base.join(name);
        assert_eq!(
            format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap())),
            c["sha256"].as_str().unwrap(),
            "{name}"
        );
        let s = ReadOnlyHdfSession::open(&path).unwrap();
        let strings = s.read_strings("Names");
        let catalogue = s.classification_catalogue(ReadFamily::LandCover, 1 << 20);
        match c["expected"].as_str().unwrap() {
            "text" => {
                // NULLTERM keeps the bytes before the first NUL, exactly as h5py.
                let want = vec!["Grass".to_owned(), "Forest".into(), "Urban".into()];
                assert_eq!(strings.unwrap(), want, "{name}");
                let names: Vec<String> = catalogue
                    .unwrap()
                    .value()
                    .unwrap()
                    .entries
                    .iter()
                    .map(|e| e.name.clone())
                    .collect();
                assert_eq!(names, want, "{name}");
            }
            "unsupported" => {
                let e = strings.expect_err(name);
                assert_eq!(e.kind, ErrorKind::Unsupported, "{name}: {e:?}");
                assert_eq!(e.code, Some(UnsupportedCode::Encoding), "{name}: {e:?}");
                // No empty, aliased or re-padded names escape through the catalogue.
                match catalogue {
                    Err(e) => assert_ne!(e.kind, ErrorKind::Corrupt, "{name}: {e:?}"),
                    Ok(r) => assert!(r.value().is_none(), "{name}: {r:?}"),
                }
            }
            other => panic!("{name}: unknown expectation {other}"),
        }
    }
}
