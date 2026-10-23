#![cfg(feature = "hdf")]
//! Synthetic time-axis/units fixtures (generator: tests/fixtures/time/make_fixtures.py) with
//! SHA256 manifest, parser unit tests, and an env-gated real-file oracle comparison.
use ras_commander_rust::hdf::time::{
    parse_hec_ms_stamp, parse_hec_stamp, CivilTimestamp, MillisecondSource, TimeEpoch,
    UNSTEADY_TIME_GROUP,
};
use ras_commander_rust::hdf::units::{
    UnitSystemKind, UnitsEvidence, UnitsFamily, UnitsSource, MESH2D_UNIT_VARIABLES,
};
use ras_commander_rust::hdf::DEFAULT_MAX_BYTES;
use ras_commander_rust::hdf::{ReadOnlyHdfSession, Slice};
use ras_commander_rust::{Capability, ErrorKind};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const MAX: usize = 1 << 20;
fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/time")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(dir().join("manifest.json")).unwrap()).unwrap()
}
fn open(name: &str) -> ReadOnlyHdfSession {
    let m = manifest();
    let entry = &m["fixtures"][name];
    let path = dir().join(entry["file"].as_str().unwrap());
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        entry["sha256"].as_str().unwrap(),
        "{name} hash"
    );
    ReadOnlyHdfSession::open(path).unwrap()
}
fn kind(s: &str) -> ErrorKind {
    match s {
        "Corrupt" => ErrorKind::Corrupt,
        "Unsupported" => ErrorKind::Unsupported,
        "NotFound" => ErrorKind::NotFound,
        _ => panic!("kind {s}"),
    }
}
fn strings(v: &serde_json::Value) -> Option<Vec<String>> {
    v.as_array()
        .map(|a| a.iter().map(|s| s.as_str().unwrap().to_owned()).collect())
}
fn ints(v: &serde_json::Value) -> Option<Vec<i64>> {
    v.as_array()
        .map(|a| a.iter().map(|s| s.as_i64().unwrap()).collect())
}

