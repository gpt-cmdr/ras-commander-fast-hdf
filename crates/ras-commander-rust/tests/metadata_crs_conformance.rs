#![cfg(feature = "hdf")]
use ras_commander_rust::hdf::DEFAULT_MAX_BYTES;
use ras_commander_rust::{
    crs::{combine, relative_projection, CrsEvidence, EvidenceStatus},
    hdf::{metadata::MetadataValue, ReadOnlyHdfSession},
    ErrorKind,
};
#[test]
fn raw_crs_statuses_do_not_infer_semantics() {
    for text in [
        "BOUNDCRS[\"custom\"]",
        "GEODCRS[\"custom\"]",
        "projcs[\"lowercase\"]",
        "unfamiliar balanced text",
    ] {
        let e = CrsEvidence::raw("unknown-syntax", text);
        assert_eq!(e.status, EvidenceStatus::Unvalidated);
        assert_eq!(e.exact_wkt, text);
    }
    for unit in ["Foot_US", "foot", "metre"] {
        let wkt = format!("PROJCS[\"custom Davis\",UNIT[\"{unit}\",1],AXIS[\"X\",EAST]]");
        let e = CrsEvidence::raw("test", wkt.clone());
        assert_eq!(e.status, EvidenceStatus::Unvalidated);
        assert_eq!(e.exact_wkt, wkt);
        assert_eq!(e.unit_clauses.len(), 1);
        assert_eq!(e.axis_clauses.len(), 1);
    }
    assert_eq!(
        CrsEvidence::raw("missing", "").status,
        EvidenceStatus::Unknown
    );
    assert_eq!(
        CrsEvidence::raw("bad", "PROJCS[broken").status,
        EvidenceStatus::Invalid
    );
    assert_eq!(
        combine(
            vec![
                CrsEvidence::raw("a", "PROJCS[\"A\"]"),
                CrsEvidence::raw("b", "PROJCS[\"B\"]")
            ],
            Some("US Customary".into())
        )
        .status,
        EvidenceStatus::Conflict
    );
    // Balanced but semantically invalid text must stay explicitly UNVALIDATED.
    assert_eq!(
        CrsEvidence::raw("not valid CRS", "PROJCS[garbage]").status,
        EvidenceStatus::Unvalidated
    );
}
#[test]
fn project_relative_projection_evidence() {
    let folder = std::env::temp_dir().join(format!("ras-crs-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let project = folder.join("model.prj");
    let text = "PROJCS[\"custom\",UNIT[\"metre\",1]]\n";
    std::fs::write(folder.join("projection.prj"), text).unwrap();
    assert_eq!(
        relative_projection(&project, std::path::Path::new("projection.prj"))
            .unwrap()
            .exact_wkt,
        text
    );
    assert_eq!(
        relative_projection(&project, std::path::Path::new("missing.prj"))
            .unwrap_err()
            .kind,
        ErrorKind::FileNotFound
    );
    assert_eq!(
        relative_projection(&project, std::path::Path::new("../escape.prj"))
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    std::fs::remove_file(folder.join("projection.prj")).unwrap();
    std::fs::remove_dir(folder).unwrap();
}
#[test]
#[ignore = "requires immutable real fixtures and independent h5py oracle"]
fn real_metadata_5_6_7() {
    use sha2::{Digest, Sha256};
    let oracle: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("RAS_METADATA_ORACLE").expect("oracle required")).unwrap(),
    )
    .unwrap();
    let dir =
        std::path::PathBuf::from(std::env::var("RAS_NATIVE_FIXTURES").expect("fixtures required"));
    for f in oracle.as_array().unwrap() {
        let path = dir.join(f["name"].as_str().unwrap());
        let hash = format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap()));
        assert_eq!(hash, f["sha256"].as_str().unwrap());
        let s = ReadOnlyHdfSession::open(&path).unwrap();
        let inventory = s.mesh_inventory(DEFAULT_MAX_BYTES).unwrap();
        assert_eq!(inventory.len(), f["meshes"].as_array().unwrap().len());
        for (mesh, expected) in inventory.iter().zip(f["meshes"].as_array().unwrap()) {
            assert_eq!(mesh.mesh_name, expected["name"].as_str().unwrap());
            assert_eq!(mesh.physical_cells, expected["physical"].as_u64().unwrap());
            assert_eq!(mesh.stored_cells, expected["stored"].as_u64().unwrap());
            assert_eq!(mesh.ghost_cells, mesh.stored_cells - mesh.physical_cells);
            let expected = &expected["variables"];
            if expected.is_null() {
                assert!(matches!(
                    mesh.results.capability,
                    ras_commander_rust::Capability::Unavailable { .. }
                ));
            } else {
                assert!(mesh.results.capability.is_supported());
                let vars = &mesh.results.variables;
                assert_eq!(vars.len(), expected.as_object().unwrap().len());
                for v in vars {
                    assert_eq!(
                        serde_json::json!(v.shape),
                        expected[v.dataset.rsplit('/').next().unwrap()]
                    );
                }
            }
        }
        let crs = s.raw_crs_evidence().unwrap();
        assert_eq!(crs.evidence[0].exact_wkt, f["projection"].as_str().unwrap());
        assert_eq!(crs.hydraulic_units.as_deref(), f["units"].as_str());
        assert_eq!(crs.status, EvidenceStatus::Unvalidated);
        let table = s
            .simple_compound("Geometry/2D Flow Areas/Attributes", 65536)
            .unwrap();
        let names = match &table
            .columns
            .iter()
            .find(|c| c.name == "Name")
            .unwrap()
            .value
        {
            MetadataValue::Strings(v) => v,
            _ => panic!("fixed strings required"),
        };
        let expected: Vec<_> = f["meshes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, &expected);
        assert!(matches!(
            &table
                .columns
                .iter()
                .find(|c| c.name == "Cell Count")
                .unwrap()
                .value,
            MetadataValue::Signed(_)
        ));
        let mann = match &table
            .columns
            .iter()
            .find(|c| c.name == "Mann")
            .unwrap()
            .value
        {
            MetadataValue::Float(v) => v,
            _ => panic!("float proof"),
        };
        assert_eq!(mann.len(), f["meshes"].as_array().unwrap().len());
        for (value, expected) in mann.iter().zip(f["meshes"].as_array().unwrap()) {
            assert_eq!(*value, expected["manning"].as_f64().unwrap());
        }
        assert_eq!(
            s.simple_compound("Geometry/2D Flow Areas/Attributes", 0)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidRequest
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap())),
            hash
        );
    }
}
#[test]
#[ignore = "requires independent Davis/raw CRS oracle and immutable project sources"]
fn real_davis_project_crs() {
    use sha2::{Digest, Sha256};
    use std::path::Path;
    let oracle: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("RAS_CRS_ORACLE").expect("CRS oracle required")).unwrap(),
    )
    .unwrap();
    let path = Path::new(oracle["davis_geometry"].as_str().unwrap());
    let hash = format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()));
    assert_eq!(hash, oracle["davis_sha256"].as_str().unwrap());
    let s = ReadOnlyHdfSession::open(path).unwrap();
    let hdf = s.raw_crs_evidence().unwrap();
    assert_eq!(
        hdf.evidence[0].exact_wkt,
        oracle["davis_wkt"].as_str().unwrap()
    );
    let project = Path::new(oracle["project"].as_str().unwrap());
    let p = relative_projection(project, Path::new("GIS/2871.prj")).unwrap();
    assert_eq!(p.exact_wkt, oracle["project_wkt"].as_str().unwrap());
    assert_eq!(
        combine(vec![hdf.evidence[0].clone(), p.clone()], None).status,
        EvidenceStatus::Unvalidated
    );
    let conflicting = CrsEvidence::raw(
        "other-source",
        "PROJCS[\"metre alternative\",UNIT[\"metre\",1]]",
    );
    assert_eq!(
        combine(vec![p, conflicting], None).status,
        EvidenceStatus::Conflict
    );
    #[cfg(feature = "catalog")]
    {
        let catalog = ras_commander_rust::catalog::ProjectCatalog::open(project).unwrap();
        assert_eq!(
            ras_commander_rust::crs::catalog_projection(&catalog, Path::new("GIS/2871.prj"))
                .unwrap()
                .exact_wkt,
            oracle["project_wkt"].as_str().unwrap()
        );
    }
    for u in oracle["units_supplemental"].as_array().unwrap() {
        let evidence = CrsEvidence::raw("pyproj-supplement", u["wkt"].as_str().unwrap());
        assert_eq!(evidence.status, EvidenceStatus::Unvalidated);
        assert!(!evidence.unit_clauses.is_empty());
        assert!(!evidence.axis_clauses.is_empty());
    }
    assert_eq!(
        format!("{:x}", Sha256::digest(std::fs::read(path).unwrap())),
        hash
    );
}

