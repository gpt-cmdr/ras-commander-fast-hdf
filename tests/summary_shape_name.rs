#![cfg(all(feature = "hdf", feature = "arrow"))]
//! Summary Output arrays without a stored `Location` (`DomainRule::SummaryShapeAndName`), on
//! the synthetic `fixtures/summary_shape_name` files (`generate.py`): the domain is
//! ras-commander's name rule (`Face` in the name), proven by the width against the stored cell
//! and face counts; equal counts, a contradicting width and an unlabelled row 1 stay refused.
use arrow_array::cast::AsArray;
use arrow_array::types::{Float32Type, Int64Type};
use ras_commander_fast_hdf::arrow_layers::LayerOptions;
use ras_commander_fast_hdf::hdf::mesh_results::{DomainRule, ElementDomain, MeshResultVariable};
use ras_commander_fast_hdf::hdf::{NumericValues, ReadOnlyHdfSession};
use ras_commander_fast_hdf::{Capability, ErrorKind, UnsupportedCode};

fn session(name: &str) -> ReadOnlyHdfSession {
    ReadOnlyHdfSession::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/summary_shape_name")
            .join(name),
    )
    .unwrap()
}

fn variable(s: &ReadOnlyHdfSession, name: &str) -> MeshResultVariable {
    s.mesh_summary_inventory("A", 1 << 20)
        .unwrap()
        .variables
        .into_iter()
        .find(|v| v.name == name)
        .unwrap_or_else(|| panic!("{name} absent"))
}

#[test]
fn names_and_widths_qualify_the_observed_layouts() {
    let s = session("valid.h5");
    for (name, domain, timed) in [
        ("Maximum Face Velocity", ElementDomain::Face, true), // 6.1 `Rows`/`Units`
        ("Maximum Cell Courant", ElementDomain::Cell, true),
        ("Maximum Water Surface", ElementDomain::Cell, true),
        ("Maximum Face Shear Stress", ElementDomain::Face, true), // 5.0 row labels
        ("Cell Last Iteration", ElementDomain::Cell, false),      // rank one
    ] {
        let v = variable(&s, name);
        assert_eq!(v.capability, Capability::Supported, "{name}");
        assert_eq!(
            (v.domain, v.domain_rule),
            (domain, DomainRule::SummaryShapeAndName),
            "{name}"
        );
        assert_eq!(v.domain_rule.as_str(), "summary_shape_and_name");
        assert_eq!(v.time_of_extreme_row, timed.then_some(1), "{name}");
        // Values are the literal dataset.
        let typed = s
            .read_mesh_summary_values("A", name, None, 1 << 20, 1 << 20)
            .unwrap();
        assert_eq!(typed, s.read_numeric(&v.path, None).unwrap(), "{name}");
    }
}

#[test]
fn unproven_layouts_stay_refused() {
    let s = session("valid.h5");
    for name in [
        "Cell Cumulative Iter Lookup", // (1, 2): not an element array
        "Starting Differences WSE",    // three rows
        "Maximum Face Courant",        // row 1 labelled `Count`, not `Time`
        "Minimum Face Velocity",       // named for faces, as wide as the cells
        "Cell Volume Error",           // width matches no count
    ] {
        let v = variable(&s, name);
        assert_eq!(v.domain_rule, DomainRule::Unqualified, "{name}");
        assert_eq!(
            v.capability.code(),
            Some(UnsupportedCode::IdentityLayout),
            "{name}: {:?}",
            v.capability
        );
        assert_eq!(v.time_of_extreme_row, None, "{name}");
    }
    let Capability::Unsupported { reason, .. } = variable(&s, "Minimum Face Velocity").capability
    else {
        panic!()
    };
    assert!(reason.contains("width 4 differs from the face count 6"), "{reason}");
}

#[test]
fn equal_cell_and_face_counts_are_ambiguous_and_refused() {
    let s = session("equal_counts.h5");
    for name in ["Maximum Face Velocity", "Cell Last Iteration"] {
        let v = variable(&s, name);
        assert_eq!(v.domain_rule, DomainRule::Unqualified, "{name}");
        let Capability::Unsupported { code, reason } = &v.capability else {
            panic!("{name}: {:?}", v.capability)
        };
        assert_eq!(*code, UnsupportedCode::IdentityLayout);
        assert!(reason.contains("cell and face counts are equal (4)"), "{reason}");
        let e = s
            .mesh_summary_layer(name, &LayerOptions::default())
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Unsupported, "{name}");
    }
}

/// The Arrow layers are ras-commander's `get_mesh_summary_output(include_geometry=False)`: cell
/// rows without the ghost cell, the value row, row 1 as time, integers as Int64.
#[test]
fn arrow_layers_read_the_qualified_summaries() {
    let s = session("valid.h5");
    let faces = s
        .mesh_summary_layer("Maximum Face Velocity", &LayerOptions::default())
        .unwrap();
    assert_eq!(faces.identity, ("mesh_name".into(), Some("face_id".into())));
    assert_eq!(faces.num_rows(), 6);
    let b = &faces.batches[0];
    let NumericValues::F32(stored) = s
        .read_numeric(
            "Results/Unsteady/Output/Output Blocks/Base Output/Summary Output/2D Flow Areas/A/Maximum Face Velocity",
            None,
        )
        .unwrap()
        .values
    else {
        panic!()
    };
    assert_eq!(b.column(2).as_primitive::<Float32Type>().values(), &stored[..6]);
    assert_eq!(b.schema().field(3).name(), "maximum_face_velocity_time");

    let cells = s
        .mesh_summary_layer("Cell Last Iteration", &LayerOptions::default())
        .unwrap();
    assert_eq!(cells.num_rows(), 3, "the ghost cell is left out");
    let b = &cells.batches[0];
    assert_eq!(b.column(1).as_primitive::<Int64Type>().values(), &[0, 1, 2]);
    assert_eq!(b.column(2).as_primitive::<Int64Type>().values(), &[0, 3, 1]);

    let shear = s
        .mesh_summary_layer("Maximum Face Shear Stress", &LayerOptions::default())
        .unwrap();
    assert_eq!(shear.num_rows(), 6);
}
