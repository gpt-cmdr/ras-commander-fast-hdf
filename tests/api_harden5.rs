//! API 5 hardening: shared status vocabulary and code-carrying errors.
use ras_commander_fast_hdf::crs::EvidenceStatus;
use ras_commander_fast_hdf::{Capability, Error, ErrorKind, Reported, UnsupportedCode};

#[test]
fn errors_carry_their_unsupported_code() {
    let e = Error::unsupported(UnsupportedCode::ResourceLimit, "too big");
    assert_eq!(e.kind, ErrorKind::Unsupported);
    assert_eq!(e.code, Some(UnsupportedCode::ResourceLimit));
    assert_eq!(Error::new(ErrorKind::Corrupt, "x").code, None);
    let e =
        Error::new(ErrorKind::InvalidRequest, "budget").with_code(UnsupportedCode::ResourceLimit);
    assert_eq!(e.code, Some(UnsupportedCode::ResourceLimit));
}

#[test]
fn reported_has_exactly_one_consistent_status() {
    let ok = Reported::supported(3u8).with_raw_bytes(vec![3]);
    assert_eq!(ok.value(), Some(&3));
    assert_eq!(ok.raw_bytes(), Some(&[3u8][..]));
    assert_eq!(ok.capability(), &Capability::Supported);
    assert!(ok.is_supported() && !ok.is_unavailable());

    let none = Reported::<u8>::unavailable("absent");
    assert!(none.value().is_none() && none.is_unavailable());
    assert_eq!(none.capability().reason(), Some("absent"));

    let bad =
        Reported::<u8>::unsupported(UnsupportedCode::Encoding, "latin1").with_raw_bytes(vec![0xb0]);
    assert!(bad.value().is_none() && !bad.is_supported());
    assert_eq!(bad.capability().code(), Some(UnsupportedCode::Encoding));
    assert_eq!(bad.raw_bytes(), Some(&[0xb0u8][..]));

    let dmg = Reported::<u8>::corrupt("duplicate");
    assert!(matches!(dmg.capability(), Capability::Corrupt { .. }));
}

#[test]
fn crs_status_exposes_the_shared_vocabulary() {
    assert!(matches!(
        EvidenceStatus::Unknown.capability(),
        Capability::Unavailable { .. }
    ));
    assert!(EvidenceStatus::Unvalidated.capability().is_supported());
    assert!(matches!(
        EvidenceStatus::Invalid.capability(),
        Capability::Corrupt { .. }
    ));
    assert!(matches!(
        EvidenceStatus::Conflict.capability(),
        Capability::Unsupported { .. }
    ));
    let s = EvidenceStatus::unsupported(UnsupportedCode::ResourceLimit, "big");
    assert_eq!(s.capability().code(), Some(UnsupportedCode::ResourceLimit));
    assert!(matches!(
        EvidenceStatus::corrupt("bad").capability(),
        Capability::Corrupt { .. }
    ));
}

#[cfg(feature = "hdf")]
#[test]
fn stable_snake_case_spellings() {
    use ras_commander_fast_hdf::hdf::structures::StructureFamilyKind;
    use ras_commander_fast_hdf::hdf::units::{UnitSystemKind, UnitsFamily};
    use ras_commander_fast_hdf::hdf::xs1d::{PlanResultMode, ResultFamily};
    use ras_commander_fast_hdf::hdf::{NumericKind, ResultMode};
    assert_eq!(ErrorKind::FileNotFound.as_str(), "file_not_found");
    assert_eq!(
        ErrorKind::StructuralTopology.as_str(),
        "structural_topology"
    );
    assert_eq!(UnsupportedCode::IdentityLayout.as_str(), "identity_layout");
    assert_eq!(Capability::Supported.as_str(), "supported");
    assert_eq!(Capability::corrupt("x").as_str(), "corrupt");
    assert_eq!(
        ResultFamily::CrossSectionsUnsteady.as_str(),
        "cross_sections_unsteady"
    );
    assert_eq!(ResultFamily::Mesh2D.as_str(), "mesh_2d");
    assert_eq!(
        StructureFamilyKind::Sa2dAreaConnection.as_str(),
        "sa_2d_area_connection"
    );
    assert_eq!(PlanResultMode::Mixed.as_str(), "mixed");
    assert_eq!(ResultMode::Steady.as_str(), "steady");
    assert_eq!(UnitsFamily::CrossSectionSteady.as_str(), "steady_profiles");
    assert_eq!(
        UnitsFamily::CrossSectionUnsteady.as_str(),
        ResultFamily::CrossSectionsUnsteady.as_str()
    );
    assert_eq!(UnitSystemKind::SiMetric.as_str(), "si_metric");
    assert_eq!(NumericKind::F32.as_str(), "f32");
}