#[test]
fn lexical_clauses_preserve_nested_ids_and_ignore_quoted_keywords() {
    let wkt = r#"PROJCRS["quoted UNIT[ and ""AXIS["" text",LENGTHUNIT["metre",1,ID["EPSG",9001]],AXIS["X",east,ORDER[1]]]"#;
    let e = CrsEvidence::raw("lexical", wkt);
    assert_eq!(e.status, EvidenceStatus::Unvalidated);
    assert_eq!(
        e.unit_clauses,
        vec![r#"LENGTHUNIT["metre",1,ID["EPSG",9001]]"#]
    );
    assert_eq!(e.axis_clauses, vec![r#"AXIS["X",east,ORDER[1]]"#]);
    let e = CrsEvidence::raw(
        "wkt1",
        r#"PROJCS["x",UNIT["metre",1,AUTHORITY["EPSG","9001"]]]"#,
    );
    assert_eq!(
        e.unit_clauses,
        vec![r#"UNIT["metre",1,AUTHORITY["EPSG","9001"]]"#]
    );
}

#[test]
fn lexical_suffix_lookalikes_are_not_clauses() {
    let e = CrsEvidence::raw(
        "lookalike",
        r#"PROJCS["UNIT[ MYAXIS[",FAKEUNIT["x",1],MYAXIS["y"],unit["metre",1]]"#,
    );
    assert_eq!(e.unit_clauses, vec![r#"unit["metre",1]"#]);
    assert!(e.axis_clauses.is_empty());
}

#[test]
fn bounded_crs_lexical_evidence() {
    let text = "UNIT[".repeat(129) + &"]".repeat(129);
    let e = CrsEvidence::raw("depth", text.clone());
    assert!(
        matches!(e.status,EvidenceStatus::Unsupported { ref reason, .. } if reason.contains("depth"))
    );
    assert_eq!(e.exact_wkt, text);
    assert!(e.unit_clauses.is_empty());
    let text = "x".repeat((1 << 20) + 1);
    let e = CrsEvidence::raw("size", text.clone());
    assert!(
        matches!(e.status,EvidenceStatus::Unsupported { ref reason, .. } if reason.contains("input"))
    );
    assert_eq!(e.exact_wkt, text);
    let e = CrsEvidence::raw("bad", "UNIT[UNIT[");
    assert_eq!(e.status, EvidenceStatus::Invalid);
    assert!(e.unit_clauses.is_empty());
    let e = CrsEvidence::raw(
        "literal",
        r#"PROJCS["C:\dir\",FOO_UNIT["x",1],2UNIT["x",1],UNIT["metre",1]]"#,
    );
    assert_eq!(e.status, EvidenceStatus::Unvalidated);
    assert_eq!(e.unit_clauses, vec![r#"UNIT["metre",1]"#]);
}
