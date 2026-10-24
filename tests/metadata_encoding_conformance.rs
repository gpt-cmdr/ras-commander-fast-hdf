#![cfg(feature = "hdf")]
use ras_commander_fast_hdf::{
    hdf::{metadata::MetadataValue as V, ReadOnlyHdfSession},
    ErrorKind,
};

/// Frozen evidence directory: `var` when set, else the copy committed under
/// `tests/fixtures/<fixture>`. `None` (the test skips) when neither exists, so a
/// partial checkout never fails on missing inputs.
fn evidence_dir(var: &str, fixture: &str) -> Option<std::path::PathBuf> {
    let dir = std::env::var_os(var)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(fixture)
        });
    if dir.is_dir() {
        Some(dir)
    } else {
        eprintln!("skipping: evidence directory {} is absent", dir.display());
        None
    }
}

#[test]
fn byte_level_metadata_encoding_boundaries() {
    use sha2::{Digest, Sha256};
    let Some(base) = evidence_dir("RAS_ENCODING_EVIDENCE", "encoding_evidence") else {
        return;
    };
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("malformed-manifest.json")).unwrap())
            .unwrap();
    for c in cases.as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let path = base.join("malformed").join(name);
        assert_eq!(
            format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap())),
            c["sha256"].as_str().unwrap()
        );
        let result = ReadOnlyHdfSession::open(&path);
        let expected = c["expected"].as_str().unwrap();
        if expected == "corrupt" {
            let error = match result {
                Err(e) => e,
                Ok(s) => s.typed_group_attributes("/").unwrap_err(),
            };
            assert_eq!(error.kind, ErrorKind::Corrupt, "{name}");
            continue;
        }
        let s = result.unwrap();
        if ["compound", "legacy", "corrupt_compound"].contains(&expected) {
            let table = s.simple_compound("table", 1024);
            if expected == "corrupt_compound" {
                assert_eq!(table.unwrap_err().kind, ErrorKind::Corrupt);
                continue;
            }
            let table = table.unwrap();
            if expected == "legacy" {
                assert!(matches!(&table.columns[0].value, V::Unsupported { .. }));
            } else {
                assert_eq!(table.columns[0].value, V::Strings(vec!["abcd".into()]));
                assert_eq!(table.columns[1].value, V::Signed(vec![7]));
            }
            continue;
        }
        let a = s.typed_group_attributes("/").unwrap();
        assert_eq!(a.len(), 1, "{name}");
        match expected {
            "unsigned" => assert_eq!(a[0].value, V::Unsigned(vec![9007199254740993, u64::MAX])),
            "signed" => assert_eq!(a[0].value, V::Signed(vec![-1, i64::MIN])),
            "float" => assert_eq!(a[0].value, V::Float(vec![f64::from(0.1f32), -2.5])),
            "float64" => assert_eq!(a[0].value, V::Float(vec![0.1, -2.5])),
            "utf8" => assert_eq!(a[0].value, V::Strings(vec!["h\u{e9}".into()])),
            "nullpad" => assert_eq!(a[0].value, V::Strings(vec!["a\0b".into()])),
            "spacepad" => assert_eq!(a[0].value, V::Strings(vec!["ab".into()])),
            "text" => assert_eq!(a[0].value, V::Strings(vec!["hello".into()])),
            "unsupported" => assert!(
                matches!(a[0].value, V::Unsupported { .. }),
                "{name}: {:?}",
                a[0]
            ),
            _ => panic!("unknown case {name}"),
        }
    }
}

#[test]
fn revision_metadata_boundaries() {
    use sha2::{Digest, Sha256};
    let Some(base) = evidence_dir("RAS_REVISION_EVIDENCE", "revision_evidence") else {
        return;
    };
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("revision-manifest.json")).unwrap())
            .unwrap();
    for c in cases.as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let path = base.join("revision").join(name);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            c["sha256"].as_str().unwrap(),
            "{name}"
        );
        let kind = c["kind"].as_str().unwrap();
        let result = (|| {
            let s = ReadOnlyHdfSession::open(&path)?;
            if kind.starts_with("compound") {
                let table = s.simple_compound("table", 4096)?;
                if kind == "compound_unsupported" {
                    assert!(
                        matches!(&table.columns[0].value, V::Unsupported { .. }),
                        "{name}: {table:?}"
                    );
                } else {
                    assert_eq!(table.columns.len(), 2, "{name}");
                    assert_eq!(
                        table.columns[0].value,
                        V::Strings(vec!["abcd".into()]),
                        "{name}"
                    );
                    assert_eq!(table.columns[1].value, V::Signed(vec![7]), "{name}");
                }
            } else if kind == "crs_ascii" {
                let attrs = s.typed_group_attributes("/")?;
                let title = attrs.iter().find(|a| a.name == "Title").unwrap();
                assert!(
                    matches!(&title.value,V::Unsupported { reason, .. } if reason.contains("non-ASCII")),
                    "{name}"
                );
                assert_eq!(title.raw_bytes[0], 233, "{name}");
                assert_eq!(
                    s.raw_crs_evidence()?.evidence[0].exact_wkt,
                    "PROJCS[\"x\",UNIT[\"metre\",1]]",
                    "{name}"
                );
            } else {
                let attrs = s.typed_group_attributes("/")?;
                assert_eq!(attrs.len(), 1, "{name}");
                let a = &attrs[0];
                let expected = &c["value"];
                match kind {
                    "signed" => assert_eq!(
                        a.value,
                        V::Signed(
                            expected
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|x| x.as_i64().unwrap())
                                .collect()
                        ),
                        "{name}"
                    ),
                    "unsigned" => assert_eq!(
                        a.value,
                        V::Unsigned(
                            expected
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|x| x.as_u64().unwrap())
                                .collect()
                        ),
                        "{name}"
                    ),
                    "float" => assert_eq!(
                        a.value,
                        V::Float(
                            expected
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|x| x.as_f64().unwrap())
                                .collect()
                        ),
                        "{name}"
                    ),
                    "text" => assert_eq!(
                        a.value,
                        V::Strings(
                            expected
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|x| x.as_str().unwrap().to_owned())
                                .collect()
                        ),
                        "{name}"
                    ),
                    "unsupported" => {
                        assert!(
                            matches!(&a.value,V::Unsupported { reason, .. } if reason.contains(c["reason"].as_str().unwrap())),
                            "{name}: {a:?}"
                        );
                        if let Some(prefix) = c["raw_prefix"].as_array() {
                            assert_eq!(
                                &a.raw_bytes[..prefix.len()],
                                prefix
                                    .iter()
                                    .map(|x| x.as_u64().unwrap() as u8)
                                    .collect::<Vec<_>>(),
                                "{name}"
                            );
                        }
                    }
                    "error" => panic!("{name}: expected error"),
                    _ => panic!("unknown case {name}"),
                }
            }
            Ok::<_, ras_commander_fast_hdf::Error>(())
        })();
        if kind == "error" || kind == "compound_error" {
            let err = result.expect_err(name);
            assert_eq!(
                err.kind,
                if c["error"] == "Unsupported" {
                    ErrorKind::Unsupported
                } else {
                    ErrorKind::Corrupt
                },
                "{name}: {err}"
            );
            assert!(
                err.to_string().contains(c["reason"].as_str().unwrap()),
                "{name}: {err}"
            );
        } else {
            result.unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }
}
