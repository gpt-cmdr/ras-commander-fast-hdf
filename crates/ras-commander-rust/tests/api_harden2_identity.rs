#![cfg(feature = "hdf")]
use ras_commander_rust::hdf::DEFAULT_MAX_BYTES;
use ras_commander_rust::hdf::{
    xs1d::{FamilyStatus, ResultFamily, XsVariable},
    ReadOnlyHdfSession, ResultMode, Slice,
};
use sha2::{Digest, Sha256};
#[test]
#[ignore = "requires frozen API harden2 identity fixtures"]
fn packed_identity_and_family_isolation() {
    let root = std::path::PathBuf::from(std::env::var("RAS_HARDEN2_EVIDENCE").unwrap());
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("identity-manifest.json")).unwrap())
            .unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 9);
    for c in cases.as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let p = root.join("identity").join(name);
        assert_eq!(
            format!("{:x}", Sha256::digest(std::fs::read(&p).unwrap())),
            c["sha256"].as_str().unwrap(),
            "{name}"
        );
        let s = ReadOnlyHdfSession::open(&p).unwrap();
        let caps = s.plan_capabilities(DEFAULT_MAX_BYTES).unwrap();
        let expected = c["expected"].as_str().unwrap();
        let family = match expected {
            "mesh_corrupt" => ResultFamily::Mesh2D,
            "structure_corrupt" => ResultFamily::Structures,
            _ => ResultFamily::CrossSectionsUnsteady,
        };
        let cap = caps.families.iter().find(|f| f.family == family).unwrap();
        let reason = match (&cap.status, expected) {
            (FamilyStatus::Supported, "supported") => "",
            (FamilyStatus::Unsupported { reason: r, .. }, "unsupported") => r,
            (
                FamilyStatus::Corrupt { reason: r, .. },
                "corrupt" | "mesh_corrupt" | "structure_corrupt",
            ) => r,
            _ => panic!("{name}: {cap:?}"),
        };
        assert!(
            reason.contains(c["reason"].as_str().unwrap()),
            "{name}: {reason}"
        );
        if expected == "supported" || expected.ends_with("_corrupt") {
            assert!(caps
                .families
                .iter()
                .any(|f| f.family == ResultFamily::CrossSectionsUnsteady
                    && f.status == FamilyStatus::Supported));
            let a = s
                .read_cross_section_result(
                    ResultMode::Unsteady,
                    XsVariable::WaterSurface,
                    Slice::new(0, 1, 1),
                    Slice::new(0, 2, 1),
                    DEFAULT_MAX_BYTES,
                    8,
                )
                .unwrap();
            assert_eq!(a.shape, vec![1, 2]);
        }
    }
}
