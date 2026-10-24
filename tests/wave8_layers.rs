#![cfg(feature = "hdf")]
use ras_commander_fast_hdf::hdf::{
    mesh_geometry::MESH_GEOMETRY_FIELDS, NumericValues, ReadOnlyHdfSession, Slice,
};
use sha2::{Digest, Sha256};

fn bytes(v: &NumericValues) -> Vec<u8> {
    macro_rules! raw {
        ($v:expr) => {
            $v.iter().flat_map(|x| x.to_le_bytes()).collect()
        };
    }
    match v {
        NumericValues::I8(v) => v.iter().map(|&x| x as u8).collect(),
        NumericValues::U8(v) => v.clone(),
        NumericValues::I16(v) => raw!(v),
        NumericValues::U16(v) => raw!(v),
        NumericValues::I32(v) => raw!(v),
        NumericValues::U32(v) => raw!(v),
        NumericValues::I64(v) => raw!(v),
        NumericValues::U64(v) => raw!(v),
        NumericValues::F32(v) => raw!(v),
        NumericValues::F64(v) => raw!(v),
        _ => panic!("unknown numeric kind"),
    }
}
#[test]
#[ignore = "RAS_WAVE8_LAYERS_ORACLE points to read-only h5py frozen manifest"]
fn real_mesh_geometry_matches_h5py() {
    let path = std::env::var("RAS_WAVE8_LAYERS_ORACLE").expect("RAS_WAVE8_LAYERS_ORACLE");
    let oracle: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let limit = 512 * 1024 * 1024;
    for file in oracle["files"].as_array().unwrap() {
        assert!(file.get("error").is_none(), "fixture oracle error: {file}");
        let path = file["path"].as_str().unwrap();
        let s = ReadOnlyHdfSession::open(path).unwrap();
        let meshes = s.mesh_inventory(limit).unwrap();
        for m in &meshes {
            let geometry = s.mesh_geometry(&m.mesh_name, limit).unwrap();
            assert_eq!(geometry.mesh.id, m.id);
            for &field in MESH_GEOMETRY_FIELDS {
                let ds = format!(
                    "Geometry/2D Flow Areas/{}/{}",
                    m.mesh_name,
                    field.dataset_name()
                );
                let expected = file["datasets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|d| d["path"] == ds);
                let desc = geometry.fields.iter().find(|d| d.field == field).unwrap();
                let Some(expected) = expected else {
                    assert!(
                        matches!(
                            desc.capability,
                            ras_commander_fast_hdf::Capability::Unavailable { .. }
                        ),
                        "{path}:{ds}"
                    );
                    continue;
                };
                assert!(
                    desc.capability.is_supported(),
                    "{path}:{ds}: {:?}",
                    desc.capability
                );
                let full = s
                    .read_mesh_geometry(&m.mesh_name, field, None, limit, limit)
                    .unwrap();
                assert_eq!(
                    serde_json::json!(full.shape),
                    expected["shape"],
                    "{path}:{ds}"
                );
                assert_eq!(
                    format!("{:x}", Sha256::digest(bytes(&full.values))),
                    expected["sha256"],
                    "{path}:{ds}"
                );
                for label in ["head", "tail"] {
                    let e = &expected[label];
                    let a = s
                        .read_mesh_geometry(
                            &m.mesh_name,
                            field,
                            Some(Slice::new(
                                e["start"].as_u64().unwrap(),
                                e["end"].as_u64().unwrap(),
                                1,
                            )),
                            limit,
                            limit,
                        )
                        .unwrap();
                    let hex: String = bytes(&a.values)
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect();
                    assert_eq!(hex, e["hex"], "{path}:{ds}:{label}");
                }
                assert!(s
                    .read_mesh_geometry(&m.mesh_name, field, None, limit, 0)
                    .is_err());
                assert!(s
                    .read_mesh_geometry(
                        &m.mesh_name,
                        field,
                        Some(Slice::new(0, desc.shape[0] + 1, 1)),
                        limit,
                        limit
                    )
                    .is_err());
                eprintln!("PASS {path}:{ds}");
            }
        }
        let tables = s.mesh_geometry_tables(limit).unwrap();
        assert_eq!(tables.attributes.table.shape, vec![meshes.len() as u64]);
        let expected = file["datasets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["path"] == "Geometry/2D Flow Areas/Attributes")
            .unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(&tables.attributes.raw_bytes)),
            expected["raw_sha256"]
        );
        assert_eq!(
            tables.cell_info.is_supported(),
            file["datasets"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["path"] == "Geometry/2D Flow Areas/Cell Info")
        );
        if let Some(info) = tables.cell_info.value() {
            let expected = file["datasets"]
                .as_array()
                .unwrap()
                .iter()
                .find(|d| d["path"] == "Geometry/2D Flow Areas/Cell Info")
                .unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(bytes(&info.values))),
                expected["sha256"]
            );
        }
        for mesh in &meshes {
            assert!(s
                .compare_mesh_identity(&s, &mesh.mesh_name, limit, 2 * 1024 * 1024)
                .unwrap()
                .first_mismatch
                .is_none());
        }
        let small = s.layer_inventory(64).unwrap();
        assert!(
            small.iter().any(|l| l.capability.code()
                == Some(ras_commander_fast_hdf::UnsupportedCode::ResourceLimit)),
            "{path}: budget status missing"
        );
        let layers = s.layer_inventory(limit).unwrap();
        assert_eq!(
            layers
                .iter()
                .filter(|l| l.family == "mesh_geometry")
                .count(),
            meshes.len() * MESH_GEOMETRY_FIELDS.len()
        );
        assert!(layers
            .iter()
            .any(|l| l.family == "results" && l.name == "structures"));
        assert_eq!(layers.iter().filter(|l| l.family == "geometry").count(), 6);
        eprintln!(
            "PASS identity and {} layer descriptors: {path}",
            layers.len()
        );
    }
}

