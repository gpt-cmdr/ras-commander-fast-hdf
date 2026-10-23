#[cfg(feature = "catalog")]
use ras_commander_rust::Error;
use ras_commander_rust::ErrorKind;
#[test]
#[cfg(feature = "hdf")]
fn missing_file_is_distinct_and_has_source_path() {
    let p = std::env::temp_dir().join(format!("missing-harden-{}.hdf", std::process::id()));
    let e = ras_commander_rust::hdf::ReadOnlyHdfSession::open(&p).unwrap_err();
    assert_eq!(e.kind, ErrorKind::FileNotFound);
    assert_eq!(e.path.as_deref(), Some(p.as_path()));
}
#[test]
#[cfg(feature = "catalog")]
fn catalog_error_conversion_retains_path() {
    let c =
        ras_commander_rust::catalog::SourceDocument::from_bytes("bad.prj", vec![0]).unwrap_err();
    let e: Error = c.into();
    assert_eq!(e.kind, ErrorKind::Unsupported);
    assert_eq!(e.path.unwrap(), std::path::Path::new("bad.prj"));
}
#[test]
fn bounded_projection_latin1_and_missing_file() {
    let dir = std::env::temp_dir().join(format!("projection-harden-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("model.prj");
    std::fs::write(dir.join("x.prj"), b"PROJCS[\"caf\xe9\"]").unwrap();
    assert_eq!(
        ras_commander_rust::crs::relative_projection(&p, "x.prj")
            .unwrap()
            .exact_wkt,
        "PROJCS[\"caf\u{e9}\"]"
    );
    assert_eq!(
        ras_commander_rust::crs::relative_projection(&p, "absent.prj")
            .unwrap_err()
            .kind,
        ErrorKind::FileNotFound
    );
    std::fs::write(dir.join("x.prj"), vec![b'x'; (1 << 20) + 1]).unwrap();
    assert_eq!(
        ras_commander_rust::crs::relative_projection(&p, "x.prj")
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    std::fs::remove_file(dir.join("x.prj")).unwrap();
    std::fs::remove_dir(dir).unwrap();
}

#[test]
#[cfg(feature = "hdf")]
fn corrupt_attribute_does_not_hide_other_evidence() {
    use ras_commander_rust::{
        crs::EvidenceStatus,
        hdf::{metadata::MetadataValue, ReadOnlyHdfSession},
    };
    let s = ReadOnlyHdfSession::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/attribute-isolation/tolerant.h5"),
    )
    .unwrap();
    let attrs = s.typed_group_attributes("/").unwrap();
    assert!(attrs
        .iter()
        .any(|a| a.name == "bad" && matches!(a.value, MetadataValue::Corrupt { .. })));
    let crs = s.raw_crs_evidence().unwrap();
    assert_eq!(crs.status, EvidenceStatus::Unvalidated);
    assert!(crs.evidence.iter().any(|e| e.exact_wkt == "PROJCS[\"x\"]"));
    assert!(crs
        .hydraulic_units_evidence
        .iter()
        .any(|e| e.source.ends_with("@Units System")
            && matches!(e.status, EvidenceStatus::Unsupported { .. })));
}

#[test]
#[cfg(feature = "hdf")]
fn corrupt_projection_retains_status_and_bytes() {
    let s = ras_commander_rust::hdf::ReadOnlyHdfSession::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/attribute-isolation/corrupt-projection.h5"),
    )
    .unwrap();
    let report = s.raw_crs_evidence().unwrap();
    assert!(matches!(
        report.status,
        ras_commander_rust::crs::EvidenceStatus::Corrupt { .. }
    ));
    assert!(!report.evidence[0].raw_bytes.is_empty());
}

#[test]
fn decoded_projection_limit_and_bom_fallback() {
    let dir = std::env::temp_dir().join(format!("crs-decoded-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("model.prj");
    let target = dir.join("x.prj");
    std::fs::write(&target, vec![255; (1 << 19) + 1]).unwrap();
    assert_eq!(
        ras_commander_rust::crs::relative_projection(&source, "x.prj")
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    std::fs::write(&target, [239, 187, 191, 255]).unwrap();
    assert_eq!(
        ras_commander_rust::crs::relative_projection(&source, "x.prj")
            .unwrap()
            .exact_wkt,
        "ï»¿ÿ"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
