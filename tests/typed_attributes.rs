#![cfg(feature = "hdf")]
//! Typed per-dataset attributes and exact HDF5 bitfield attributes on the real JUL2020_D100RAS.p04
//! excerpt (`generate_legacy_location_excerpts.py`). The h5py bitfield bytes are recorded below.
use ras_commander_fast_hdf::{
    hdf::{metadata::MetadataValue, AttributeType, ByteOrder, ReadOnlyHdfSession},
    ErrorKind, UnsupportedCode,
};
const AREA: &str = "D601";
const ROOT: &str =
    "Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/2D Flow Areas/D601";
/// h5py `attrs['Property Tables LC Hash'][:8]` (uint8) on the JUL2020 source.
const LC_HASH_HEAD: [u8; 8] = [250, 233, 126, 113, 139, 15, 201, 186];
fn fixture(name: &str) -> ReadOnlyHdfSession {
    ReadOnlyHdfSession::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy_location")
            .join(name),
    )
    .unwrap()
}
#[test]
fn typed_dataset_attributes_types_every_attribute_with_exact_bitfield_bytes() {
    let s = fixture("valid.h5");
    let r = s
        .typed_dataset_attributes(&format!("{ROOT}/Depth"), 1 << 20)
        .unwrap();
    assert!(r.is_supported());
    let attrs = r.value().unwrap();
    let names: Vec<_> = attrs.iter().map(|a| a.name.as_str()).collect();
    for n in [
        "Depth",
        "Maximum Value of Data Set",
        "Property Tables LC Hash",
    ] {
        assert!(names.contains(&n), "{n} in {names:?}");
    }
    let units = attrs.iter().find(|a| a.name == "Depth").unwrap();
    assert_eq!(units.value, MetadataValue::Strings(vec!["Feet".into()]));
    let hash = attrs
        .iter()
        .find(|a| a.name == "Property Tables LC Hash")
        .unwrap();
    assert_eq!(
        hash.attribute_type,
        AttributeType::Bitfield {
            size: 1,
            order: ByteOrder::Little
        }
    );
    assert!(hash.attribute_type.capability().is_supported());
    assert_eq!(hash.shape, vec![32]);
    let MetadataValue::Bitfield(bytes) = &hash.value else {
        panic!("typed bitfield required: {:?}", hash.value)
    };
    assert_eq!(bytes.len(), 32);
    assert_eq!(&bytes[..8], &LC_HASH_HEAD);
    assert_eq!(bytes, &hash.raw_bytes);
    // Group attributes share the proof.
    let group = s
        .typed_group_attributes(&format!("Geometry/2D Flow Areas/{AREA}"))
        .unwrap();
    let g = group
        .iter()
        .find(|a| a.name == "Property Tables LC Hash")
        .unwrap();
    assert_eq!(g.value, hash.value);
}

