#![cfg(feature = "hdf")]
use ras_commander_fast_hdf::{
    hdf::{families::ReadFamily, ReadOnlyHdfSession},
    UnsupportedCode,
};
fn fixture(name: &str) -> ReadOnlyHdfSession {
    ReadOnlyHdfSession::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/wave3_review")
            .join(name),
    )
    .unwrap()
}
#[test]
fn oversized_message_does_not_hide_inventory_siblings() {
    let s = fixture("budget_messages.h5");
    for max in [10_000, 100_000, 400_000] {
        let inv = s
            .read_family_inventory(ReadFamily::PlanMetadata, max)
            .unwrap();
        let message = inv
            .datasets
            .iter()
            .find(|d| d.stored.path.ends_with("Compute Messages (rtf)"))
            .unwrap();
        assert_eq!(
            message.stored.capability.code(),
            Some(UnsupportedCode::ResourceLimit)
        );
        assert_eq!(message.stored.storage, "strings");
        assert!(
            inv.datasets
                .iter()
                .any(|d| d.stored.path.ends_with("Review Small")
                    && d.stored.capability.is_supported())
        );
        let blocks = s.output_block_inventory(max).unwrap();
        let block = blocks.iter().find(|b| b.name == "Review Copy").unwrap();
        assert_eq!(
            block
                .datasets
                .iter()
                .find(|d| d.path.ends_with("Compute Messages (rtf)"))
                .unwrap()
                .capability
                .code(),
            Some(UnsupportedCode::ResourceLimit)
        );
        assert!(block
            .datasets
            .iter()
            .any(|d| d.path.ends_with("/Small") && d.capability.is_supported()));
    }
}
#[test]
fn pipe_domains_are_explicit_even_with_coincident_counts() {
    let s = fixture("pipe_unknown.h5");
    let inv = s
        .read_family_inventory(ReadFamily::PipeResults, 1 << 20)
        .unwrap();
    let known = inv
        .datasets
        .iter()
        .find(|d| d.stored.path.ends_with("Nodes/Depth"))
        .unwrap();
    assert!(known.stored.capability.is_supported());
    assert_eq!(known.domain, "node");
    assert_eq!(
        known.identity_table.as_deref(),
        Some("Geometry/Pipe Networks/Davis/Node Indices")
    );
    assert_eq!(known.locations.len(), 133);
    let unknown = inv
        .datasets
        .iter()
        .find(|d| d.stored.path.ends_with("Maximum Node Depth"))
        .unwrap();
    assert_eq!(
        unknown.stored.capability.code(),
        Some(UnsupportedCode::IdentityLayout)
    );
    assert!(unknown.locations.is_empty());
    assert_eq!(
        s.read_family_dataset(
            ReadFamily::PipeResults,
            &unknown.stored.path,
            None,
            1 << 20,
            1 << 20
        )
        .unwrap_err()
        .code,
        Some(UnsupportedCode::IdentityLayout)
    );
    for (f, p) in [
        (
            ReadFamily::PipeResults,
            "Results/Unsteady/Summary/Pipe Network/Unknown",
        ),
        (ReadFamily::PipeGeometry, "Geometry/Pipe Inlets/Unknown"),
    ] {
        assert_eq!(
            s.read_family_dataset(f, p, None, 1 << 20, 1 << 20)
                .unwrap_err()
                .code,
            Some(UnsupportedCode::Schema)
        );
    }
}
#[test]
fn reference_placeholder_has_literal_values_without_snapshot_meaning() {
    use ras_commander_fast_hdf::hdf::{stored::StoredValues, NumericValues};
    let s = fixture("reference_literal.h5");
    let inv = s
        .read_family_inventory(ReadFamily::ReferenceLines, 1 << 20)
        .unwrap();
    let d = inv
        .datasets
        .iter()
        .find(|d| d.stored.path.ends_with("Water Surface Max"))
        .unwrap();
    assert_eq!(d.role, "one_row_unqualified");
    assert_eq!(d.time_group, None);
    assert_eq!(d.time_of_extreme_row, None);
    let StoredValues::Numeric(a) = s
        .read_family_dataset(
            ReadFamily::ReferenceLines,
            &d.stored.path,
            None,
            1 << 20,
            1 << 20,
        )
        .unwrap()
    else {
        panic!("numeric")
    };
    assert_eq!(a.shape, [1, 1]);
    match a.values {
        NumericValues::F32(v) => assert_eq!(v[0].to_bits(), 0),
        _ => panic!("stored f32 required"),
    }
}
#[test]
fn dss_time_checks_units_order_and_stamp_lengths() {
    use ras_commander_fast_hdf::ErrorKind;
    let path="Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/Pipe Networks/Davis/Nodes/Depth";
    for name in ["pipe_valid.h5", "pipe_duplicate_time.h5"] {
        assert!(
            fixture(name)
                .read_family_dataset(ReadFamily::PipeResults, path, None, 1 << 20, 1 << 20)
                .is_ok(),
            "{name}"
        );
    }
    for name in ["pipe_decreasing.h5", "pipe_bad_stamp.h5"] {
        assert_eq!(
            fixture(name)
                .read_family_dataset(ReadFamily::PipeResults, path, None, 1 << 20, 1 << 20)
                .unwrap_err()
                .kind,
            ErrorKind::Corrupt,
            "{name}"
        );
    }
    assert_eq!(
        fixture("pipe_bad_units.h5")
            .read_family_dataset(ReadFamily::PipeResults, path, None, 1 << 20, 1 << 20)
            .unwrap_err()
            .code,
        Some(UnsupportedCode::Schema)
    );
}
