#![cfg(feature = "hdf")]
use ras_commander_fast_hdf::hdf::DEFAULT_MAX_BYTES;
use ras_commander_fast_hdf::{
    hdf::{
        xs1d::{FamilyStatus, PlanResultMode, ResultFamily, XsField, XsVariable},
        NumericValues, ReadOnlyHdfSession, ResultMode, Slice,
    },
    Capability, ErrorKind,
};
use sha2::{Digest, Sha256};
use std::path::Path;
fn hash(p: &Path) -> String {
    format!("{:x}", Sha256::digest(std::fs::read(p).unwrap()))
}
fn bytes(values: &NumericValues) -> Vec<u8> {
    match values {
        NumericValues::F32(v) => v.iter().flat_map(|v| v.to_le_bytes()).collect(),
        NumericValues::F64(v) => v.iter().flat_map(|v| v.to_le_bytes()).collect(),
        _ => panic!("unexpected dtype"),
    }
}
#[test]
#[ignore = "requires frozen real RasExamples geometry/steady sources and h5py oracles"]
fn real_cross_sections_and_steady_profiles() {
    let evidence = std::path::PathBuf::from(std::env::var("RAS_1D_EVIDENCE").unwrap());
    let root = std::path::PathBuf::from(std::env::var("RAS_1D_FIXTURES").unwrap());
    let oracle: serde_json::Value =
        serde_json::from_slice(&std::fs::read(evidence.join("real-oracles.json")).unwrap())
            .unwrap();
    assert_eq!(oracle.as_array().unwrap().len(), 3);
    for case in oracle.as_array().unwrap() {
        let name = case["path"].as_str().unwrap();
        let path = root.join(name);
        let expected_hash = case["sha256"].as_str().unwrap();
        assert_eq!(hash(&path), expected_hash, "{name}");
        for threads in [1, 4] {
            let s = ReadOnlyHdfSession::with_options(&path, threads, true).unwrap();
            let geometry = s.cross_sections(64 * 1024 * 1024).unwrap();
            let rows = case["sections"].as_array().unwrap();
            assert_eq!(geometry.sections.len(), rows.len(), "{name}");
            assert_eq!(geometry.source, path);
            for (index, (xs, row)) in geometry.sections.iter().zip(rows).enumerate() {
                assert_eq!(xs.id.row, index as u64);
                assert_eq!(xs.id.table, "Geometry/Cross Sections/Attributes");
                assert_eq!(xs.id.source, path);
                assert_eq!(xs.river, row["River"].as_str().unwrap());
                assert_eq!(xs.reach, row["Reach"].as_str().unwrap());
                assert_eq!(xs.station, row["RS"].as_str().unwrap());
                assert_eq!(
                    xs.name,
                    XsField::supported(row["Name"].as_str().unwrap().into())
                );
                for (bank, key) in [(&xs.left_bank, "Left Bank"), (&xs.right_bank, "Right Bank")] {
                    let Some(v) = bank.value() else {
                        panic!("{name}: bank unavailable")
                    };
                    if row[key].is_null() {
                        assert!(v.is_nan())
                    } else {
                        assert_eq!(v.to_bits(), row[format!("{key} Bits")].as_u64().unwrap());
                    }
                }
            }
            assert_eq!(
                s.is_steady_plan().unwrap(),
                case["is_steady"].as_bool().unwrap()
            );
            let caps = s.plan_capabilities(DEFAULT_MAX_BYTES).unwrap();
            let mut names: Vec<&str> = caps.families.iter().map(|f| f.family.as_str()).collect();
            names.sort_unstable();
            assert_eq!(
                names,
                [
                    "boundary_conditions",
                    "calibration_regions",
                    "cross_sections_unsteady",
                    "mesh_2d",
                    "pipes",
                    "reference_lines",
                    "reference_points",
                    "steady_profiles",
                    "structures",
                ]
            );
            if case["is_steady"] == true {
                let identities = case["result_identities"].as_array().unwrap();
                assert_eq!(identities.len(), geometry.sections.len());
                for (xs, row) in geometry.sections.iter().zip(identities) {
                    assert_eq!(xs.river.trim(), row["River"].as_str().unwrap().trim());
                    assert_eq!(xs.reach.trim(), row["Reach"].as_str().unwrap().trim());
                    assert_eq!(xs.station.trim(), row["RS"].as_str().unwrap().trim());
                }
                assert_eq!(caps.mode, PlanResultMode::Steady);
                assert!(caps
                    .families
                    .iter()
                    .any(|f| f.family == ResultFamily::SteadyProfiles
                        && f.status == FamilyStatus::Supported));
                let names: Vec<_> = case["profiles"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_str().unwrap().to_string())
                    .collect();
                assert_eq!(s.steady_profile_names(DEFAULT_MAX_BYTES).unwrap(), names);
                for read in case["reads"].as_array().unwrap() {
                    let variable = if read["variable"] == "Flow" {
                        XsVariable::Flow
                    } else {
                        XsVariable::WaterSurface
                    };
                    let a = s
                        .read_cross_section_result(
                            ResultMode::Steady,
                            variable,
                            Slice::new(0, 1, 1),
                            Slice::new(0, 9, 3),
                            DEFAULT_MAX_BYTES,
                            1024 * 1024,
                        )
                        .unwrap();
                    assert_eq!(a.shape, vec![1, 3]);
                    assert!(match read["dtype"].as_str().unwrap() {
                        "<f4" => matches!(a.values, NumericValues::F32(_)),
                        "<f8" => matches!(a.values, NumericValues::F64(_)),
                        _ => false,
                    });
                    assert_eq!(
                        format!("{:x}", Sha256::digest(bytes(&a.values))),
                        read["sha256"].as_str().unwrap(),
                        "{name}"
                    );
                }
            } else {
                assert_eq!(caps.mode, PlanResultMode::Unavailable);
                assert!(caps
                    .families
                    .iter()
                    .all(|f| matches!(f.status, FamilyStatus::Unavailable { .. })));
            }
        }
        assert_eq!(hash(&path), expected_hash, "{name}");
    }
}
#[test]
#[ignore = "requires frozen supplemental HDF schema fixtures"]
fn supplemental_schema_and_selected_read_boundaries() {
    let evidence = std::path::PathBuf::from(std::env::var("RAS_1D_EVIDENCE").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(evidence.join("synthetic-manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest.as_array().unwrap().len(), 8);
    for case in manifest.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let path = evidence.join("synthetic").join(name);
        assert_eq!(hash(&path), case["sha256"].as_str().unwrap());
        let s = ReadOnlyHdfSession::open(&path).unwrap();
        match name {
            "missing_identity.h5" => assert_eq!(
                s.cross_sections(4096).unwrap_err().kind,
                ErrorKind::Unsupported
            ),
            "shape.h5" | "empty_profiles.h5" => assert_eq!(
                s.plan_capabilities(DEFAULT_MAX_BYTES)
                    .unwrap()
                    .families
                    .iter()
                    .find(|f| f.family == ResultFamily::SteadyProfiles)
                    .map(|f| if matches!(f.status, FamilyStatus::Corrupt { .. }) {
                        ErrorKind::Corrupt
                    } else {
                        ErrorKind::Unsupported
                    })
                    .unwrap(),
                ErrorKind::Corrupt,
                "{name}"
            ),
            "dtype.h5" | "vlen_profiles.h5" => {
                assert!(
                    s.plan_capabilities(DEFAULT_MAX_BYTES)
                        .unwrap()
                        .families
                        .iter()
                        .any(|f| f.family == ResultFamily::SteadyProfiles
                            && matches!(f.status, FamilyStatus::Unsupported { .. })),
                    "{name}"
                );
                assert_eq!(
                    s.read_cross_section_result(
                        ResultMode::Steady,
                        XsVariable::WaterSurface,
                        Slice::new(0, 1, 1),
                        Slice::new(0, 1, 1),
                        DEFAULT_MAX_BYTES,
                        1024 * 1024
                    )
                    .unwrap_err()
                    .kind,
                    ErrorKind::Unsupported,
                    "{name}"
                );
            }
            "missing.h5" => {
                assert_eq!(s.plan_result_mode().unwrap(), PlanResultMode::Unavailable);
                assert!(s
                    .plan_capabilities(DEFAULT_MAX_BYTES)
                    .unwrap()
                    .families
                    .iter()
                    .all(|f| matches!(f.status, FamilyStatus::Unavailable { .. })));
            }
            "steady.h5" | "unsteady.h5" => {
                let kind = if name == "steady.h5" {
                    ResultMode::Steady
                } else {
                    ResultMode::Unsteady
                };
                let a = s
                    .read_cross_section_result(
                        kind,
                        XsVariable::WaterSurface,
                        Slice::new(0, 2, 1),
                        Slice::new(0, 2, 1),
                        DEFAULT_MAX_BYTES,
                        1024 * 1024,
                    )
                    .unwrap();
                assert_eq!(a.shape, vec![2, 2]);
                assert_eq!(a.values, NumericValues::F32(vec![1., 2., 3., 4.]));
                let flow = s
                    .read_cross_section_result(
                        kind,
                        XsVariable::Flow,
                        Slice::new(0, 2, 1),
                        Slice::new(0, 2, 1),
                        DEFAULT_MAX_BYTES,
                        1024 * 1024,
                    )
                    .unwrap();
                assert_eq!(flow.values, NumericValues::F64(vec![1., 2., 3., 4.]));
                assert_eq!(
                    s.read_cross_section_result(
                        kind,
                        XsVariable::Flow,
                        Slice::new(0, 99, 1),
                        Slice::new(0, 2, 1),
                        DEFAULT_MAX_BYTES,
                        1024 * 1024
                    )
                    .unwrap_err()
                    .kind,
                    ErrorKind::InvalidRequest
                );
                drop(s);
                assert_eq!(flow.values, NumericValues::F64(vec![1., 2., 3., 4.]));
            }
            _ => panic!("unknown {name}"),
        }
        assert_eq!(hash(&path), case["sha256"].as_str().unwrap());
    }
}

#[test]
#[ignore = "requires source-host computed unsteady HDF and independent frozen oracle"]
fn real_computed_unsteady_cross_sections() {
    let path = std::path::PathBuf::from(std::env::var("RAS_1D_UNSTEADY_FIXTURE").unwrap());
    let oracle: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("RAS_1D_UNSTEADY_ORACLE").unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(hash(&path), oracle["file_sha256"].as_str().unwrap());
    for threads in [1, 4] {
        let s = ReadOnlyHdfSession::with_options(&path, threads, true).unwrap();
        assert_eq!(s.plan_result_mode().unwrap(), PlanResultMode::Unsteady);
        assert!(s
            .plan_capabilities(DEFAULT_MAX_BYTES)
            .unwrap()
            .families
            .iter()
            .any(|f| f.family == ResultFamily::CrossSectionsUnsteady
                && f.status == FamilyStatus::Supported));
        let geometry = s.cross_sections(64 * 1024 * 1024).unwrap();
        assert_eq!(
            geometry.sections.len() as u64,
            oracle["xs_count"].as_u64().unwrap()
        );
        assert_eq!(oracle["identities"].as_array().unwrap().len(), 3);
        for row in oracle["identities"].as_array().unwrap() {
            let xs = &geometry.sections[row["index"].as_u64().unwrap() as usize];
            assert_eq!(xs.river, row["River"].as_str().unwrap());
            assert_eq!(xs.reach, row["Reach"].as_str().unwrap());
            assert_eq!(xs.station, row["RS"].as_str().unwrap());
        }
        assert_eq!(
            oracle["result_identities"].as_array().unwrap().len(),
            geometry.sections.len()
        );
        for row in oracle["result_identities"].as_array().unwrap() {
            let xs = &geometry.sections[row["index"].as_u64().unwrap() as usize];
            assert_eq!(xs.river.trim(), row["River"].as_str().unwrap().trim());
            assert_eq!(xs.reach.trim(), row["Reach"].as_str().unwrap().trim());
            assert_eq!(xs.station.trim(), row["RS"].as_str().unwrap().trim());
        }
        assert_eq!(oracle["reads"].as_array().unwrap().len(), 6);
        for read in oracle["reads"].as_array().unwrap() {
            let slice = |key: &str| {
                let a = read[key].as_array().unwrap();
                Slice::new(
                    a[0].as_u64().unwrap(),
                    a[1].as_u64().unwrap(),
                    a[2].as_u64().unwrap(),
                )
            };
            let variable = read["variable"].as_str().unwrap();
            let rows = slice("rows");
            let sections = slice("sections");
            let capability = s
                .numeric_capability(read["dataset"].as_str().unwrap())
                .unwrap();
            assert_eq!(serde_json::json!(capability.shape), read["shape"]);
            let a = if variable == "Flow Volume Cumulative" {
                s.read_numeric(read["dataset"].as_str().unwrap(), Some(&[rows, sections]))
                    .unwrap()
            } else {
                s.read_cross_section_result(
                    ResultMode::Unsteady,
                    if variable == "Flow" {
                        XsVariable::Flow
                    } else {
                        XsVariable::WaterSurface
                    },
                    rows,
                    sections,
                    DEFAULT_MAX_BYTES,
                    1024 * 1024,
                )
                .unwrap()
            };
            assert_eq!(serde_json::json!(a.shape), read["selected_shape"]);
            assert!(
                match read["dtype"].as_str().unwrap() {
                    "<f4" => matches!(a.values, NumericValues::F32(_)),
                    "<f8" => matches!(a.values, NumericValues::F64(_)),
                    _ => false,
                },
                "{variable}: dtype"
            );
            assert_eq!(
                format!("{:x}", Sha256::digest(bytes(&a.values))),
                read["sha256"].as_str().unwrap(),
                "{variable} threads={threads}"
            );
        }
    }
    assert_eq!(hash(&path), oracle["file_sha256"].as_str().unwrap());
}

#[test]
#[ignore = "requires frozen review boundary fixtures"]
fn review_identity_and_capability_boundaries() {
    let root = std::path::PathBuf::from(std::env::var("RAS_1D_EVIDENCE").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("review-manifest.json")).unwrap()).unwrap();
    for (name, digest) in manifest.as_object().unwrap() {
        let path = root.join("review").join(name);
        assert_eq!(hash(&path), digest.as_str().unwrap());
        let s = ReadOnlyHdfSession::open(&path).unwrap();
        if name.starts_with("optional") {
            assert!(s
                .cross_sections(1)
                .unwrap_err()
                .message
                .contains("byte limit"));
            let g = s.cross_sections(4096).unwrap();
            if name == "optional_absent.h5" {
                assert!(g.sections[0].name.is_unavailable());
                assert!(g.sections[0].left_bank.is_unavailable());
            } else {
                assert!(matches!(
                    g.sections[0].name.capability(),
                    Capability::Unsupported { .. }
                ));
                assert!(matches!(
                    g.sections[0].right_bank.capability(),
                    Capability::Unsupported { .. }
                ));
            }
            continue;
        }
        let cap = s.plan_capabilities(DEFAULT_MAX_BYTES).unwrap();
        if name == "mixed.h5" {
            assert_eq!(cap.mode, PlanResultMode::Mixed);
            let e = s
                .read_cross_section_result(
                    ResultMode::Steady,
                    XsVariable::WaterSurface,
                    Slice::new(0, 2, 1),
                    Slice::new(0, 2, 1),
                    DEFAULT_MAX_BYTES,
                    1,
                )
                .unwrap_err();
            assert!(e.message.contains("max_bytes"));
            continue;
        }
        let f = cap
            .families
            .iter()
            .find(|f| f.family == ResultFamily::SteadyProfiles)
            .unwrap();
        if name == "count_limit.h5" {
            assert!(matches!(f.status, FamilyStatus::Unsupported { .. }));
        } else {
            assert!(matches!(f.status, FamilyStatus::Corrupt { .. }));
        }
        let reason = match &f.status {
            FamilyStatus::Corrupt { reason: r, .. }
            | FamilyStatus::Unsupported { reason: r, .. } => r,
            _ => panic!("{name}: {f:?}"),
        };
        let expected = match name.as_str() {
            "reordered.h5" => "identity mismatch",
            "missing_geometry.h5" => "not found",
            "missing_profiles.h5" => "not found",
            "wrong_object.h5" => "expected dataset",
            "duplicate_profiles.h5" => "duplicate steady",
            "count_limit.h5" => "100000",
            "zero_profiles.h5" => "empty steady",
            _ => panic!("unknown {name}"),
        };
        assert!(reason.to_lowercase().contains(expected), "{name}: {reason}");
    }
}