#[test]
fn typed_dataset_attributes_budgets_absence_and_invalid_requests() {
    let s = fixture("valid.h5");
    let depth = format!("{ROOT}/Depth");
    // The summed payload (bitfield 32 bytes + text + f32) exceeds 16 bytes.
    let over = s.typed_dataset_attributes(&depth, 16).unwrap();
    assert_eq!(
        over.capability().code(),
        Some(UnsupportedCode::ResourceLimit)
    );
    assert!(over.value().is_none());
    // The budget is the exact summed raw payload: equal passes, one byte less refuses.
    let total: usize = s
        .typed_dataset_attributes(&depth, 1 << 20)
        .unwrap()
        .value()
        .unwrap()
        .iter()
        .map(|a| a.raw_bytes.len())
        .sum();
    assert!(s
        .typed_dataset_attributes(&depth, total)
        .unwrap()
        .is_supported());
    assert_eq!(
        s.typed_dataset_attributes(&depth, total - 1)
            .unwrap()
            .capability()
            .code(),
        Some(UnsupportedCode::ResourceLimit)
    );
    for absent in [format!("{ROOT}/Nope"), "No/Such/Group/x".into()] {
        assert!(s
            .typed_dataset_attributes(&absent, 1 << 20)
            .unwrap()
            .is_unavailable());
    }
    for bad in ["", "/", ROOT, "a/../b"] {
        let e = s.typed_dataset_attributes(bad, 1 << 20).unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidRequest, "{bad:?}");
    }
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
#[test]
fn raw_fixed_strings_return_the_exact_stored_bytes_and_declared_padding() {
    use ras_commander_fast_hdf::hdf::{StringCharset, StringPadding};
    let s = fixture("valid.h5");
    // A real 5.0.7 SPACEPAD declaration: row 0 is unwritten NUL fill, row 1 is space padded.
    let areas = s
        .raw_fixed_strings("Results/Unsteady/Geometry Info/2D Area(s)", 128)
        .unwrap()
        .into_value()
        .unwrap();
    assert_eq!(
        (
            areas.shape.clone(),
            areas.width,
            areas.padding,
            areas.charset
        ),
        (vec![2], 64, StringPadding::SpacePad, StringCharset::Ascii)
    );
    let mut stored = vec![0u8; 64];
    stored.extend(b"D601");
    stored.resize(128, b' ');
    assert_eq!(areas.raw_bytes, stored);
    // h5py `d[()].tobytes()` reads into NULLPAD memory: HDF5 strips the trailing spaces of a
    // SPACEPAD element and pads with NULs. The stored bytes plus padding reproduce it exactly.
    let h5py: Vec<u8> = areas
        .raw_bytes
        .chunks(areas.width)
        .flat_map(|e| {
            let n = e.iter().rposition(|b| *b != b' ').map_or(0, |i| i + 1);
            let mut v = e[..n].to_vec();
            v.resize(e.len(), 0);
            v
        })
        .collect();
    let mut expected = vec![0u8; 128];
    expected[64..68].copy_from_slice(b"D601");
    assert_eq!(h5py, expected);
    // h5py-written NULLPAD S19 keeps its trailing NUL byte.
    let stamps = s
        .raw_fixed_strings(
            "Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/Time Date Stamp",
            1 << 20,
        )
        .unwrap()
        .into_value()
        .unwrap();
    assert_eq!((stamps.width, stamps.padding), (19, StringPadding::NullPad));
    assert_eq!(
        stamps.raw_bytes,
        unhex("30344a554c323032302030303a30303a30300030344a554c323032302030303a31353a303000")
    );
    // The decoded reader trims what the raw payload retains.
    assert_eq!(
        s.read_strings(
            "Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/Time Date Stamp"
        )
        .unwrap()[0]
            .trim_end_matches('\0'),
        "04JUL2020 00:00:00"
    );
}
#[test]
fn raw_fixed_strings_refuse_non_text_budgets_absence_and_groups() {
    let s = fixture("valid.h5");
    let numeric = s
        .raw_fixed_strings(&format!("{ROOT}/Depth"), 1 << 20)
        .unwrap();
    assert_eq!(numeric.capability().code(), Some(UnsupportedCode::Encoding));
    let over = s
        .raw_fixed_strings("Results/Unsteady/Geometry Info/2D Area(s)", 127)
        .unwrap();
    assert_eq!(
        over.capability().code(),
        Some(UnsupportedCode::ResourceLimit)
    );
    assert!(s
        .raw_fixed_strings("Results/Unsteady/Geometry Info/Nope", 1 << 20)
        .unwrap()
        .is_unavailable());
    for bad in ["", "Results/Unsteady/Geometry Info", "a/./b"] {
        assert_eq!(
            s.raw_fixed_strings(bad, 1 << 20).unwrap_err().kind,
            ErrorKind::InvalidRequest,
            "{bad:?}"
        );
    }
}

