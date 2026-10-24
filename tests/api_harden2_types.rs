#![cfg(feature = "hdf")]
use ras_commander_fast_hdf::{
    hdf::{AttributeType, ByteOrder, ReadOnlyHdfSession, StringCharset, StringPadding, TypeClass},
    Capability, UnsupportedCode,
};
#[test]
fn shared_outcome_contract() {
    let supported = Capability::Supported;
    assert!(supported.is_supported());
    assert_eq!(supported.reason(), None);
    let cap = Capability::unsupported(UnsupportedCode::Encoding, "unproven");
    assert_eq!(cap.reason(), Some("unproven"));
    assert!(matches!(
        cap,
        Capability::Unsupported {
            code: UnsupportedCode::Encoding,
            ..
        }
    ));
    for n in 0..=255 {
        let class = TypeClass::from_code(n);
        assert_eq!(class.code(), n);
        assert_eq!(TypeClass::from_code(class.code()), class);
    }
}
/// Frozen evidence directory: `var` when set, else the copy committed under
/// `tests/fixtures/<fixture>`. `None` (the test skips) when neither exists, so a
/// partial checkout never fails on missing inputs.
fn evidence_dir(var: &str, fixture: &str) -> Option<std::path::PathBuf> {
    let dir = std::env::var_os(var)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(fixture)
        });
    if dir.is_dir() {
        Some(dir)
    } else {
        eprintln!("skipping: evidence directory {} is absent", dir.display());
        None
    }
}
#[test]
fn attribute_types_come_from_raw_proof() {
    let Some(root) = evidence_dir("RAS_ENCODING_EVIDENCE", "encoding_evidence") else {
        return;
    };
    let root = root.join("malformed");
    for file in [
        "uint64.h5",
        "negative.h5",
        "fixed.h5",
        "reserved_padding.h5",
    ] {
        let s = ReadOnlyHdfSession::open(root.join(file)).unwrap();
        let a = s.typed_group_attributes("/").unwrap();
        assert_eq!(a.len(), 1);
        let t = &a[0].attribute_type;
        match file {
            "uint64.h5" => assert_eq!(
                *t,
                AttributeType::Int {
                    signed: false,
                    bytes: 8,
                    order: ByteOrder::Big
                }
            ),
            "negative.h5" => assert_eq!(
                *t,
                AttributeType::Int {
                    signed: true,
                    bytes: 8,
                    order: ByteOrder::Little
                }
            ),
            "fixed.h5" => assert_eq!(
                *t,
                AttributeType::FixedString {
                    charset: StringCharset::Ascii,
                    padding: StringPadding::NullPad,
                    width: 5
                }
            ),
            _ => assert!(matches!(
                t,
                AttributeType::Unsupported {
                    code: UnsupportedCode::Encoding,
                    ..
                }
            )),
        };
        assert!(!a[0].datatype_label.is_empty());
    }
}
