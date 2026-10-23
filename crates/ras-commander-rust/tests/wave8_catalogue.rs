#![cfg(feature = "hdf")]
use ras_commander_rust::{hdf::ReadOnlyHdfSession, Capability};
use serde_json::Value;
use sha2::{Digest, Sha256};
#[test]
#[ignore = "requires RAS_WAVE8_CATALOGUE_ORACLE manifest"]
fn catalogue_and_bc_links_match_frozen_h5py() {
    let j: Value = serde_json::from_slice(
        &std::fs::read(
            std::env::var("RAS_WAVE8_CATALOGUE_ORACLE").expect("catalogue oracle required"),
        )
        .unwrap(),
    )
    .unwrap();
    for c in j["cases"].as_array().unwrap() {
        let path = c["source"].as_str().unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(std::fs::read(path).unwrap())),
            c["sha256"].as_str().unwrap()
        );
        let s = ReadOnlyHdfSession::open(path).unwrap();
        let layers = s.layer_inventory(512 << 20).unwrap();
        for a in c["areas"].as_array().unwrap() {
            let area = a["name"].as_str().unwrap();
            let descriptors = layers
                .iter()
                .filter(|l| l.area.as_deref() == Some(area) && l.mesh_result.is_some())
                .collect::<Vec<_>>();
            assert_eq!(descriptors.len(), a["variables"].as_array().unwrap().len());
            for v in a["variables"].as_array().unwrap() {
                let d = descriptors
                    .iter()
                    .find_map(|l| {
                        l.mesh_result
                            .as_ref()
                            .filter(|d| d.path == v["path"].as_str().unwrap())
                    })
                    .unwrap();
                assert_eq!(serde_json::json!(d.shape), v["shape"]);
                assert_eq!(d.domain.as_str(), v["domain"].as_str().unwrap());
                // API 9 oracles record the qualifying rule; older frozen oracles omit it.
                if let Some(rule) = v.get("domain_rule").and_then(|r| r.as_str()) {
                    assert_eq!(d.domain_rule.as_str(), rule, "{}", d.path);
                }
                if let Some(code) = v["expected_code"].as_str() {
                    assert!(
                        matches!(&d.capability,Capability::Unsupported{code:c,..} if c.as_str()==code)
                    );
                } else {
                    assert!(d.capability.is_supported());
                }
            }
            for inv in [
                s.mesh_summary_inventory(area, 512 << 20).unwrap(),
                s.mesh_series_inventory(area, 512 << 20).unwrap(),
                s.mesh_boundary_inventory(area, 512 << 20).unwrap(),
            ] {
                for d in inv.variables {
                    let actual = descriptors
                        .iter()
                        .find_map(|l| l.mesh_result.as_ref().filter(|v| v.path == d.path))
                        .unwrap();
                    assert_eq!(actual.capability, d.capability);
                    assert_eq!(actual.kind, d.kind);
                    assert_eq!(actual.time_of_extreme_row, d.time_of_extreme_row);
                    // NaN metadata compares by exact raw evidence, not IEEE equality.
                    assert_eq!(actual.attributes.len(), d.attributes.len());
                    for (a, b) in actual.attributes.iter().zip(&d.attributes) {
                        assert_eq!(
                            (&a.name, &a.shape, &a.raw_bytes),
                            (&b.name, &b.shape, &b.raw_bytes)
                        );
                    }
                }
            }
            let links = s.mesh_boundary_links(area, 512 << 20).unwrap();
            let links = links.value().expect("qualified BC links");
            let mut names = links
                .matched
                .iter()
                .map(|m| m.name.clone())
                .collect::<Vec<_>>();
            names.sort();
            assert_eq!(serde_json::json!(names), a["matched"]);
            assert_eq!(
                serde_json::json!(links.unmatched_geometry),
                a["unmatched_geometry"]
            );
            assert_eq!(
                serde_json::json!(links.unmatched_series),
                a["unmatched_series"]
            );
            for m in &links.matched {
                assert!(m
                    .series_path
                    .ends_with(&format!("/Boundary Conditions/{}", m.name)));
                assert_eq!(
                    m.geometry.table,
                    "Geometry/Boundary Condition Lines/Attributes"
                );
            }
        }
        let run = s.run_summary(512 << 20).unwrap();
        for g in run.value().unwrap() {
            let actual = layers
                .iter()
                .find_map(|l| l.run_summary.as_ref().filter(|v| v.path == g.path))
                .unwrap();
            assert_eq!(actual.attributes.len(), g.attributes.len());
            for (a, b) in actual.attributes.iter().zip(&g.attributes) {
                assert_eq!(
                    (&a.name, &a.shape, &a.raw_bytes),
                    (&b.name, &b.shape, &b.raw_bytes)
                );
            }
        }
        // Resource limits stay isolated; the catalogue is still a reviewable report.
        assert!(s.layer_inventory(0).is_ok());
    }
}