#[test]
fn manifest_time_cases_match_independent_oracle() {
    let m = manifest();
    let mut checked = 0;
    for (name, entry) in m["fixtures"].as_object().unwrap() {
        let expect = &entry["expect"];
        if expect.get("units").is_some() {
            continue;
        }
        let s = open(name);
        let got = s.unsteady_time_axis(MAX);
        if let Some(e) = expect.get("error") {
            let err = got.expect_err(name);
            assert_eq!(err.kind, kind(e.as_str().unwrap()), "{name}: {err}");
        } else {
            let axis = got.unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(axis.group, UNSTEADY_TIME_GROUP);
            assert_eq!(axis.epoch, TimeEpoch::CivilUnix1970);
            assert_eq!(axis.raw_strings, strings(&expect["strings"]), "{name}");
            assert_eq!(axis.ms_since_epoch, ints(&expect["ms"]), "{name}");
            assert_eq!(
                axis.raw_ms_strings,
                strings(&expect["ms_strings"]),
                "{name}"
            );
            assert_eq!(axis.raw_ms, ints(&expect["raw_ms"]), "{name}");
            assert_eq!(
                axis.integer_ms.is_some(),
                expect["integer_ms_unsupported"] == true,
                "{name}"
            );
            if axis.raw_ms.is_some() {
                assert!(matches!(
                    axis.integer_ms,
                    Some(Capability::Unsupported { .. })
                ));
            }
            let source = expect["ms_source"].as_str().map(|s| match s {
                "StoredStringDataset" => MillisecondSource::StoredStringDataset,
                _ => MillisecondSource::DerivedFromStrings,
            });
            assert_eq!(axis.ms_source, source, "{name}");
            let rows: Vec<u64> = expect["hour_2400_rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap())
                .collect();
            assert_eq!(axis.hour_2400_rows, rows, "{name}");
            if let Some(ms) = &axis.ms_since_epoch {
                let ts = axis.timestamps.as_ref().unwrap();
                assert_eq!(ts.len(), ms.len());
                for (t, m) in ts.iter().zip(ms) {
                    assert_eq!(t.ms_since_epoch(), *m);
                    assert_eq!(CivilTimestamp::from_ms_since_epoch(*m).unwrap(), *t);
                }
            } else {
                assert!(axis.timestamps.is_none());
            }
        }
        checked += 1;
    }
    assert_eq!(checked, 30);
}

#[test]
fn rollover_2400_is_next_day_midnight_and_raw_strings_are_kept() {
    let axis = open("rollover_2400_with_ms").base_axis();
    let ts = axis.timestamps.as_ref().unwrap();
    assert_eq!(ts[2], CivilTimestamp::new(2020, 1, 1, 0, 0, 0, 0).unwrap());
    assert_eq!(ts[3].to_string(), "2020-01-01T01:00:00.000");
    assert_eq!(axis.raw_strings.as_ref().unwrap()[2], "31DEC2019 24:00:00");
    assert_eq!(axis.hour_2400_rows, vec![2]);
    // Integer ms has no proven epoch: retained, never used for civil time.
    assert_eq!(axis.ms_source, Some(MillisecondSource::DerivedFromStrings));
    assert!(axis.raw_ms.is_some() && axis.integer_ms.is_some());
    // f32 days are widened exactly, not rounded to a decimal.
    assert_eq!(axis.days.as_ref().unwrap()[1], f64::from(1.0f32 / 24.0));
    assert_eq!(axis.rows, 4);
}
trait AxisExt {
    fn base_axis(&self) -> ras_commander_rust::hdf::time::TimeAxis;
}
impl AxisExt for ReadOnlyHdfSession {
    fn base_axis(&self) -> ras_commander_rust::hdf::time::TimeAxis {
        self.unsteady_time_axis(MAX).unwrap()
    }
}

#[test]
fn leap_day_and_month_names() {
    let leap = open("leap_day").base_axis();
    let ts = leap.timestamps.unwrap();
    assert_eq!(ts[1], CivilTimestamp::new(2020, 2, 29, 0, 0, 0, 0).unwrap());
    assert_eq!(ts[2], CivilTimestamp::new(2020, 3, 1, 0, 0, 0, 0).unwrap());
    let months = open("month_names").base_axis();
    let ts = months.timestamps.unwrap();
    assert_eq!(ts.len(), 12);
    for (i, t) in ts.iter().enumerate() {
        assert_eq!((t.year, t.month, t.day, t.hour), (2021, i as u8 + 1, 15, 6));
    }
}

#[test]
fn error_categories_are_explicit() {
    let e = open("impossible_leap_day")
        .unsteady_time_axis(MAX)
        .unwrap_err();
    assert!(e.message.contains("row 0"), "{e}");
    let e = open("ms_strings_mismatch")
        .unsteady_time_axis(MAX)
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Corrupt);
    assert!(e.message.contains("row 1"), "{e}");
    let e = open("length_mismatch_results")
        .unsteady_time_axis(MAX)
        .unwrap_err();
    assert!(e.message.contains("4 rows but results have 5"), "{e}");
    // Explicit expected_rows on a standalone axis.
    let s = open("rollover_2400_strings_only");
    assert!(s.time_axis(UNSTEADY_TIME_GROUP, Some(4), MAX).is_ok());
    assert_eq!(
        s.time_axis(UNSTEADY_TIME_GROUP, Some(3), MAX)
            .unwrap_err()
            .kind,
        ErrorKind::Corrupt
    );
    assert_eq!(
        s.time_axis(UNSTEADY_TIME_GROUP, None, 8).unwrap_err().kind,
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        s.time_axis("Results/Nope", None, MAX).unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[test]
fn row_times_select_stored_rows_without_changing_numeric_reads() {
    let s = open("rollover_2400_with_ms");
    let sel = s.unsteady_row_times(Slice::new(1, 4, 2), MAX).unwrap();
    assert_eq!(sel.row_indices, vec![1, 3]);
    assert_eq!(
        sel.raw_strings.unwrap(),
        vec!["31DEC2019 23:00:00", "01JAN2020 01:00:00"]
    );
    assert_eq!(sel.timestamps.unwrap()[1].hour, 1);
    assert_eq!(
        s.unsteady_row_times(Slice::new(0, 5, 1), MAX)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        s.unsteady_row_times(Slice::new(0, 2, 0), MAX)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    // Existing numeric path is untouched (shape/dtype of stored XS result).
    let a = s
        .read_numeric(
            "Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/Cross Sections/Water Surface",
            None,
        )
        .unwrap();
    assert_eq!(a.shape, vec![4, 2]);
}

#[test]
fn real_ms_string_form_is_parsed_checked_and_kept_raw() {
    let a = open("ms_strings_2400").base_axis();
    assert_eq!(a.ms_source, Some(MillisecondSource::StoredStringDataset));
    assert!(a.raw_ms.is_none());
    assert_eq!(
        a.raw_ms_strings.as_ref().unwrap()[2],
        "31DEC2019 24:00:00:000"
    );
    assert_eq!(
        a.timestamps.as_ref().unwrap()[2].to_string(),
        "2020-01-01T00:00:00.000"
    );
    assert_eq!(a.hour_2400_rows, vec![2]);
    let a = open("ms_strings_subsecond_only").base_axis();
    let ts = a.timestamps.unwrap();
    assert_eq!((ts[0].second, ts[0].millisecond), (0, 250));
    assert_eq!(ts[2].to_string(), "2020-01-02T00:00:00.000");
    let e = open("ms_strings_mismatch")
        .unsteady_time_axis(MAX)
        .unwrap_err();
    assert!(e.message.contains("row 1"), "{e}");
    let e = open("days_disagree_with_stamps")
        .unsteady_time_axis(MAX)
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Corrupt);
    assert!(e.message.contains("Time (days)"), "{e}");
    let p = parse_hec_ms_stamp("01JAN1999 12:00:00:007").unwrap();
    assert_eq!(p.timestamp.millisecond, 7);
    for bad in [
        "01JAN1999 12:00:00",
        "01JAN1999 12:00:00:7",
        "01JAN1999 12:00:00:0000",
        "01JAN1999 12:00:00:abc",
    ] {
        assert_eq!(
            parse_hec_ms_stamp(bad).unwrap_err().kind,
            ErrorKind::Unsupported,
            "{bad}"
        );
    }
    // 24:00:00:001 is not an end-of-day alias.
    assert_eq!(
        parse_hec_ms_stamp("01JAN1999 24:00:00:001")
            .unwrap_err()
            .kind,
        ErrorKind::Corrupt
    );
    assert_eq!(
        parse_hec_ms_stamp("01JAN1999 12:00:00:999")
            .unwrap()
            .timestamp
            .millisecond,
        999
    );
    // The seconds-only parser does not accept the ms form.
    assert_eq!(
        parse_hec_stamp("01JAN1999 12:00:00:000").unwrap_err().kind,
        ErrorKind::Unsupported
    );
}

#[test]
fn integer_ms_extremes_do_not_overflow_or_become_civil_time() {
    let a = open("ms_extreme_integers").base_axis();
    assert_eq!(a.raw_ms, Some(vec![i64::MIN, 0, i64::MAX]));
    assert!(a.ms_since_epoch.is_none() && a.timestamps.is_none() && a.ms_source.is_none());
    assert!(matches!(a.integer_ms, Some(Capability::Unsupported { .. })));
    let a = open("ms_only").base_axis();
    assert!(a.timestamps.is_none() && a.raw_ms.is_some());
    assert_eq!(
        open("ms_u64_overflow")
            .unsteady_time_axis(MAX)
            .unwrap_err()
            .kind,
        ErrorKind::Corrupt
    );
}

#[test]
fn subsecond_ms_strings_pass_whole_second_stamp_check_and_many_2400_rows_are_linear() {
    let a = open("ms_strings_subsecond_with_stamps").base_axis();
    assert_eq!(a.ms_source, Some(MillisecondSource::StoredStringDataset));
    let ts = a.timestamps.unwrap();
    assert_eq!((ts[0].second, ts[0].millisecond), (0, 250));
    assert_eq!(ts[1].millisecond, 999);
    // Whole-second disagreement is still Corrupt (ms_strings_mismatch covers it).
    let a = open("daily_2400_many_rows").base_axis();
    assert_eq!(a.hour_2400_rows.len(), 3000);
    assert!(a.hour_2400_rows.windows(2).all(|w| w[0] + 1 == w[1]));
}

#[test]
fn stamp_parser_boundaries() {
    let p = parse_hec_stamp("01jan2020 00:00").unwrap();
    assert_eq!(p.timestamp.to_string(), "2020-01-01T00:00:00.000");
    assert!(!p.hour_2400);
    assert_eq!(
        parse_hec_stamp("  31DEC1999 24:00:00 ").unwrap().timestamp,
        CivilTimestamp::new(2000, 1, 1, 0, 0, 0, 0).unwrap()
    );
    // 2100 is not a leap year, 2000 is.
    assert_eq!(
        parse_hec_stamp("29FEB2100 01:00:00").unwrap_err().kind,
        ErrorKind::Corrupt
    );
    assert!(parse_hec_stamp("29FEB2000 01:00:00").is_ok());
    // 24:30 is not a valid end-of-day alias.
    assert_eq!(
        parse_hec_stamp("01JAN2020 24:30:00").unwrap_err().kind,
        ErrorKind::Corrupt
    );
    assert_eq!(
        parse_hec_stamp("01JAN2020").unwrap_err().kind,
        ErrorKind::Unsupported
    );
    assert_eq!(
        parse_hec_stamp("1JAN2020 00:00:00").unwrap_err().kind,
        ErrorKind::Unsupported
    );
    assert_eq!(
        parse_hec_stamp("01JAN2020 0:00:00").unwrap_err().kind,
        ErrorKind::Unsupported
    );
    assert_eq!(
        parse_hec_stamp("01JAN2020 00:00:60").unwrap_err().kind,
        ErrorKind::Corrupt
    );
    assert_eq!(
        parse_hec_stamp("\u{ff11}1JAN2020 00:00:00")
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    // ms round trip across the epoch and a pre-epoch day.
    for ms in [0i64, -1, 86_399_999, 951_782_400_000, -2_208_988_800_000] {
        let t = CivilTimestamp::from_ms_since_epoch(ms).unwrap();
        assert_eq!(t.ms_since_epoch(), ms);
    }
    assert_eq!(
        CivilTimestamp::from_ms_since_epoch(i64::MAX)
            .unwrap_err()
            .kind,
        ErrorKind::Corrupt
    );
}

fn declared(e: &UnitsEvidence) -> &str {
    match e.value() {
        Some(text) => text,
        None => panic!("not declared: {e:?}"),
    }
}
fn is_unsupported(e: &UnitsEvidence) -> bool {
    matches!(e.capability(), Capability::Unsupported { .. })
}

#[test]
fn units_are_reported_exactly_with_missing_and_non_ascii_cases() {
    let r = open("units_full").result_units(DEFAULT_MAX_BYTES).unwrap();
    assert_eq!(r.variables.len(), 6);
    let find = |fam: UnitsFamily, v: &str| {
        r.variables
            .iter()
            .find(|x| x.family == fam && x.variable == v)
            .unwrap()
    };
    let ws = find(UnitsFamily::Mesh2D, "Water Surface");
    assert_eq!(ws.area.as_deref(), Some("Perimeter 1"));
    assert_eq!(declared(&ws.units), "ft");
    assert_eq!(declared(&find(UnitsFamily::Mesh2D, "Depth").units), "ft");
    // A dataset without a Units attribute is Absent, never defaulted.
    assert!(find(UnitsFamily::Mesh2D, "Velocity").units.is_unavailable());
    let face = find(UnitsFamily::Mesh2D, "Face Velocity");
    assert_eq!(declared(&face.units), "m\u{b3}/s");
    assert_eq!(face.units.raw_bytes(), Some("m\u{b3}/s".as_bytes()));
    let flow = find(UnitsFamily::CrossSectionUnsteady, "Flow");
    assert_eq!(declared(&flow.units), "ft\u{b3}/s");
    assert_eq!(MESH2D_UNIT_VARIABLES.len(), 4);
    // Both Units System sources are reported separately; the conflict is not resolved.
    assert_eq!(r.unit_system.len(), 2);
    assert_eq!(r.unit_system[0].source, "/");
    assert_eq!(r.unit_system[0].kind, Some(UnitSystemKind::UsCustomary));
    assert_eq!(r.unit_system[1].source, "/Geometry");
    assert_eq!(r.unit_system[1].kind, Some(UnitSystemKind::SiMetric));
}

#[test]
fn undecodable_wrong_type_and_unrecognized_units_stay_evidence() {
    let s = open("units_latin1_numeric");
    let r = s.result_units(DEFAULT_MAX_BYTES).unwrap();
    assert_eq!(r.variables.len(), 2);
    for v in &r.variables {
        assert!(is_unsupported(&v.units), "{:?}", v.units);
        match &*v.variable {
            "Water Surface" => assert_eq!(v.units.raw_bytes(), Some(&[0xb0, b'F'][..])),
            "Flow" => assert_eq!(v.units.raw_bytes(), Some(&7i32.to_le_bytes()[..])),
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(r.unit_system.len(), 1);
    assert_eq!(declared(&r.unit_system[0].value), "Imperial");
    assert_eq!(r.unit_system[0].kind, Some(UnitSystemKind::Unrecognized));
}

#[test]
fn missing_units_and_missing_system_are_absent_not_empty_strings() {
    let r = open("units_missing")
        .result_units(DEFAULT_MAX_BYTES)
        .unwrap();
    assert_eq!(r.variables.len(), 1);
    assert!(r.variables[0].units.is_unavailable());
    assert_eq!(r.unit_system.len(), 1);
    assert!(r.unit_system[0].value.is_unavailable());
    assert_eq!(r.unit_system[0].kind, None);
    // Direct dataset query on a non-existent dataset is NotFound, not Absent.
    let e = open("units_missing")
        .dataset_units_attribute("Results/missing")
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
}

fn oracle_text(v: &serde_json::Value) -> Option<String> {
    v["text"].as_str().map(str::to_owned)
}

/// RAS_NATIVE_FIXTURES = fixture directory; RAS_TIME_ORACLE = JSON from
/// tests/fixtures/time/oracle_real_time_units.py (h5py) for p07/p18/Davis result files.
#[test]
#[ignore = "requires real fixtures (RAS_NATIVE_FIXTURES) and h5py oracle JSON (RAS_TIME_ORACLE)"]
fn real_time_axes_and_units_match_h5py_oracle() {
    let root = PathBuf::from(std::env::var_os("RAS_NATIVE_FIXTURES").expect("RAS_NATIVE_FIXTURES"));
    let oracle: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("RAS_TIME_ORACLE").expect("RAS_TIME_ORACLE")).unwrap(),
    )
    .unwrap();
    let cases = oracle.as_array().unwrap();
    // All three required real fixtures must be present in the oracle.
    for token in [".p07.", ".p18.", "JUL2020_D100RAS.p04"] {
        assert!(
            cases
                .iter()
                .any(|c| c["path"].as_str().unwrap().contains(token)),
            "oracle lacks required file {token}"
        );
    }
    for case in cases {
        let name = case["path"].as_str().unwrap();
        let path = root.join(name);
        assert_eq!(
            format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap())),
            case["sha256"].as_str().unwrap(),
            "{name} hash"
        );
        let axis_o = &case["axis"];
        for threads in [1, 4] {
            let s = ReadOnlyHdfSession::with_options(&path, threads, true).unwrap();
            let got = s.unsteady_time_axis(256 << 20);
            if axis_o.get("ms_matches_strings") == Some(&serde_json::Value::Bool(false))
                || axis_o.get("days_consistent") == Some(&serde_json::Value::Bool(false))
            {
                assert_eq!(got.expect_err(name).kind, ErrorKind::Corrupt, "{name}");
            } else if axis_o.get("strings").is_none()
                && axis_o.get("ms").is_none()
                && axis_o.get("days").is_none()
            {
                assert_eq!(got.expect_err(name).kind, ErrorKind::NotFound, "{name}");
            } else {
                let a = got.unwrap_or_else(|e| panic!("{name}: {e}"));
                assert_eq!(a.raw_strings, strings(&axis_o["strings"]), "{name}");
                assert_eq!(a.raw_ms, ints(&axis_o["ms"]), "{name}");
                assert_eq!(a.raw_ms_strings, strings(&axis_o["ms_strings"]), "{name}");
                // Bit-exact: the oracle freezes u64 IEEE-754 bits, never decimal text.
                let days = axis_o["days_bits"]
                    .as_array()
                    .map(|d| d.iter().map(|v| v.as_u64().unwrap()).collect::<Vec<_>>());
                let got = a
                    .days
                    .as_ref()
                    .map(|d| d.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
                assert_eq!(got, days, "{name}");
                let expect_ms =
                    ints(&axis_o["ms_string_ms"]).or_else(|| ints(&axis_o["string_ms"]));
                assert_eq!(a.ms_since_epoch, expect_ms, "{name}");
                for (p, rows) in axis_o["result_rows"].as_object().unwrap() {
                    assert_eq!(a.rows, rows.as_u64().unwrap(), "{name} {p}");
                }
            }
        }
        let s = ReadOnlyHdfSession::open(&path).unwrap();
        let r = s.result_units(DEFAULT_MAX_BYTES).unwrap();
        let units_o = &case["units"];
        let vo = units_o["variables"].as_array().unwrap();
        assert_eq!(r.variables.len(), vo.len(), "{name} unit variable count");
        for want in vo {
            let ds = want["dataset"].as_str().unwrap();
            let got = r
                .variables
                .iter()
                .find(|v| v.dataset == ds)
                .unwrap_or_else(|| panic!("{name}: native lacks {ds}"));
            let e = &want["expected"];
            match (got.units.value(), e["status"].as_str().unwrap()) {
                (None, "absent") if got.units.is_unavailable() => {}
                (Some(text), "declared") => {
                    assert_eq!(Some(text.as_str()), e["text"].as_str(), "{name} {ds}")
                }
                (None, "unsupported") if is_unsupported(&got.units) => {}
                other => panic!("{name} {ds}: {other:?}"),
            }
            let src = match e["source"].as_str().unwrap() {
                "UnitsAttribute" => UnitsSource::UnitsAttribute,
                "VariableUnits" => UnitsSource::VariableUnits,
                "VariablePair" => UnitsSource::VariablePair,
                "QuantityLabel" => UnitsSource::QuantityLabel {
                    attribute: e["attribute"].as_str().unwrap().into(),
                },
                _ => UnitsSource::None,
            };
            assert_eq!(got.source, src, "{name} {ds}");
            assert_eq!(
                got.quantity_name.as_deref(),
                e["quantity_name"].as_str(),
                "{name} {ds}"
            );
        }
        if name.contains("JUL2020_D100RAS.p04") {
            // Davis: 1D unsteady Cross Sections with declared units and a stored row count.
            for v in ["Water Surface", "Flow"] {
                let x = r
                    .variables
                    .iter()
                    .find(|x| x.family == UnitsFamily::CrossSectionUnsteady && x.variable == v)
                    .unwrap_or_else(|| panic!("{name}: no 1D XS {v}"));
                assert!(x.units.is_supported(), "{name} {v}");
            }
            let rows = axis_o["result_rows"].as_object().unwrap();
            assert!(
                rows.keys().any(|k| k.contains("/Cross Sections/")),
                "{name}"
            );
        }
        let so = units_o["unit_system"].as_array().unwrap();
        assert_eq!(r.unit_system.len(), so.len(), "{name} unit system sources");
        for (got, want) in r.unit_system.iter().zip(so) {
            assert_eq!(got.source, want["source"].as_str().unwrap());
            match (
                got.value.value(),
                want["value"]["present"].as_bool().unwrap(),
            ) {
                (None, false) if got.value.is_unavailable() => {}
                (Some(text), true) => {
                    assert_eq!(Some(text.clone()), oracle_text(&want["value"]), "{name}")
                }
                other => panic!("{name} {}: {other:?}", got.source),
            }
        }
    }
}

fn find<'a>(
    r: &'a ras_commander_rust::hdf::units::ResultUnits,
    fam: UnitsFamily,
    v: &str,
) -> &'a ras_commander_rust::hdf::units::VariableUnits {
    r.variables
        .iter()
        .find(|x| x.family == fam && x.variable == v)
        .unwrap_or_else(|| panic!("missing {v}"))
}

