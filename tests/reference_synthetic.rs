#![cfg(feature = "hdf")]
//! Synthetic capability fixtures for Reference Lines/Points, Boundary Conditions and
//! Calibration Regions (generator tests/fixtures/reference/make_fixtures.py). No real fixture
//! qualifies these minimal x-only layouts. The real 7.0.1 point reader refuses x as Schema.
use ras_commander_fast_hdf::hdf::xs1d::ResultFamily as F;
use ras_commander_fast_hdf::hdf::ReadOnlyHdfSession;
use ras_commander_fast_hdf::hdf::DEFAULT_MAX_BYTES;
use ras_commander_fast_hdf::{Capability, UnsupportedCode};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reference")
}
fn label(c: &Capability) -> &'static str {
    match c {
        Capability::Unavailable { .. } => "Unavailable",
        Capability::Corrupt { .. } => "Corrupt",
        Capability::Unsupported {
            code: UnsupportedCode::IdentityLayout,
            ..
        } => "Unsupported:IdentityLayout",
        Capability::Unsupported {
            code: UnsupportedCode::Schema,
            ..
        } => "Unsupported:Schema",
        _ => "Other",
    }
}

#[test]
fn unqualified_reference_layouts_report_capabilities_with_isolation() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir().join("manifest.json")).unwrap()).unwrap();
    let mut n = 0;
    for (name, e) in m["fixtures"].as_object().unwrap() {
        let path = dir().join(e["file"].as_str().unwrap());
        let got: String = Sha256::digest(std::fs::read(&path).unwrap())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(got, e["sha256"].as_str().unwrap(), "{name} hash");
        let caps = ReadOnlyHdfSession::open(path)
            .unwrap()
            .plan_capabilities(DEFAULT_MAX_BYTES)
            .unwrap();
        for (fam, key) in [
            (F::ReferenceLines, "rl"),
            (F::ReferencePoints, "rp"),
            (F::BoundaryConditions, "bc"),
            (F::CalibrationRegions, "cr"),
        ] {
            let f = caps.families.iter().find(|f| f.family == fam).unwrap();
            assert_eq!(
                label(&f.status),
                e["expect"][key].as_str().unwrap(),
                "{name} {key}"
            );
            if label(&f.status) == "Unsupported:IdentityLayout" {
                assert!(f
                    .status
                    .reason()
                    .unwrap()
                    .contains("no qualified real layout"));
            }
        }
        n += 1;
    }
    assert_eq!(n, 4);
}