#[cfg(feature = "hdf")]
#[test]
#[allow(deprecated)]
fn deprecated_mode_aliases_are_the_shared_type() {
    use ras_commander_fast_hdf::hdf::{xs1d::XsResultKind, ResultMode};
    use ras_commander_fast_hdf::schema::MeshResultMode;
    assert_eq!(XsResultKind::Steady, ResultMode::Steady);
    assert_eq!(MeshResultMode::Unsteady, ResultMode::Unsteady);
}

#[cfg(feature = "hdf")]
mod budgets {
    use ras_commander_fast_hdf::hdf::structures::StructureFamilyKind;
    use ras_commander_fast_hdf::hdf::{ReadOnlyHdfSession, DEFAULT_MAX_BYTES};
    use ras_commander_fast_hdf::{Capability, ErrorKind, UnsupportedCode};
    use std::path::PathBuf;

    fn fixture(dir: &str, name: &str) -> ReadOnlyHdfSession {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(dir)
            .join(format!("{name}.h5"));
        ReadOnlyHdfSession::open(p).unwrap()
    }

    #[test]
    fn capability_budget_is_the_caller_budget_with_resource_limit_code() {
        let s = fixture("structures", "good");
        let ok = s.plan_capabilities(DEFAULT_MAX_BYTES).unwrap();
        let tiny = s.plan_capabilities(1).unwrap();
        let status = |c: &ras_commander_fast_hdf::hdf::xs1d::PlanCapabilities| {
            c.structures
                .iter()
                .find(|x| x.kind == StructureFamilyKind::LateralStructure)
                .unwrap()
                .status
                .clone()
        };
        assert!(status(&ok).is_supported());
        match status(&tiny) {
            Capability::Unsupported { code, .. } => {
                assert_eq!(code, UnsupportedCode::ResourceLimit)
            }
            other => panic!("{other:?}"),
        }
        // The matching read-side inventory uses the same single meaning of max_bytes.
        let e = s
            .structure_inventory(StructureFamilyKind::LateralStructure, 1)
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidRequest);
        assert_eq!(e.code, Some(UnsupportedCode::ResourceLimit));
    }

    #[test]
    fn mesh_calls_take_a_budget() {
        let s = fixture("deferred", "blocks");
        assert!(s.mesh_inventory(DEFAULT_MAX_BYTES).is_ok());
        let e = s.mesh_inventory(1).unwrap_err();
        assert_eq!(e.code, Some(UnsupportedCode::ResourceLimit));
        let e = s.mesh_name_diagnostics(1).unwrap_err();
        assert_eq!(e.code, Some(UnsupportedCode::ResourceLimit));
    }

    #[test]
    fn unit_attribute_budget_is_reported_not_retained() {
        let s = fixture("time", "units_full");
        let full = s.result_units(DEFAULT_MAX_BYTES).unwrap();
        assert!(full
            .variables
            .iter()
            .all(|v| !v.units.is_unavailable() || v.units.capability().reason().is_some()));
        let tiny = s.result_units(1).unwrap();
        let over: Vec<_> = tiny
            .variables
            .iter()
            .filter(|v| v.units.capability().code() == Some(UnsupportedCode::ResourceLimit))
            .collect();
        assert!(!over.is_empty());
        assert!(over.iter().all(|v| v.units.raw_bytes().is_none()));
    }
}

#[cfg(feature = "hdf")]
#[test]
fn mesh_descriptor_and_structure_identity_share_one_shape() {
    use ras_commander_fast_hdf::hdf::{Identity, ReadOnlyHdfSession, DEFAULT_MAX_BYTES};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let path = root.join("deferred/blocks.h5");
    let s = ReadOnlyHdfSession::open(&path).unwrap();
    let meshes = s.mesh_inventory(DEFAULT_MAX_BYTES).unwrap();
    for (row, m) in meshes.iter().enumerate() {
        assert_eq!(
            m.id,
            Identity::new(&path, "Geometry/2D Flow Areas/Attributes", row as u64)
        );
    }
}

