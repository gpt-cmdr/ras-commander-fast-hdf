#![cfg(feature = "hdf")]
use ras_commander_fast_hdf::{
    hdf::{NumericValues, ReadOnlyHdfSession, Slice},
    ErrorKind, UnsupportedCode,
};
fn session() -> ReadOnlyHdfSession {
    use sha2::{Digest, Sha256};
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bitfield_numeric");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let file = root.join("bitfields.h5");
    assert_eq!(
        format!("{:x}", Sha256::digest(std::fs::read(&file).unwrap())),
        manifest["sha256"].as_str().unwrap()
    );
    ReadOnlyHdfSession::with_options(file, 2, true).unwrap()
}
fn unsigned(v: NumericValues) -> Vec<u64> {
    match v {
        NumericValues::U8(v) => v.into_iter().map(u64::from).collect(),
        NumericValues::U16(v) => v.into_iter().map(u64::from).collect(),
        NumericValues::U32(v) => v.into_iter().map(u64::from).collect(),
        NumericValues::U64(v) => v,
        _ => panic!("bitfield must preserve unsigned bits"),
    }
}
#[test]
fn full_width_bitfields_preserve_bits_in_full_batch_and_strided_slabs() {
    let s = session();
    for bits in [8u32, 16, 32, 64] {
        for endian in ["LE", "BE"] {
            for layout in ["contiguous", "chunked"] {
                let name = format!("b{bits}_{endian}_{layout}");
                let c = s.numeric_capability(&name).unwrap();
                assert_eq!(c.encoding.as_ref().unwrap().bit_offset, Some(0));
                assert_eq!(
                    c.encoding.as_ref().unwrap().bit_precision,
                    Some(bits as u16)
                );
                if (endian == "LE") != cfg!(target_endian = "little") {
                    assert!(!c.capability.is_supported());
                    assert_eq!(
                        s.read_numeric(&name, None).unwrap_err().kind,
                        ErrorKind::Unsupported
                    );
                    continue;
                }
                assert!(c.capability.is_supported());
                let max = u64::MAX >> (64 - bits);
                let expected = vec![0, 1, 2, 3, 1u64 << (bits - 1), max];
                let a = s.read_numeric(&name, None).unwrap();
                assert_eq!(a.shape, [2, 3]);
                assert_eq!(unsigned(a.values), expected);
                assert_eq!(
                    unsigned(s.read_numeric_batch(&[&name]).unwrap().remove(0).1.values),
                    expected
                );
                let a = s
                    .read_numeric(&name, Some(&[Slice::new(0, 2, 1), Slice::new(0, 3, 2)]))
                    .unwrap();
                assert_eq!(a.shape, [2, 2]);
                assert_eq!(unsigned(a.values), [0, 2, 3, max]);
                let e = s
                    .read_numeric_bounded(&name, None, (bits / 8 * 6 - 1) as usize)
                    .unwrap_err();
                assert_eq!(e.kind, ErrorKind::Unsupported);
                assert_eq!(e.code, Some(UnsupportedCode::ResourceLimit));
            }
        }
    }
}
#[test]
fn shifted_and_reduced_precision_bitfields_refuse_numeric_decoding() {
    let s = session();
    for name in ["partial", "offset"] {
        assert!(!s
            .numeric_capability(name)
            .unwrap()
            .capability
            .is_supported());
        assert_eq!(
            s.read_numeric(name, None).unwrap_err().kind,
            ErrorKind::Unsupported
        );
    }
}

#[test]
fn stored_terrain_mask_keeps_its_existing_raw_byte_envelope() {
    use ras_commander_fast_hdf::hdf::{families::ReadFamily, stored::StoredValues};
    let s = session();
    let value = s
        .read_family_dataset(
            ReadFamily::Terrain,
            "Terrain/test/0/Mask",
            None,
            1 << 20,
            1 << 20,
        )
        .unwrap();
    match value {
        StoredValues::Bitfield { shape, raw_bytes } => {
            assert_eq!(shape, [2, 3]);
            assert_eq!(raw_bytes, [0, 1, 2, 3, 128, 255]);
        }
        _ => panic!("stored byte-bitfield envelope changed"),
    }
}
