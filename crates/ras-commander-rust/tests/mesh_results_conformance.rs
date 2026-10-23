#![cfg(feature = "hdf")]
use ras_commander_rust::hdf::{
    mesh_results::MeshResultVariable, NumericValues, ReadOnlyHdfSession, Slice,
};
use ras_commander_rust::{ErrorKind, UnsupportedCode};
use serde_json::Value;
use sha2::{Digest, Sha256};
fn bytes(v: &NumericValues) -> Vec<u8> {
    macro_rules! b {
        ($v:expr) => {
            $v.iter().flat_map(|v| v.to_le_bytes()).collect()
        };
    }
    match v {
        NumericValues::I8(v) => b!(v),
        NumericValues::U8(v) => v.clone(),
        NumericValues::I16(v) => b!(v),
        NumericValues::U16(v) => b!(v),
        NumericValues::I32(v) => b!(v),
        NumericValues::U32(v) => b!(v),
        NumericValues::I64(v) => b!(v),
        NumericValues::U64(v) => b!(v),
        NumericValues::F32(v) => b!(v),
        NumericValues::F64(v) => b!(v),
        _ => panic!("new type"),
    }
}
fn hex(v: &[u8]) -> String {
    v.iter().map(|b| format!("{b:02x}")).collect()
}
fn check_descriptor(d: &MeshResultVariable, v: &Value) {
    assert_eq!(
        serde_json::to_value(&d.shape).unwrap(),
        v["shape"],
        "{}",
        d.path
    );
    assert_eq!(d.domain.as_str(), v["domain"].as_str().unwrap());
    // API 9 oracles record the qualifying rule; older frozen oracles omit it.
    if let Some(rule) = v.get("domain_rule").and_then(|r| r.as_str()) {
        assert_eq!(d.domain_rule.as_str(), rule, "{}", d.path);
    }
}
fn check_literal_refusal(s: &ReadOnlyHdfSession, d: &MeshResultVariable, v: &Value) {
    let width = d.kind.unwrap().as_str()[1..].parse::<usize>().unwrap() / 8;
    let elements = v["elements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_u64().unwrap() as usize)
        .collect::<Vec<_>>();
    for (i, row) in v["rows"].as_array().unwrap().iter().enumerate() {
        let row = row.as_u64().unwrap();
        let selection = if d.shape.len() == 1 {
            None
        } else {
            Some(vec![
                Slice::new(row, row + 1, 1),
                Slice::new(0, d.shape[1], 1),
            ])
        };
        let raw = bytes(
            &s.read_numeric(&d.path, selection.as_deref())
                .unwrap()
                .values,
        );
        let sample = elements
            .iter()
            .flat_map(|&index| raw[index * width..(index + 1) * width].iter().copied())
            .collect::<Vec<_>>();
        assert_eq!(
            hex(&sample),
            v["samples"][i].as_str().unwrap(),
            "literal refused {}",
            d.path
        );
    }
    if let Some(hash) = v["full_sha256"].as_str() {
        assert_eq!(
            hex(&Sha256::digest(bytes(
                &s.read_numeric(&d.path, None).unwrap().values
            ))),
            hash,
            "literal full refused {}",
            d.path
        );
    }
}
#[test]
#[ignore = "RAS_WAVE8_RESULTS_ORACLE points to the frozen h5py mesh-results oracle"]
fn every_real_stored_mesh_variable_matches_h5py() {
    let Ok(path) = std::env::var("RAS_WAVE8_RESULTS_ORACLE") else {
        panic!("RAS_WAVE8_RESULTS_ORACLE required: supply frozen fixture oracle directory");
    };
    let p = std::path::Path::new(&path);
    let p = if p.is_dir() {
        p.join("oracle.json")
    } else {
        p.to_path_buf()
    };
    let oracle: Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
    let mut checked = 0;
    let mut refused = 0;
    for case in oracle["cases"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        assert_eq!(
            hex(&Sha256::digest(std::fs::read(source).unwrap())),
            case["sha256"].as_str().unwrap()
        );
        let s = ReadOnlyHdfSession::with_options(source, 1, false).unwrap();
        for area in case["areas"].as_array().unwrap() {
            let name = area["name"].as_str().unwrap();
            let summary = s.mesh_summary_inventory(name, 64 << 20).unwrap();
            let series = s.mesh_series_inventory(name, 64 << 20).unwrap();
            let bc = s.mesh_boundary_inventory(name, 64 << 20).unwrap();
            assert_eq!(
                summary.variables.len() + series.variables.len() + bc.variables.len(),
                area["variables"].as_array().unwrap().len()
            );
            for v in area["variables"].as_array().unwrap() {
                let var = v["name"].as_str().unwrap();
                let family = v["family"].as_str().unwrap();
                let inv = match family {
                    "Summary Output" => &summary,
                    "Boundary Conditions" => &bc,
                    _ => &series,
                };
                let d = inv.variables.iter().find(|d| d.name == var).unwrap();
                check_descriptor(d, v);
                assert_eq!(serde_json::to_value(&d.chunks).unwrap(), v["chunks"]);
                assert_eq!(
                    serde_json::to_value(d.time_of_extreme_row).unwrap(),
                    v["time_row"]
                );
                assert_eq!(d.kind.unwrap().as_str(), v["kind"].as_str().unwrap());
                if let Some(code) = v["expected_code"].as_str() {
                    let ras_commander_rust::Capability::Unsupported { code: actual, .. } =
                        &d.capability
                    else {
                        panic!("expected refusal {}", d.path)
                    };
                    assert_eq!(actual.as_str(), code, "{}", d.path);
                    eprintln!("REFUSED {} {:?}", d.path, d.capability);
                    check_literal_refusal(&s, d, v);
                    refused += 1;
                    continue;
                }
                assert!(
                    d.capability.is_supported(),
                    "unexpected refusal {} {:?}",
                    d.path,
                    d.capability
                );
                let elems = v["elements"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n.as_u64().unwrap())
                    .collect::<Vec<_>>();
                let read = |rows: Slice, max| {
                    if family == "Summary Output" {
                        s.read_mesh_summary(name, var, Some(&elems), 64 << 20, max)
                    } else if family == "Boundary Conditions" {
                        s.read_mesh_boundary_series(name, var, rows, Some(&elems), 64 << 20, max)
                    } else {
                        s.read_mesh_series(name, var, rows, Some(&elems), 64 << 20, max)
                    }
                };
                if family == "Summary Output" {
                    let data = read(Slice::new(0, 1, 1), 64 << 20).unwrap();
                    let b = bytes(&data.values.values);
                    let row_width =
                        elems.len() * d.kind.unwrap().as_str()[1..].parse::<usize>().unwrap() / 8;
                    for (i, row) in v["rows"].as_array().unwrap().iter().enumerate() {
                        let row = row.as_u64().unwrap() as usize;
                        assert_eq!(
                            hex(&b[row * row_width..(row + 1) * row_width]),
                            v["samples"][i].as_str().unwrap(),
                            "{}",
                            d.path
                        );
                    }
                    let full = s
                        .read_mesh_summary(name, var, None, 64 << 20, 64 << 20)
                        .unwrap();
                    assert_eq!(
                        hex(&Sha256::digest(bytes(&full.values.values))),
                        v["full_sha256"].as_str().unwrap(),
                        "{}",
                        d.path
                    );
                } else {
                    for (i, row) in v["rows"].as_array().unwrap().iter().enumerate() {
                        let row = row.as_u64().unwrap();
                        let data = read(Slice::new(row, row + 1, 1), 64 << 20).unwrap();
                        assert_eq!(
                            hex(&bytes(&data.values.values)),
                            v["samples"][i].as_str().unwrap(),
                            "{}",
                            d.path
                        );
                    }
                    let slab =
                        read(Slice::new(0, v["slab_end"].as_u64().unwrap(), 2), 64 << 20).unwrap();
                    assert_eq!(
                        hex(&bytes(&slab.values.values)),
                        v["slab"].as_str().unwrap()
                    );
                    assert!(read(Slice::new(1, 0, 1), 64 << 20).is_err());
                }
                if !elems.is_empty() {
                    assert_eq!(
                        read(Slice::new(0, 1, 1), 0).unwrap_err().kind,
                        ErrorKind::InvalidRequest
                    );
                    assert_eq!(
                        read(Slice::new(0, 1, 1), 0).unwrap_err().code,
                        Some(UnsupportedCode::ResourceLimit)
                    );
                }
                if let Some(window) = v["window"].as_u64() {
                    let reduced = s
                        .reduce_mesh_series(
                            name,
                            var,
                            Slice::new(0, window, 1),
                            Some(&elems),
                            64 << 20,
                            64 << 20,
                        )
                        .unwrap();
                    assert_eq!(
                        hex(&bytes(&reduced.values.values)),
                        v["extrema"].as_str().unwrap(),
                        "{}",
                        d.path
                    );
                    assert_eq!(
                        serde_json::to_value(reduced.time_indices).unwrap(),
                        v["indices"],
                        "{}",
                        d.path
                    );
                }
                checked += 1;
            }
        }
        let r = s.run_summary(64 << 20).unwrap();
        let groups = r.value().unwrap();
        for a in case["summary_attrs"].as_array().unwrap() {
            let attr = groups
                .iter()
                .find(|g| g.path == a["path"].as_str().unwrap())
                .unwrap()
                .attributes
                .iter()
                .find(|t| t.name == a["name"].as_str().unwrap())
                .unwrap();
            assert_eq!(hex(&attr.raw_bytes), a["raw"].as_str().unwrap());
        }
        assert_eq!(
            s.mesh_summary_inventory("../escape", 64 << 20)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidRequest
        );
    }
    eprintln!("mesh oracle: {checked} variables exact; {refused} explicit refusals");
    assert_eq!(
        checked,
        oracle["expected_counts"]["supported"].as_u64().unwrap()
    );
    assert_eq!(
        refused,
        oracle["expected_counts"]["refused"].as_u64().unwrap()
    );
}
#[test]
#[ignore = "RAS_WAVE8_RESULTS_ORACLE points to the frozen h5py mesh-results oracle"]
fn reduction_ties_nan_and_refusal_boundaries() {
    let Ok(p) = std::env::var("RAS_WAVE8_RESULTS_ORACLE") else {
        panic!("RAS_WAVE8_RESULTS_ORACLE required for edge cases");
    };
    let s = ReadOnlyHdfSession::open(std::path::Path::new(&p).join("edge.hdf")).unwrap();
    let r = s
        .reduce_mesh_series(
            "A",
            "Test",
            Slice::new(0, 3, 1),
            Some(&[2, 0, 1]),
            4096,
            4096,
        )
        .unwrap();
    assert_eq!(r.time_indices, vec![0, 2, 0, 2, 2, 0]);
    let NumericValues::F32(v) = r.values.values else {
        panic!()
    };
    assert_eq!(v[0].to_bits(), 0f32.to_bits());
    assert_eq!(v[3], 4.0);
    for i in [1, 2, 4, 5] {
        assert!(v[i].is_nan());
    }
    assert_eq!(
        s.reduce_mesh_series("A", "Test", Slice::new(0, 0, 1), None, 4096, 4096)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        s.read_mesh_series("A", "Test", Slice::new(0, 1, 1), Some(&[3]), 4096, 4096)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        s.read_mesh_series("A", "BadRows", Slice::new(0, 1, 1), None, 4096, 4096)
            .unwrap_err()
            .kind,
        ErrorKind::Corrupt
    );
    assert_eq!(
        s.read_mesh_summary("A", "Unmatched", None, 4096, 4096)
            .unwrap_err()
            .code,
        Some(UnsupportedCode::IdentityLayout)
    );
    // Output 36 + IDs 24 + one-column slab 12 = exactly 72 bytes.
    s.read_mesh_series("A", "Test", Slice::new(0, 3, 1), Some(&[2, 0, 1]), 4096, 72)
        .unwrap();
    let err = s
        .read_mesh_series("A", "Test", Slice::new(0, 3, 1), Some(&[2, 0, 1]), 4096, 71)
        .unwrap_err();
    assert_eq!(
        (err.kind, err.code),
        (
            ErrorKind::InvalidRequest,
            Some(UnsupportedCode::ResourceLimit)
        )
    );
    assert_eq!(
        s.mesh_series_inventory("Typo", 4096).unwrap_err().kind,
        ErrorKind::NotFound
    );
    let empty = s
        .read_mesh_series("A", "Test", Slice::new(1, 1, 1), Some(&[2]), 4096, 4096)
        .unwrap();
    assert_eq!(empty.values.shape, vec![0, 1]);
}