/// h5py reads fixed strings into NULLPAD memory; derive those bytes from the stored element.
fn h5py_nullpad(stored: &[u8], padding: ras_commander_fast_hdf::hdf::StringPadding) -> Vec<u8> {
    use ras_commander_fast_hdf::hdf::StringPadding as P;
    let n = match padding {
        P::SpacePad => stored.iter().rposition(|b| *b != b' ').map_or(0, |i| i + 1),
        P::NullTerminate => stored.iter().position(|b| *b == 0).unwrap_or(stored.len()),
        _ => stored.len(),
    };
    let mut v = stored[..n].to_vec();
    v.resize(stored.len(), 0);
    v
}
#[test]
#[ignore = "RAS_RAW_PAYLOAD_ORACLE points to the h5py raw-payload oracle (fixtures/raw_payload_oracle.py)"]
fn raw_payload_oracle_bitfield_attributes_and_fixed_strings_match_h5py() {
    use ras_commander_fast_hdf::hdf::StringPadding;
    let path = std::env::var("RAS_RAW_PAYLOAD_ORACLE").expect("RAS_RAW_PAYLOAD_ORACLE required");
    let oracle: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let (mut bitfields, mut strings) = (0usize, 0usize);
    for case in oracle["cases"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        let s = ReadOnlyHdfSession::open(source).unwrap();
        for b in case["bitfields"].as_array().unwrap() {
            let (p, name) = (b["path"].as_str().unwrap(), b["name"].as_str().unwrap());
            let attrs = if b["kind"] == "group" {
                s.typed_group_attributes(p).unwrap()
            } else {
                s.typed_dataset_attributes(p, 1 << 20)
                    .unwrap()
                    .into_value()
                    .unwrap()
            };
            let a = attrs.iter().find(|a| a.name == name).unwrap();
            let order = if b["big"].as_bool().unwrap() {
                ByteOrder::Big
            } else {
                ByteOrder::Little
            };
            let size = b["size"].as_u64().unwrap() as usize;
            assert_eq!(
                a.attribute_type,
                AttributeType::Bitfield { size, order },
                "{source} {p} {name}"
            );
            assert_eq!(serde_json::json!(a.shape), b["shape"]);
            let MetadataValue::Bitfield(bytes) = &a.value else {
                panic!("{source} {p} {name}: {:?}", a.value)
            };
            assert_eq!(&unhex(b["h5py"].as_str().unwrap()), bytes, "{p} {name}");
            bitfields += 1;
        }
        for x in case["strings"].as_array().unwrap() {
            let p = x["path"].as_str().unwrap();
            let r = s.raw_fixed_strings(p, 1 << 20).unwrap();
            let r = r
                .value()
                .unwrap_or_else(|| panic!("{source} {p}: {:?}", r.capability()));
            let padding = match x["padding"].as_u64().unwrap() {
                0 => StringPadding::NullTerminate,
                1 => StringPadding::NullPad,
                _ => StringPadding::SpacePad,
            };
            assert_eq!(
                (r.width, r.padding, serde_json::json!(r.shape)),
                (
                    x["width"].as_u64().unwrap() as usize,
                    padding,
                    x["shape"].clone()
                ),
                "{p}"
            );
            let h5py: Vec<u8> = r
                .raw_bytes
                .chunks(r.width)
                .flat_map(|e| h5py_nullpad(e, r.padding))
                .collect();
            assert_eq!(h5py, unhex(x["h5py"].as_str().unwrap()), "{source} {p}");
            strings += 1;
        }
    }
    let counts = &oracle["counts"];
    assert_eq!(bitfields as u64, counts["bitfields"].as_u64().unwrap());
    assert_eq!(strings as u64, counts["strings"].as_u64().unwrap());
    assert!(bitfields > 0 && strings > 0, "empty oracle");
    println!(
        "raw payload oracle: {bitfields} bitfield attributes, {strings} fixed-string datasets"
    );
}

#[test]
fn space_padded_nul_fill_decodes_like_h5py() {
    use ras_commander_fast_hdf::hdf::{StringCharset, StringPadding};
    // Real excerpts (`generate_spacepad_excerpts.py`): SPACEPAD elements holding
    // only NUL fill are h5py b'' (HDF5 NULLPAD conversion, then NumPy trims trailing NULs).
    let s = ReadOnlyHdfSession::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/spacepad/spacepad.h5"),
    )
    .unwrap();
    let attrs = s
        .typed_dataset_attributes("baxter_storage_area_variables", 1 << 20)
        .unwrap()
        .into_value()
        .unwrap();
    let unit = attrs.iter().find(|a| a.name == "Variable_Unit").unwrap();
    assert_eq!(
        unit.attribute_type,
        AttributeType::FixedString {
            charset: StringCharset::Ascii,
            padding: StringPadding::SpacePad,
            width: 32
        }
    );
    // h5py `attrs['Variable_Unit'].tolist()` on Baxter.p01 Northside.
    let h5py: Vec<String> = [
        "Stage",
        "ft",
        "Net Inflow",
        "cfs",
        "Total Inflow",
        "cfs",
        "Total Outflow",
        "cfs",
        "Surface Area",
        "",
        "Volume",
        "",
    ]
    .iter()
    .map(|v| v.to_string())
    .collect();
    assert_eq!(unit.value, MetadataValue::Strings(h5py));
    // Stored element 9 is 16 NULs then 16 spaces: spaces strip first, then the NUL fill.
    let mut stored = vec![0u8; 16];
    stored.resize(32, b' ');
    assert_eq!(&unit.raw_bytes[9 * 32..10 * 32], stored.as_slice());
    let summary = s.typed_group_attributes("weise_summary").unwrap();
    let transport = summary
        .iter()
        .find(|a| a.name == "Transport Function")
        .unwrap();
    assert_eq!(transport.value, MetadataValue::Strings(vec![String::new()]));
    assert!(transport.raw_bytes.iter().all(|b| *b == 0));
    // read_strings applies the same rule: JUL 2D Area(s) row 0 is unwritten NUL fill.
    let v = fixture("valid.h5");
    assert_eq!(
        v.read_strings("Results/Unsteady/Geometry Info/2D Area(s)")
            .unwrap(),
        vec!["".to_string(), "D601".to_string()]
    );
}