#[cfg(feature = "hdf")]
#[test]
fn dataset_units_never_applies_the_label_rule_below_structures() {
    use ras_commander_fast_hdf::hdf::structures::StructureFamilyKind as K;
    use ras_commander_fast_hdf::hdf::units::UnitsSource;
    use ras_commander_fast_hdf::hdf::{ReadOnlyHdfSession, DEFAULT_MAX_BYTES};
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/structures/good.h5");
    let s = ReadOnlyHdfSession::open(p).unwrap();
    for kind in K::ALL {
        let inv = s.structure_inventory(kind, DEFAULT_MAX_BYTES).unwrap();
        for st in &inv.structures {
            for series in &st.series {
                let d = s.dataset_units(&series.path).unwrap();
                assert!(
                    !matches!(d.source, UnitsSource::QuantityLabel { .. }),
                    "{}",
                    series.path
                );
            }
        }
    }
    // The old inverted names: dataset_units_attribute is the exact Units attribute only.
    let any = s
        .structure_inventory(K::LateralStructure, DEFAULT_MAX_BYTES)
        .unwrap()
        .structures[0]
        .series[0]
        .path
        .clone();
    let attr = s.dataset_units_attribute(&any).unwrap();
    assert_eq!(
        attr.is_supported(),
        s.dataset_units(&any).unwrap().units.is_supported()
    );
}

#[cfg(feature = "hdf")]
#[test]
fn caller_input_errors_are_invalid_request() {
    use ras_commander_fast_hdf::hdf::time::CivilTimestamp;
    assert_eq!(
        CivilTimestamp::new(2021, 2, 29, 0, 0, 0, 0)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        CivilTimestamp::new(2021, 1, 1, 24, 0, 0, 0)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    // Stored-value decoding keeps reporting Corrupt.
    assert_eq!(
        CivilTimestamp::from_ms_since_epoch(i64::MAX)
            .unwrap_err()
            .kind,
        ErrorKind::Corrupt
    );
    let e =
        ras_commander_fast_hdf::crs::relative_projection("C:/proj/a.prj", "../escape.prj").unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
}

#[cfg(feature = "hdf")]
#[test]
fn crs_and_units_share_one_units_system_reader() {
    use ras_commander_fast_hdf::hdf::ReadOnlyHdfSession;
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/time");
    for name in ["units_full", "units_latin1_numeric", "units_missing"] {
        let s = ReadOnlyHdfSession::open(dir.join(format!("{name}.h5"))).unwrap();
        let crs = s.raw_crs_evidence().unwrap();
        let sys = s.unit_system_evidence("/").unwrap();
        assert_eq!(
            crs.hydraulic_units.as_deref(),
            sys.value.value().map(String::as_str),
            "{name}"
        );
        let bad = sys.value.value().is_none() && !sys.value.is_unavailable();
        assert_eq!(
            crs.hydraulic_units_evidence.len(),
            usize::from(bad),
            "{name}"
        );
        for e in &crs.hydraulic_units_evidence {
            assert_eq!(&e.capability(), sys.value.capability(), "{name}");
            assert_eq!(Some(e.raw_bytes.as_slice()), sys.value.raw_bytes());
        }
    }
}

#[cfg(feature = "hdf")]
#[test]
fn time_select_rejects_edited_axes_instead_of_panicking() {
    use ras_commander_fast_hdf::hdf::{time::TimeAxis, ReadOnlyHdfSession, Slice};
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/time/month_names.h5");
    let s = ReadOnlyHdfSession::open(p).unwrap();
    let mut axis: TimeAxis = s.unsteady_time_axis(1 << 20).unwrap();
    assert!(axis.select(Slice::new(0, axis.rows, 1)).is_ok());
    axis.rows += 5;
    let e = axis.select(Slice::new(0, axis.rows, 1)).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Corrupt);
}