#[test]
#[ignore = "supplemental generated cases adjacent to RAS_WAVE8_LAYERS_ORACLE"]
fn identity_first_mismatch_and_malformed_geometry() {
    use ras_commander_fast_hdf::hdf::{
        lines::GeometryFamily, mesh_geometry::MeshGeometryField, mesh_identity::MeshMismatchKind,
    };
    use ras_commander_fast_hdf::{Capability, ErrorKind, UnsupportedCode};
    let root = std::path::PathBuf::from(std::env::var("RAS_WAVE8_LAYERS_ORACLE").unwrap())
        .parent()
        .unwrap()
        .join("cases");
    let base = ReadOnlyHdfSession::open(root.join("base.hdf")).unwrap();
    let max = 1024 * 1024;
    for (name, field, kind, element) in [
        (
            "centers",
            MeshGeometryField::CellCenters,
            MeshMismatchKind::Value,
            Some((1, 1)),
        ),
        (
            "faces",
            MeshGeometryField::FaceCellIndexes,
            MeshMismatchKind::Value,
            Some((0, 1)),
        ),
        (
            "normals",
            MeshGeometryField::FaceNormalsAndLength,
            MeshMismatchKind::Value,
            Some((1, 2)),
        ),
        (
            "shape",
            MeshGeometryField::FaceCellIndexes,
            MeshMismatchKind::Shape,
            None,
        ),
        (
            "kind",
            MeshGeometryField::FaceCellIndexes,
            MeshMismatchKind::NumericKind,
            None,
        ),
    ] {
        let other = ReadOnlyHdfSession::open(root.join(format!("{name}.hdf"))).unwrap();
        let mismatch = base
            .compare_mesh_identity(&other, "A", max, 24)
            .unwrap()
            .first_mismatch
            .unwrap();
        assert_eq!(mismatch.field, field);
        assert_eq!(mismatch.kind, kind);
        assert_eq!(mismatch.element, element);
    }
    assert!(base
        .compare_mesh_identity(&base, "A", max, 24)
        .unwrap()
        .first_mismatch
        .is_none());
    let err = base.compare_mesh_identity(&base, "A", max, 0).unwrap_err();
    assert_eq!(err.code, Some(UnsupportedCode::ResourceLimit));
    for name in ["bad_offsets", "bad_shape", "wrong_object"] {
        let s = ReadOnlyHdfSession::open(root.join(format!("{name}.hdf"))).unwrap();
        assert_eq!(
            s.geometry_family(GeometryFamily::BoundaryConditions, max)
                .unwrap_err()
                .kind,
            ErrorKind::Corrupt
        );
        assert!(matches!(
            s.geometry_family_capability(GeometryFamily::BoundaryConditions, max)
                .unwrap(),
            Capability::Corrupt { .. }
        ));
    }
    let blank = ReadOnlyHdfSession::open(root.join("blank_name.hdf")).unwrap();
    let family = blank
        .geometry_family(GeometryFamily::BoundaryConditions, max)
        .unwrap();
    assert!(matches!(
        family.value().unwrap().features[0].name_capability,
        Capability::Corrupt { .. }
    ));
    let cached = base.mesh_geometry("A", max).unwrap();
    assert_eq!(
        base.read_mesh_geometry_cached(&cached, MeshGeometryField::CellCenters, None, max)
            .unwrap()
            .values,
        NumericValues::F64(vec![1., 2., 3., 4., 5., 6.])
    );
    let b = base
        .read_mesh_geometry(
            "A",
            MeshGeometryField::CellCenters,
            Some(Slice::new(1, 3, 2)),
            max,
            16,
        )
        .unwrap();
    assert_eq!(b.shape, [1, 2]);
    assert_eq!(b.values, NumericValues::F64(vec![3., 4.]));
    let empty = base
        .read_mesh_geometry(
            "A",
            MeshGeometryField::CellCenters,
            Some(Slice::new(3, 3, 1)),
            max,
            0,
        )
        .unwrap();
    assert_eq!(empty.shape, [0, 2]);
    assert!(base
        .read_mesh_geometry(
            "A",
            MeshGeometryField::CellCenters,
            Some(Slice::new(0, 3, 0)),
            max,
            max
        )
        .is_err());
    assert_eq!(
        base.read_mesh_geometry("A", MeshGeometryField::FacePoints, None, max, max)
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
}

#[test]
#[ignore = "RAS_WAVE8_LAYERS_ORACLE points to read-only h5py frozen manifest"]
fn real_line_polygon_families_match_h5py() {
    use ras_commander_fast_hdf::hdf::lines::{GeometryFamily, GEOMETRY_FAMILIES};
    let oracle: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("RAS_WAVE8_LAYERS_ORACLE").unwrap()).unwrap(),
    )
    .unwrap();
    for file in oracle["files"].as_array().unwrap() {
        assert!(file.get("error").is_none(), "fixture oracle error: {file}");
        let path = file["path"].as_str().unwrap();
        let s = ReadOnlyHdfSession::open(path).unwrap();
        for &family in GEOMETRY_FAMILIES {
            let g = s.geometry_family(family, 512 * 1024 * 1024).unwrap();
            let present = file["datasets"].as_array().unwrap().iter().any(|d| {
                d["path"]
                    == format!(
                        "{}/{} Info",
                        family.group_path(),
                        if matches!(
                            family,
                            GeometryFamily::RefinementRegions | GeometryFamily::CalibrationRegions
                        ) {
                            "Polygon"
                        } else {
                            "Polyline"
                        }
                    )
            });
            assert_eq!(
                g.capability().as_str(),
                if present { "supported" } else { "unavailable" },
                "{path}:{}",
                family.as_str()
            );
            let Some(g) = g.value() else {
                eprintln!("STATUS {path}:{} {:?}", family.as_str(), g.capability());
                continue;
            };
            let prefix = if matches!(
                family,
                GeometryFamily::RefinementRegions | GeometryFamily::CalibrationRegions
            ) {
                "Polygon"
            } else {
                "Polyline"
            };
            for (suffix, a) in [
                ("Info", &g.info),
                ("Parts", &g.parts),
                ("Points", &g.points),
            ] {
                let ds = format!("{}/{prefix} {suffix}", family.group_path());
                let e = file["datasets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|d| d["path"] == ds)
                    .unwrap();
                assert_eq!(
                    format!("{:x}", Sha256::digest(bytes(&a.values))),
                    e["sha256"],
                    "{path}:{ds}"
                );
            }
            let ds = format!("{}/Attributes", family.group_path());
            let e = file["datasets"]
                .as_array()
                .unwrap()
                .iter()
                .find(|d| d["path"] == ds)
                .unwrap();
            assert_eq!(g.features.len(), e["shape"][0].as_u64().unwrap() as usize);
            for (i, f) in g.features.iter().enumerate() {
                assert_eq!(f.name, e["rows"][i]["Name"]);
                assert_eq!(f.id.row, i as u64);
                let info = file["datasets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|d| d["path"] == format!("{}/{prefix} Info", family.group_path()))
                    .unwrap();
                assert_eq!(
                    serde_json::json!([f.point_range.start, f.point_range.end]),
                    info["features"][i]["points"]
                );
                assert_eq!(
                    serde_json::json!(f.parts.iter().map(|r| [r.start, r.end]).collect::<Vec<_>>()),
                    info["features"][i]["parts"]
                );
            }
            assert_eq!(
                format!("{:x}", Sha256::digest(&g.attributes.raw_bytes)),
                e["raw_sha256"],
                "{path}:{ds}"
            );
            for name in [
                "External Faces",
                "Internal Faces",
                "Internal Cells",
                "Calibration Table",
            ] {
                assert_eq!(
                    g.tables.iter().any(|(n, _)| n == name),
                    file["datasets"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|d| d["path"] == format!("{}/{name}", family.group_path())),
                    "{path}:{name}"
                );
            }
            for name in ["Terrain Profiles Info", "Terrain Profiles Values"] {
                let ds = format!("{}/{name}", family.group_path());
                let expected = file["datasets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|d| d["path"] == ds);
                assert_eq!(
                    g.terrain_profiles.iter().any(|(n, _)| n == name),
                    expected.is_some(),
                    "{path}:{ds}"
                );
                if let Some(e) = expected {
                    let a = &g
                        .terrain_profiles
                        .iter()
                        .find(|(n, _)| n == name)
                        .unwrap()
                        .1;
                    assert_eq!(
                        format!("{:x}", Sha256::digest(bytes(&a.values))),
                        e["sha256"]
                    );
                }
            }
            for (name, table) in &g.tables {
                let ds = format!("{}/{name}", family.group_path());
                let e = file["datasets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|d| d["path"] == ds)
                    .unwrap();
                assert_eq!(
                    format!("{:x}", Sha256::digest(&table.raw_bytes)),
                    e["raw_sha256"],
                    "{path}:{ds}"
                );
            }
            eprintln!(
                "PASS {path}:{} {} features",
                family.as_str(),
                g.features.len()
            );
        }
    }
}