#[test]
fn real_hecras_unit_layouts_report_source_attribute_without_inference() {
    use UnitsFamily::{CrossSectionUnsteady as Xs, Mesh2D};
    let r = open("units_real_layouts")
        .result_units(DEFAULT_MAX_BYTES)
        .unwrap();
    assert_eq!(
        r.variables.len(),
        10,
        "Cross Section Only is not a result variable"
    );
    let ws = find(&r, Xs, "Water Surface");
    assert_eq!(
        (declared(&ws.units), &ws.source),
        ("Feet", &UnitsSource::VariableUnits)
    );
    assert_eq!(ws.quantity_name.as_deref(), Some("Stage"));
    let flow = find(&r, Xs, "Flow");
    assert_eq!(
        (declared(&flow.units), flow.quantity_name.as_deref()),
        ("cfs", Some("Flow"))
    );
    for (v, attr, unit) in [
        ("Flow Lateral", "Lateral Flow", "cfs"),
        (
            "Flow Volume Cumulative",
            "Cumulative Volumetric Flow",
            "Feet^3",
        ),
        ("Velocity Channel", "Velocity", "ft/s"),
        ("Velocity Total", "Velocity", "ft/s"),
    ] {
        let x = find(&r, Xs, v);
        assert_eq!(declared(&x.units), unit, "{v}");
        assert_eq!(
            x.source,
            UnitsSource::QuantityLabel {
                attribute: attr.into()
            }
        );
        assert_eq!(x.quantity_name, None);
    }
    for (v, attr, unit) in [
        ("Water Surface", "WSEL", "Feet"),
        ("Depth", "Depth", "Feet"),
        ("Face Velocity", "Face Velocity", "ft/s"),
    ] {
        let x = find(&r, Mesh2D, v);
        assert_eq!(declared(&x.units), unit, "{v}");
        assert_eq!(
            x.source,
            UnitsSource::QuantityLabel {
                attribute: attr.into()
            }
        );
    }
    // A 2D Velocity dataset with no attributes at all stays Absent.
    let v = find(&r, Mesh2D, "Velocity");
    assert!(v.units.is_unavailable());
    assert_eq!(v.source, UnitsSource::None);
    // The legacy Units-only accessor still reports Absent for these layouts.
    assert!(open("units_real_layouts")
        .dataset_units_attribute("Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/Cross Sections/Water Surface")
        .unwrap()
        .is_unavailable());
}