#[cfg(feature = "hdf")]
mod xs_time_rows {
    use ras_commander_fast_hdf::hdf::xs1d::{ResultFamily, XsVariable};
    use ras_commander_fast_hdf::hdf::{ReadOnlyHdfSession, ResultMode, Slice, DEFAULT_MAX_BYTES};
    use ras_commander_fast_hdf::{Capability, ErrorKind};

    fn open(name: &str) -> ReadOnlyHdfSession {
        ReadOnlyHdfSession::open(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/xs_time")
                .join(format!("{name}.h5")),
        )
        .unwrap()
    }
    fn read(
        s: &ReadOnlyHdfSession,
    ) -> ras_commander_fast_hdf::Result<ras_commander_fast_hdf::hdf::NumericArray> {
        s.read_cross_section_result(
            ResultMode::Unsteady,
            XsVariable::WaterSurface,
            Slice::new(0, 3, 1),
            Slice::new(0, 2, 1),
            DEFAULT_MAX_BYTES,
            1024,
        )
    }

    #[test]
    fn matching_time_axis_is_supported_for_report_and_read() {
        let s = open("xs_time_ok");
        let caps = s.plan_capabilities(DEFAULT_MAX_BYTES).unwrap();
        let f = caps
            .families
            .iter()
            .find(|f| f.family == ResultFamily::CrossSectionsUnsteady)
            .unwrap();
        assert_eq!(f.status, Capability::Supported, "{:?}", f.status);
        assert_eq!(read(&s).unwrap().shape, vec![3, 2]);
    }

    #[test]
    fn mismatched_time_axis_is_corrupt_for_report_and_read() {
        let s = open("xs_time_mismatch");
        let caps = s.plan_capabilities(DEFAULT_MAX_BYTES).unwrap();
        let f = caps
            .families
            .iter()
            .find(|f| f.family == ResultFamily::CrossSectionsUnsteady)
            .unwrap();
        match &f.status {
            Capability::Corrupt { reason } => {
                assert!(
                    reason.contains("Water Surface") && reason.contains("Time"),
                    "{reason}"
                )
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(read(&s).unwrap_err().kind, ErrorKind::Corrupt);
    }
}

#[cfg(feature = "hdf")]
#[test]
fn plan_capabilities_reports_the_nine_families_by_name() {
    use ras_commander_fast_hdf::hdf::{ReadOnlyHdfSession, DEFAULT_MAX_BYTES};
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xs_time/xs_time_ok.h5");
    let caps = ReadOnlyHdfSession::open(p)
        .unwrap()
        .plan_capabilities(DEFAULT_MAX_BYTES)
        .unwrap();
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
}

#[test]
fn unavailable_reports_never_carry_raw_bytes() {
    let r = Reported::<u8>::unavailable("none").with_raw_bytes(vec![1, 2]);
    assert!(r.raw_bytes().is_none());
    assert!(Reported::<u8>::corrupt("x")
        .with_raw_bytes(vec![1])
        .raw_bytes()
        .is_some());
}

#[cfg(feature = "hdf")]
#[test]
fn unsigned_cell_count_is_accepted_and_overflow_is_corrupt() {
    use ras_commander_fast_hdf::hdf::{ReadOnlyHdfSession, DEFAULT_MAX_BYTES};
    let dir =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mesh_unsigned");
    let ok = ReadOnlyHdfSession::open(dir.join("unsigned.h5")).unwrap();
    let m = ok.mesh_inventory(DEFAULT_MAX_BYTES).unwrap();
    assert_eq!(
        (m[0].physical_cells, m[0].stored_cells, m[0].ghost_cells),
        (2, 3, 1)
    );
    let huge = ReadOnlyHdfSession::open(dir.join("unsigned_huge.h5")).unwrap();
    assert_eq!(
        huge.mesh_inventory(DEFAULT_MAX_BYTES).unwrap_err().kind,
        ErrorKind::Corrupt
    );
}

#[cfg(feature = "hdf")]
#[test]
fn crs_entrypoints_are_core_not_catalog_tagged() {
    let c = ras_commander_fast_hdf::capabilities();
    for name in ["combine", "relative_projection"] {
        let e = c
            .entrypoints
            .iter()
            .find(|e| e.owner == "crs" && e.name == name)
            .unwrap();
        assert_eq!(e.feature, "core");
    }
}
