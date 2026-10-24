#![cfg(feature = "hdf")]
//! 1D (API 7) real-file oracle: 1D geometry, 1D results, 1D summary output and 1D
//! structures compared bit-exactly with a frozen h5py manifest
//! (`tests/fixtures/wave2_1d/oracle_wave2_1d.py`). Gate: `RAS_WAVE2_1D_ORACLE` = manifest path.
use ras_commander_fast_hdf::hdf::xs_geometry::{
    OneDLineFamily, XsGeometryField, XsGeometryValues, ONE_D_LINE_FAMILIES, XS_GEOMETRY_FIELDS,
};
use ras_commander_fast_hdf::hdf::{NumericValues, ReadOnlyHdfSession};
use ras_commander_fast_hdf::{Capability, ErrorKind, Reported};
use serde_json::Value;
use sha2::{Digest, Sha256};

const MB: usize = 1 << 20;

fn manifest() -> Value {
    let path = std::env::var("RAS_WAVE2_1D_ORACLE").unwrap_or_else(|_| {
        panic!("RAS_WAVE2_1D_ORACLE must name the frozen h5py manifest (oracle_wave2_1d.py output)")
    });
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("RAS_WAVE2_1D_ORACLE={path} is set but unreadable: {e}"));
    serde_json::from_slice(&bytes).expect("oracle manifest JSON")
}
pub fn le(v: &NumericValues) -> Vec<u8> {
    match v {
        NumericValues::I8(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        NumericValues::I16(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        NumericValues::I32(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        NumericValues::I64(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        NumericValues::U8(v) => v.clone(),
        NumericValues::U16(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        NumericValues::U32(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        NumericValues::U64(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        NumericValues::F32(v) => v.iter().flat_map(|x| x.to_bits().to_le_bytes()).collect(),
        NumericValues::F64(v) => v.iter().flat_map(|x| x.to_bits().to_le_bytes()).collect(),
        _ => panic!("unexpected numeric kind"),
    }
}
pub fn sha(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn file_sha(p: &str) -> String {
    use std::io::Read;
    let mut f = std::fs::File::open(p).unwrap();
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 22];
    loop {
        let n = f.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    format!("{:x}", h.finalize())
}
fn trim(s: &str) -> &str {
    s.trim_end_matches(['\0', ' '])
}
fn labels_json(r: &Reported<Vec<ras_commander_fast_hdf::hdf::structures::ColumnLabel>>) -> Value {
    match r.capability() {
        Capability::Supported => Value::Array(
            r.value()
                .unwrap()
                .iter()
                .map(|l| serde_json::json!([trim(&l.name), l.unit.as_deref().map(trim)]))
                .collect(),
        ),
        Capability::Unavailable { .. } => Value::Null,
        Capability::Corrupt { .. } => Value::String("corrupt".into()),
        c => panic!("unexpected label status {c:?}"),
    }
}
fn cases(j: &Value) -> Vec<(Value, ReadOnlyHdfSession)> {
    j["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            // Relocatable: fixtures are relative to RAS_WAVE2_1D_FIXTURE_ROOT (or the manifest
            // `base`); the SHA256 proves the same bytes are read on any host.
            let root = std::env::var("RAS_WAVE2_1D_FIXTURE_ROOT")
                .unwrap_or_else(|_| j["base"].as_str().expect("manifest base").to_string());
            let p = std::path::Path::new(&root).join(c["path"].as_str().unwrap());
            assert!(p.is_file(), "fixture {} missing below {root}", p.display());
            let p = p.to_str().unwrap().to_string();
            assert_eq!(file_sha(&p), c["sha256"].as_str().unwrap(), "{p}");
            (c.clone(), ReadOnlyHdfSession::open(&p).unwrap())
        })
        .collect()
}

#[test]
#[ignore = "requires RAS_WAVE2_1D_ORACLE (frozen h5py manifest of real HEC-RAS files)"]
fn wave2_xs_geometry_matches_h5py() {
    let j = manifest();
    for (c, s) in cases(&j) {
        let name = c["name"].as_str().unwrap();
        let x = &c["xs_geometry"];
        match x["status"].as_str().unwrap() {
            "absent" => {
                let e = s.xs_geometry_inventory(64 * MB).unwrap_err();
                assert_eq!(e.kind, ErrorKind::NotFound, "{name}");
                continue;
            }
            "unsupported" => {
                let e = s.xs_geometry_inventory(64 * MB).unwrap_err();
                assert_eq!(e.kind, ErrorKind::Unsupported, "{name}");
                continue;
            }
            _ => {}
        }
        let inv = s.xs_geometry_inventory(64 * MB).unwrap();
        assert_eq!(
            inv.geometry.sections.len() as u64,
            x["sections"].as_u64().unwrap()
        );
        assert_eq!(
            sha(&inv.attributes.raw_bytes),
            x["attributes_sha"].as_str().unwrap(),
            "{name}"
        );
        for (xs, id) in inv
            .geometry
            .sections
            .iter()
            .zip(x["identities"].as_array().unwrap())
        {
            assert_eq!(
                [trim(&xs.river), trim(&xs.reach), trim(&xs.station)],
                [
                    id[0].as_str().unwrap(),
                    id[1].as_str().unwrap(),
                    id[2].as_str().unwrap()
                ]
            );
        }
        let unq: Vec<&str> = inv.unqualified.iter().map(|u| u.0.as_str()).collect();
        let want: Vec<&str> = x["unqualified"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(unq, want, "{name}");
        assert!(inv.unqualified.iter().all(|u| u.1.code().is_some()));
        for &field in XS_GEOMETRY_FIELDS {
            let o = &x["fields"][field.as_str()];
            let d = inv.fields.iter().find(|d| d.field == field).unwrap();
            if o["status"] == "unavailable" {
                assert!(
                    matches!(d.capability, Capability::Unavailable { .. }),
                    "{name} {field:?}"
                );
                let e = s
                    .read_xs_geometry(field, None, 64 * MB, 64 * MB)
                    .unwrap_err();
                assert_eq!(e.kind, ErrorKind::NotFound);
                continue;
            }
            assert!(
                d.capability.is_supported(),
                "{name} {field:?}: {:?}",
                d.capability
            );
            assert_eq!(serde_json::json!(d.info_shape), o["info_shape"]);
            let one_to_one = field != XsGeometryField::PropertyCells
                && o["info_shape"][0].as_u64() == x["sections"].as_u64();
            assert_eq!(d.records_are_sections, one_to_one, "{name} {field:?}");
            assert_eq!(
                d.section_records,
                o["section_records"].as_u64().unwrap(),
                "{name} {field:?}"
            );
            assert_eq!(serde_json::json!(d.values_shape), o["values_shape"]);
            if o["values_dtype"] != "compound" {
                assert_eq!(labels_json(&d.labels), o["labels"], "{name} {field:?}");
            }
            let all = s.read_xs_geometry(field, None, 64 * MB, 256 * MB).unwrap();
            assert_eq!(sha(&le(&all.info.values)), o["info_sha"].as_str().unwrap());
            let packed = match &all.values {
                XsGeometryValues::Numeric(a) => {
                    assert_eq!(a.shape[0], o["packed_rows"].as_u64().unwrap());
                    le(&a.values)
                }
                XsGeometryValues::Compound(c) => c.raw_bytes.clone(),
                _ => unreachable!(),
            };
            assert_eq!(
                sha(&packed),
                o["packed_sha"].as_str().unwrap(),
                "{name} {field:?}"
            );
            let sample: Vec<u64> = o["sample"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap())
                .collect();
            let sel = s
                .read_xs_geometry(field, Some(&sample), 64 * MB, 64 * MB)
                .unwrap();
            let bytes = match &sel.values {
                XsGeometryValues::Numeric(a) => le(&a.values),
                XsGeometryValues::Compound(c) => c.raw_bytes.clone(),
                _ => unreachable!(),
            };
            assert_eq!(
                sha(&bytes),
                o["sample_sha"].as_str().unwrap(),
                "{name} {field:?}"
            );
            for r in &sel.records {
                let table = if r.id.row < d.section_records {
                    "Geometry/Cross Sections/Attributes"
                } else {
                    d.identity_table.as_str()
                };
                assert_eq!(r.id.table, table, "{name} {field:?}");
            }
            for (r, want) in sel
                .records
                .iter()
                .zip(o["sample_ranges"].as_array().unwrap())
            {
                assert_eq!(
                    [r.stored_rows.start, r.stored_rows.end],
                    [want[0].as_u64().unwrap(), want[1].as_u64().unwrap()]
                );
                assert_eq!(r.id.source, s.path());
            }
            // Budgets are explicit: a zero budget is a resource-limit request error.
            let e = s
                .read_xs_geometry(field, Some(&sample), 64 * MB, 0)
                .unwrap_err();
            assert_eq!(e.kind, ErrorKind::InvalidRequest);
        }
        let n = inv.geometry.sections.len() as u64;
        let e = s
            .read_xs_geometry(
                XsGeometryField::StationElevation,
                Some(&[n]),
                64 * MB,
                64 * MB,
            )
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidRequest);
    }
}

#[test]
#[ignore = "requires RAS_WAVE2_1D_ORACLE (frozen h5py manifest of real HEC-RAS files)"]
fn wave2_lines_reaches_junctions_surfaces_match_h5py() {
    let j = manifest();
    for (c, s) in cases(&j) {
        let name = c["name"].as_str().unwrap();
        for &family in ONE_D_LINE_FAMILIES {
            let o = &c["lines"][family.as_str()];
            let r = s.one_d_line_family(family, 256 * MB);
            if o["status"] == "unavailable" {
                assert!(r.unwrap().is_unavailable(), "{name} {family:?}");
                continue;
            }
            if family == OneDLineFamily::CrossSectionCutLines
                && c["xs_geometry"]["status"] == "absent"
            {
                // Cut lines need the identity table; without it they are unsupported.
                assert_eq!(r.unwrap_err().kind, ErrorKind::Unsupported);
                continue;
            }
            let r = r.unwrap();
            let d = r
                .value()
                .unwrap_or_else(|| panic!("{name} {family:?} {:?}", r.capability()));
            assert_eq!(d.features.len() as u64, o["features"].as_u64().unwrap());
            assert_eq!(sha(&le(&d.info.values)), o["info_sha"].as_str().unwrap());
            assert_eq!(sha(&le(&d.parts.values)), o["parts_sha"].as_str().unwrap());
            assert_eq!(
                sha(&le(&d.points.values)),
                o["points_sha"].as_str().unwrap()
            );
            assert_eq!(
                d.attributes.is_some(),
                o["has_attributes"].as_bool().unwrap()
            );
            if let Some(a) = &d.attributes {
                assert_eq!(sha(&a.raw_bytes), o["attributes_sha"].as_str().unwrap());
            }
            let ranges: Vec<Value> = d
                .features
                .iter()
                .map(|f| {
                    serde_json::json!([
                        f.point_range.start,
                        f.point_range.end,
                        f.parts.iter().map(|p| [p.start, p.end]).collect::<Vec<_>>()
                    ])
                })
                .collect();
            let text = serde_json::to_string(&ranges).unwrap().replace(',', ", ");
            assert_eq!(
                sha(text.as_bytes()),
                o["ranges_sha"].as_str().unwrap(),
                "{name} {family:?}"
            );
        }
        let reaches = s.river_reaches(64 * MB).unwrap();
        match c["reaches"].as_array() {
            None => assert!(reaches.is_unavailable()),
            Some(want) => {
                let got = reaches.value().unwrap();
                assert_eq!(got.len(), want.len());
                for (g, w) in got.iter().zip(want) {
                    let opt = |r: &Reported<String>| r.value().map(|v| trim(v).to_string());
                    assert_eq!(
                        serde_json::json!([
                            trim(&g.river),
                            trim(&g.reach),
                            opt(&g.upstream_type),
                            opt(&g.upstream_name),
                            opt(&g.downstream_type),
                            opt(&g.downstream_name)
                        ]),
                        *w
                    );
                }
            }
        }
        let jn = s.river_junctions(64 * MB).unwrap();
        match c["junctions"].as_object() {
            None => assert!(jn.is_unavailable()),
            Some(w) => {
                let g = jn.value().unwrap();
                assert_eq!(g.features.len() as u64, w["rows"].as_u64().unwrap());
                assert_eq!(
                    sha(&g.attributes.raw_bytes),
                    w["attributes_sha"].as_str().unwrap()
                );
                assert_eq!(
                    sha(&le(&g.points.values)),
                    w["points_sha"].as_str().unwrap()
                );
            }
        }
        let sf = s.xs_interpolation_surfaces(256 * MB).unwrap();
        match c["surfaces"].as_object() {
            None => assert!(sf.is_unavailable()),
            Some(w) => {
                let g = sf.value().unwrap();
                assert_eq!(g.surfaces.len() as u64, w["surfaces"].as_u64().unwrap());
                assert_eq!(sha(&le(&g.info.values)), w["info_sha"].as_str().unwrap());
                assert_eq!(
                    sha(&le(&g.points.values)),
                    w["points_sha"].as_str().unwrap()
                );
                assert_eq!(
                    sha(&le(&g.triangles.values)),
                    w["triangles_sha"].as_str().unwrap()
                );
                let ps = w["per_surface"].as_object().unwrap();
                assert_eq!(g.per_surface.len(), ps.len());
                for (n, a) in &g.per_surface {
                    assert_eq!(sha(&le(&a.values)), ps[n].as_str().unwrap(), "{name} {n}");
                }
            }
        }
    }
}

fn mode_of(m: &str) -> ras_commander_fast_hdf::hdf::ResultMode {
    match m {
        "steady" => ras_commander_fast_hdf::hdf::ResultMode::Steady,
        _ => ras_commander_fast_hdf::hdf::ResultMode::Unsteady,
    }
}

#[test]
#[ignore = "requires RAS_WAVE2_1D_ORACLE (frozen h5py manifest of real HEC-RAS files)"]
fn wave2_xs_results_match_h5py() {
    use ras_commander_fast_hdf::hdf::Slice;
    let j = manifest();
    for (c, s) in cases(&j) {
        let name = c["name"].as_str().unwrap();
        for (m, o) in c["xs_results"].as_object().unwrap() {
            let mode = mode_of(m);
            let inv = s.xs_result_inventory(mode, 64 * MB).unwrap();
            match o["status"].as_str().unwrap() {
                "unavailable" => {
                    assert!(
                        matches!(inv.capability, Capability::Unavailable { .. }),
                        "{name} {m}"
                    );
                    assert!(inv.variables.is_empty());
                    continue;
                }
                "identity_failed" => {
                    assert!(!inv.capability.is_supported(), "{name} {m}");
                    continue;
                }
                _ => assert!(
                    inv.capability.is_supported(),
                    "{name} {m}: {:?}",
                    inv.capability
                ),
            }
            assert_eq!(inv.rows, o["rows"].as_u64().unwrap());
            let want = o["variables"].as_object().unwrap();
            let got: Vec<&str> = inv.variables.iter().map(|v| v.name.as_str()).collect();
            let mut keys: Vec<&str> = want.keys().map(|k| k.as_str()).collect();
            keys.sort();
            assert_eq!(got, keys, "{name} {m}");
            for v in &inv.variables {
                let w = &want[&v.name];
                assert_eq!(serde_json::json!(v.shape), w["shape"]);
                assert_eq!(
                    v.domain.as_str(),
                    w["domain"].as_str().unwrap(),
                    "{name} {}",
                    v.name
                );
                let mut attrs: Vec<&str> = v.attributes.iter().map(|a| a.name.as_str()).collect();
                attrs.sort();
                assert_eq!(serde_json::json!(attrs), w["attrs"]);
                assert_eq!(
                    v.capability.is_supported(),
                    w["readable"].as_bool().unwrap(),
                    "{name} {} {:?}",
                    v.name,
                    v.capability
                );
                if !v.capability.is_supported() {
                    continue;
                }
                let rows = inv.rows;
                let all = s
                    .read_xs_result(
                        mode,
                        &v.name,
                        Slice::new(0, rows, 1),
                        None,
                        64 * MB,
                        512 * MB,
                    )
                    .unwrap();
                assert_eq!(
                    sha(&le(&all.values.values)),
                    w["sha"].as_str().unwrap(),
                    "{name} {}",
                    v.name
                );
                assert_eq!(all.section_ids.len(), inv.sections.len());
                let sr = &w["sample_rows"];
                let cols: Vec<u64> = w["sample_cols"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_u64().unwrap())
                    .collect();
                let rs = Slice::new(
                    sr[0].as_u64().unwrap(),
                    sr[1].as_u64().unwrap(),
                    sr[2].as_u64().unwrap(),
                );
                // A large budget takes the covering-slab path; a budget of exactly the returned
                // bytes forces the per-column path; one byte less is a resource-limit refusal.
                let nr = (rs.end - rs.start).div_ceil(rs.step);
                let inner = if v.shape.len() == 3 { v.shape[1] } else { 1 };
                let width = match v.kind {
                    Some(ras_commander_fast_hdf::hdf::NumericKind::F64) => 8,
                    _ => 4,
                };
                let returned =
                    (nr * inner * cols.len() as u64 * width + cols.len() as u64 * 8) as usize;
                let e = s
                    .read_xs_result(mode, &v.name, rs, Some(&cols), 64 * MB, returned - 1)
                    .unwrap_err();
                assert_eq!(e.kind, ErrorKind::InvalidRequest);
                assert_eq!(
                    e.code,
                    Some(ras_commander_fast_hdf::UnsupportedCode::ResourceLimit)
                );
                for budget in [512 * MB, returned] {
                    let sel = s
                        .read_xs_result(mode, &v.name, rs, Some(&cols), 64 * MB, budget)
                        .unwrap();
                    assert_eq!(
                        sha(&le(&sel.values.values)),
                        w["sample_sha"].as_str().unwrap(),
                        "{name} {} budget {budget}",
                        v.name
                    );
                    assert_eq!(sel.section_ids[0], inv.sections[cols[0] as usize]);
                }
                let ct = &w["contig"];
                let k = ct[2].as_u64().unwrap();
                let contig: Vec<u64> = (0..k).collect();
                let sel = s
                    .read_xs_result(
                        mode,
                        &v.name,
                        Slice::new(ct[0].as_u64().unwrap(), ct[1].as_u64().unwrap(), 1),
                        Some(&contig),
                        64 * MB,
                        64 * MB,
                    )
                    .unwrap();
                assert_eq!(
                    sha(&le(&sel.values.values)),
                    w["contig_sha"].as_str().unwrap()
                );
                let e = s
                    .read_xs_result(
                        mode,
                        &v.name,
                        Slice::new(0, rows + 1, 1),
                        None,
                        64 * MB,
                        64 * MB,
                    )
                    .unwrap_err();
                assert_eq!(e.kind, ErrorKind::InvalidRequest);
                let e = s
                    .read_xs_result(mode, &v.name, Slice::new(0, rows, 1), None, 64 * MB, 0)
                    .unwrap_err();
                assert_eq!(e.kind, ErrorKind::InvalidRequest);
            }
            let e = s
                .read_xs_result(
                    mode,
                    "No Such Variable",
                    Slice::new(0, 1, 1),
                    None,
                    64 * MB,
                    64 * MB,
                )
                .unwrap_err();
            assert_eq!(e.kind, ErrorKind::NotFound);
            if let Some(w) = o["window"].as_array() {
                let win = s
                    .unsteady_row_window(w[0].as_i64().unwrap(), w[1].as_i64().unwrap(), 64 * MB)
                    .unwrap();
                assert_eq!(
                    [win.start, win.end],
                    [w[2].as_u64().unwrap(), w[3].as_u64().unwrap()],
                    "{name}"
                );
            }
        }
    }
}

#[test]
#[ignore = "requires RAS_WAVE2_1D_ORACLE (frozen h5py manifest of real HEC-RAS files)"]
fn wave2_one_d_summary_matches_h5py() {
    let j = manifest();
    for (c, s) in cases(&j) {
        let name = c["name"].as_str().unwrap();
        let inv = s.one_d_summary_inventory(64 * MB).unwrap();
        let Some(want) = c["summary"].as_array() else {
            assert!(
                matches!(inv.capability, Capability::Unavailable { .. }),
                "{name}"
            );
            continue;
        };
        assert_eq!(inv.variables.len(), want.len(), "{name}");
        for (v, w) in inv.variables.iter().zip(want) {
            assert_eq!(
                (v.family.as_str(), v.name.as_str()),
                (w["family"].as_str().unwrap(), w["name"].as_str().unwrap())
            );
            assert_eq!(serde_json::json!(v.shape), w["shape"]);
            assert_eq!(
                serde_json::json!(v.element_axis),
                w["axis"],
                "{name} {}",
                v.name
            );
            let rl = v.row_labels.as_ref().map(|l| {
                l.iter()
                    .map(|x| serde_json::json!([trim(&x.name), x.unit.as_deref().map(trim)]))
                    .collect::<Vec<_>>()
            });
            assert_eq!(serde_json::json!(rl), w["row_labels"], "{name} {}", v.name);
            assert_eq!(serde_json::json!(v.time_of_extreme_rows), w["time_rows"]);
            assert!(
                v.capability.is_supported(),
                "{name} {} {:?}",
                v.name,
                v.capability
            );
            let all = s
                .read_one_d_summary(&v.family, &v.name, None, 64 * MB, 64 * MB)
                .unwrap();
            assert_eq!(
                sha(&le(&all.values.values)),
                w["sha"].as_str().unwrap(),
                "{name} {}",
                v.name
            );
            if let Some(sample) = w["sample"].as_array() {
                let sel: Vec<u64> = sample.iter().map(|x| x.as_u64().unwrap()).collect();
                let r = s
                    .read_one_d_summary(&v.family, &v.name, Some(&sel), 64 * MB, 64 * MB)
                    .unwrap();
                assert_eq!(
                    sha(&le(&r.values.values)),
                    w["sample_sha"].as_str().unwrap(),
                    "{name} {}",
                    v.name
                );
                assert_eq!(r.element_ids.is_some(), v.family == "Cross Sections");
                let n = v.shape[v.element_axis.unwrap()];
                let e = s
                    .read_one_d_summary(&v.family, &v.name, Some(&[n]), 64 * MB, 64 * MB)
                    .unwrap_err();
                assert_eq!(e.kind, ErrorKind::InvalidRequest);
            }
        }
    }
}

#[test]
#[ignore = "requires RAS_WAVE2_1D_ORACLE (frozen h5py manifest of real HEC-RAS files)"]
fn wave2_one_d_structures_match_h5py() {
    use ras_commander_fast_hdf::hdf::structures_1d::{OneDGeometryGroup, ONE_D_STRUCTURE_KINDS};
    use ras_commander_fast_hdf::hdf::Slice;
    let j = manifest();
    for (c, s) in cases(&j) {
        let name = c["name"].as_str().unwrap();
        let caps = s.one_d_structure_capabilities(64 * MB).unwrap();
        for (key, o) in c["structure_results"].as_object().unwrap() {
            let (k, m) = key.split_once('/').unwrap();
            let kind = *ONE_D_STRUCTURE_KINDS
                .iter()
                .find(|x| x.as_str() == k)
                .unwrap();
            let mode = mode_of(m);
            let cap = caps
                .iter()
                .find(|x| x.kind == kind && x.mode == mode)
                .unwrap();
            match o["status"].as_str().unwrap() {
                "unavailable" => {
                    assert!(
                        matches!(cap.status, Capability::Unavailable { .. }),
                        "{name} {key}"
                    );
                    let e = s
                        .one_d_structure_inventory(kind, mode, 64 * MB)
                        .unwrap_err();
                    assert_eq!(e.kind, ErrorKind::NotFound);
                    continue;
                }
                "unqualified" => {
                    assert_eq!(
                        cap.status.code(),
                        Some(ras_commander_fast_hdf::UnsupportedCode::IdentityLayout),
                        "{name} {key}"
                    );
                    continue;
                }
                "identity" => {
                    assert_eq!(
                        cap.status.code(),
                        Some(ras_commander_fast_hdf::UnsupportedCode::IdentityLayout)
                    );
                    continue;
                }
                _ => assert!(cap.status.is_supported(), "{name} {key}: {:?}", cap.status),
            }
            let inv = s.one_d_structure_inventory(kind, mode, 64 * MB).unwrap();
            assert_eq!(inv.rows, o["rows"].as_u64().unwrap());
            let groups: Vec<&str> = inv.structures.iter().map(|x| x.name.as_str()).collect();
            assert_eq!(serde_json::json!(groups), o["groups"], "{name} {key}");
            for (st, row) in inv
                .structures
                .iter()
                .zip(o["identity_rows"].as_array().unwrap())
            {
                assert_eq!(
                    st.identity.id.row,
                    row.as_u64().unwrap(),
                    "{name} {key} {}",
                    st.name
                );
                assert_eq!(st.identity.id.table, o["table"].as_str().unwrap());
                let w = &o["structures"][&st.name];
                let mut series: Vec<_> = st.series.iter().collect();
                series.sort_by(|a, b| a.relative.cmp(&b.relative));
                let ws = w["series"].as_array().unwrap();
                assert_eq!(series.len(), ws.len(), "{name} {key} {}", st.name);
                for (sr, wr) in series.iter().zip(ws) {
                    assert_eq!(sr.relative, wr["relative"].as_str().unwrap());
                    assert_eq!(serde_json::json!(sr.shape), wr["shape"]);
                    assert!(sr.capability.is_supported());
                    assert_eq!(
                        labels_json_set(&sr.columns),
                        wr["labels"],
                        "{name} {} {}",
                        st.name,
                        sr.relative
                    );
                    let full = s
                        .read_one_d_structure_series(
                            kind,
                            mode,
                            Some(&st.name),
                            &sr.relative,
                            Slice::new(0, sr.shape[0], 1),
                            None,
                            64 * MB,
                            64 * MB,
                        )
                        .unwrap();
                    assert_eq!(sha(&le(&full.values)), wr["sha"].as_str().unwrap());
                    let sm = &wr["sample"];
                    let part = s
                        .read_one_d_structure_series(
                            kind,
                            mode,
                            Some(&st.name),
                            &sr.relative,
                            Slice::new(
                                sm[0].as_u64().unwrap(),
                                sm[1].as_u64().unwrap(),
                                sm[2].as_u64().unwrap(),
                            ),
                            None,
                            64 * MB,
                            64 * MB,
                        )
                        .unwrap();
                    assert_eq!(sha(&le(&part.values)), wr["sample_sha"].as_str().unwrap());
                    let e = s
                        .read_one_d_structure_series(
                            kind,
                            mode,
                            Some(&st.name),
                            &sr.relative,
                            Slice::new(0, sr.shape[0], 1),
                            None,
                            64 * MB,
                            0,
                        )
                        .unwrap_err();
                    assert_eq!(e.kind, ErrorKind::InvalidRequest);
                }
                let mut statics = st.static_datasets.clone();
                statics.sort();
                let mut want: Vec<String> = w["statics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_str().unwrap().to_string())
                    .collect();
                want.sort();
                assert_eq!(statics, want);
            }
            let fam = o["family_series"].as_object().unwrap();
            assert_eq!(inv.family_series.len(), fam.len());
            for fs in &inv.family_series {
                let a = s
                    .read_one_d_structure_series(
                        kind,
                        mode,
                        None,
                        &fs.relative,
                        Slice::new(0, fs.shape[0], 1),
                        None,
                        64 * MB,
                        64 * MB,
                    )
                    .unwrap();
                assert_eq!(sha(&le(&a.values)), fam[&fs.relative].as_str().unwrap());
            }
        }
        // Structure geometry records and exact group dumps.
        let g = s.structure_geometry(64 * MB).unwrap();
        match c["structure_geometry"].as_object() {
            None => assert!(g.is_unavailable(), "{name}"),
            Some(w) => {
                let g = g.value().unwrap();
                assert_eq!(g.records.len() as u64, w["rows"].as_u64().unwrap());
                assert_eq!(
                    sha(&g.attributes.raw_bytes),
                    w["attributes_sha"].as_str().unwrap()
                );
                for (r, wr) in g.records.iter().zip(w["records"].as_array().unwrap()) {
                    let t = |x: &Reported<String>| x.value().map(|v| trim(v).to_string());
                    assert_eq!(
                        serde_json::json!([
                            t(&r.structure_type),
                            t(&r.river),
                            t(&r.reach),
                            t(&r.station)
                        ]),
                        wr[0]
                    );
                    let ranges: Vec<Value> = r
                        .profile_ranges
                        .iter()
                        .map(|(n, rg)| serde_json::json!([n, rg.start, rg.end]))
                        .collect();
                    assert_eq!(Value::Array(ranges), wr[1], "{name}");
                }
            }
        }
        for (key, grp) in [
            ("structures", OneDGeometryGroup::Structures),
            ("pump_stations", OneDGeometryGroup::PumpStations),
            ("storage_areas", OneDGeometryGroup::StorageAreas),
        ] {
            let r = s.one_d_geometry_group(grp, 256 * MB).unwrap();
            let Some(w) = c["geometry_groups"][key].as_object() else {
                assert!(r.is_unavailable(), "{name} {key}");
                continue;
            };
            let d = r.value().unwrap();
            let tables: serde_json::Map<String, Value> = d
                .tables
                .iter()
                .map(|(n, t)| (n.clone(), Value::String(sha(&t.raw_bytes))))
                .collect();
            assert_eq!(Value::Object(tables), w["tables"], "{name} {key}");
            let arrays: serde_json::Map<String, Value> = d
                .arrays
                .iter()
                .map(|(n, a)| (n.clone(), Value::String(sha(&le(&a.values)))))
                .collect();
            assert_eq!(Value::Object(arrays), w["arrays"], "{name} {key}");
            let other: Vec<&str> = d.unsupported.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(serde_json::json!(other), w["other"]);
        }
        let ps = s.pump_stations(64 * MB).unwrap();
        match c["pump_stations"].as_object() {
            None => assert!(ps.is_unavailable()),
            Some(w) => {
                let p = ps.value().unwrap();
                assert_eq!(p.features.len() as u64, w["rows"].as_u64().unwrap());
                assert_eq!(
                    sha(&le(&p.points.values)),
                    w["points_sha"].as_str().unwrap()
                );
                let names: Vec<Option<String>> = p
                    .names
                    .iter()
                    .map(|n| n.value().map(|v| trim(v).to_string()))
                    .collect();
                assert_eq!(serde_json::json!(names), w["names"]);
            }
        }
    }
}

fn labels_json_set(r: &ras_commander_fast_hdf::hdf::structures::ColumnLabels) -> Value {
    match r.capability() {
        Capability::Supported => Value::Array(
            r.value()
                .unwrap()
                .labels
                .iter()
                .map(|l| serde_json::json!([trim(&l.name), l.unit.as_deref().map(trim)]))
                .collect(),
        ),
        Capability::Unavailable { .. } => Value::Null,
        _ => Value::String("corrupt".into()),
    }
}

#[test]
#[ignore = "requires RAS_WAVE2_1D_ORACLE (frozen h5py manifest of real HEC-RAS files)"]
fn wave2_layer_inventory_lists_one_d_families() {
    let j = manifest();
    for (c, s) in cases(&j) {
        let name = c["name"].as_str().unwrap();
        let layers = s.layer_inventory(512 * MB).unwrap();
        let of = |f: &str| layers.iter().filter(|l| l.family == f).collect::<Vec<_>>();
        let xs = of("xs_geometry");
        match c["xs_geometry"]["status"].as_str().unwrap() {
            "supported" => {
                for &field in XS_GEOMETRY_FIELDS {
                    let l = xs.iter().find(|l| l.name == field.as_str()).unwrap();
                    let want = c["xs_geometry"]["fields"][field.as_str()]["status"]
                        .as_str()
                        .unwrap();
                    assert_eq!(l.capability.as_str(), want, "{name} {field:?}");
                }
                let unq = xs.iter().find(|l| l.name == "unqualified");
                assert_eq!(
                    unq.map_or(0, |l| l.fields.len()),
                    c["xs_geometry"]["unqualified"].as_array().unwrap().len()
                );
            }
            _ => assert!(xs.iter().all(|l| !l.capability.is_supported())),
        }
        for &family in ONE_D_LINE_FAMILIES {
            let l = of("one_d_lines")
                .into_iter()
                .find(|l| l.name == family.as_str())
                .unwrap();
            let o = &c["lines"][family.as_str()];
            let cut_without_xs = family == OneDLineFamily::CrossSectionCutLines
                && c["xs_geometry"]["status"] == "absent";
            if o["status"] == "unavailable" {
                assert!(
                    matches!(l.capability, Capability::Unavailable { .. }),
                    "{name} {family:?}"
                );
            } else if cut_without_xs {
                assert_eq!(l.capability.as_str(), "unsupported", "{name} {family:?}");
            } else {
                assert!(
                    l.capability.is_supported(),
                    "{name} {family:?}: {:?}",
                    l.capability
                );
                assert_eq!(l.row_count, o["features"].as_u64(), "{name} {family:?}");
            }
        }
        for (m, o) in c["xs_results"].as_object().unwrap() {
            let n = of("xs_results")
                .iter()
                .filter(|l| l.name.starts_with(&format!("{m}/")) && l.capability.is_supported())
                .count();
            let want = o
                .get("variables")
                .and_then(|v| v.as_object())
                .map_or(0, |v| v.values().filter(|x| x["readable"] == true).count());
            assert_eq!(n, want, "{name} {m}");
        }
        let summ = of("one_d_summary")
            .iter()
            .filter(|l| l.capability.is_supported())
            .count();
        assert_eq!(
            summ,
            c["summary"].as_array().map_or(0, |a| a.len()),
            "{name}"
        );
        let structures: usize = c["structure_results"]
            .as_object()
            .unwrap()
            .values()
            .map(|o| {
                o.get("groups")
                    .and_then(|g| g.as_array())
                    .map_or(0, |g| g.len())
            })
            .sum();
        let got = of("structure_1d_results")
            .iter()
            .filter(|l| l.element_domain == "series" && l.identity.is_some())
            .count();
        assert_eq!(got, structures, "{name}");
        // Budgets stay isolated per layer; the catalogue remains a reviewable report.
        let tiny = s.layer_inventory(0).unwrap();
        assert!(tiny.iter().any(|l| l.family == "one_d_lines"));
    }
}