#[test]
fn unit_source_priority_and_ambiguity_are_explicit() {
    use UnitsFamily::CrossSectionUnsteady as Xs;
    let r = open("units_priority_edges")
        .result_units(DEFAULT_MAX_BYTES)
        .unwrap();
    let ws = find(&r, Xs, "Water Surface");
    assert_eq!(
        (declared(&ws.units), &ws.source),
        ("m", &UnitsSource::UnitsAttribute)
    );
    let flow = find(&r, Xs, "Flow");
    assert_eq!(
        (declared(&flow.units), &flow.source),
        ("cfs", &UnitsSource::VariableUnits)
    );
    let lat = find(&r, Xs, "Flow Lateral");
    assert_eq!(
        (declared(&lat.units), &lat.source),
        ("cfs", &UnitsSource::VariablePair)
    );
    assert_eq!(lat.quantity_name.as_deref(), Some("Lateral"));
    let bad = find(&r, Xs, "Velocity Channel");
    assert!(is_unsupported(&bad.units));
    assert_eq!(bad.source, UnitsSource::VariablePair);
    let amb = find(&r, Xs, "Velocity Total");
    assert!(is_unsupported(&amb.units));
    assert_eq!(amb.source, UnitsSource::None);
    let num = find(&r, Xs, "Flow Volume Cumulative");
    assert_eq!(
        num.source,
        UnitsSource::QuantityLabel {
            attribute: "Cumulative Volumetric Flow".into()
        }
    );
    assert!(is_unsupported(&num.units));
    assert_eq!(num.units.raw_bytes(), Some(&5i32.to_le_bytes()[..]));
}
