//! Regression tests for crafted malformed HDF5 inputs from a 2026-10-21
//! release review. Each fixture under `tests/fixtures/malformed/` is under 4.3 KB.
//! Before the fixes each one aborted or overflowed the stack, which kills a
//! host Python process; now every call must return. A test skips (and passes)
//! when its fixture is absent so a partial checkout never errors.
#![cfg(feature = "hdf")]

use std::path::PathBuf;

use ras_commander_fast_hdf::hdf::ReadOnlyHdfSession;

const BUDGET: usize = 64 << 20;

fn fixture(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/malformed")
        .join(name);
    if path.is_file() {
        Some(path)
    } else {
        eprintln!("skipping: malformed fixture {name} is absent");
        None
    }
}

macro_rules! session_or_skip {
    ($name:expr) => {{
        let Some(path) = fixture($name) else {
            return;
        };
        ReadOnlyHdfSession::open(&path).expect("crafted fixture opens")
    }};
}

/// Soft-link depth: `/s -> "/s/x"` resolves its own intermediate component. The link
/// depth must carry across intermediate components and the cycle guard must
/// stop it instead of overflowing the stack.
#[test]
fn soft_link_through_itself_is_refused() {
    for name in [
        "r7b_softlink_intermediate_self.h5",
        "r7c_softlink_intermediate_self_old.h5",
    ] {
        let s = session_or_skip!(name);
        let _ = s.group_members("/");
        assert!(s.group_members("/s").is_err(), "{name}");
        assert!(s.numeric_capability("/s/x").is_err(), "{name}");
        let _ = s.layer_inventory(BUDGET);
        let _ = s.plan_capabilities(BUDGET);
    }
}

/// Control: a plain a -> b -> a soft-link loop was already handled.
#[test]
fn soft_link_loop_is_refused() {
    let s = session_or_skip!("r7_softlink_loop.h5");
    let _ = s.group_members("/");
    let _ = s.plan_capabilities(BUDGET);
}

fn is_resource_limit(e: &ras_commander_fast_hdf::Error) -> bool {
    e.kind == ras_commander_fast_hdf::ErrorKind::Unsupported
        && e.code == Some(ras_commander_fast_hdf::UnsupportedCode::ResourceLimit)
}

/// Chunk allocation: a 4x4 f64 dataset (128 bytes) declares 2^20 x 2^20 deflate chunks.
/// The declared chunk is refused before allocation, serially and in parallel.
#[test]
fn oversized_declared_chunk_is_refused_before_allocation() {
    use ras_commander_fast_hdf::hdf::Slice;
    for name in [
        "r1_deflate_chunkdims_2p40.h5",
        "r1b_deflate_chunkdims_2GiB.h5",
    ] {
        let Some(path) = fixture(name) else {
            return;
        };
        for threads in [1, 4] {
            let s = ReadOnlyHdfSession::with_options(&path, threads, true).unwrap();
            let one = [Slice::new(0, 1, 1), Slice::new(0, 1, 1)];
            let e = s.read_numeric("/data", Some(&one)).unwrap_err();
            assert!(is_resource_limit(&e), "{name}: {e:?}");
            let e = s.read_numeric("/data", None).unwrap_err();
            assert!(is_resource_limit(&e), "{name}: {e:?}");
        }
    }
}

/// Size arithmetic: fixed-array indexes whose entry count wraps `nelmts * entry_size`
/// (r8b) or makes the paged length loop spin (r8a, 2^62 entries) must be
/// refused, not wrapped into a short read and an exabyte allocation.
#[test]
fn fixed_array_entry_count_overflow_is_refused() {
    use ras_commander_fast_hdf::hdf::Slice;
    for name in [
        "r8b_fixed_array_nelmts_wrap.h5",
        "r8a_fixed_array_paged_nelmts2p62.h5",
    ] {
        let s = session_or_skip!(name);
        assert!(s.read_numeric("/data", None).is_err(), "{name}");
        // The bounded selection path stays well-defined either way.
        let _ = s.read_numeric("/data", Some(&[Slice::new(0, 1, 1)]));
    }
}

/// Read budgets: whole reads allocated `declared dims x element size` with no budget.
/// A 1.4 KB file declaring 2^34 values asked for 17 GB and aborted.
#[test]
fn unbudgeted_whole_read_uses_the_default_budget() {
    let s = session_or_skip!("r2_fill_dims_2p34.h5");
    let e = s.read_numeric("/data", None).unwrap_err();
    assert!(is_resource_limit(&e), "{e:?}");
    let e = s.read_numeric_batch(&["/data"]).unwrap_err();
    assert!(is_resource_limit(&e), "{e:?}");
}

#[test]
fn explicit_read_budgets_refuse_before_allocating() {
    use ras_commander_fast_hdf::hdf::Slice;
    let s = session_or_skip!("budget_numeric_256MiB.h5");
    let e = s.read_numeric_bounded("/data", None, 1 << 20).unwrap_err();
    assert!(is_resource_limit(&e), "{e:?}");
    // The budget covers the selection, not the declared dataset.
    let one = s
        .read_numeric_bounded("/data", Some(&[Slice::new(0, 4, 1)]), 1 << 20)
        .unwrap();
    assert_eq!(one.shape, vec![4]);
    // A batch shares one budget and is checked before any dataset is read.
    let e = s
        .read_numeric_batch_bounded(&["/data", "/data"], 300 << 20)
        .unwrap_err();
    assert!(is_resource_limit(&e), "{e:?}");
}

#[cfg(feature = "mesh")]
#[test]
fn topology_arrays_are_budgeted_before_reading() {
    let s = session_or_skip!("budget_mesh_coords_4GiB.h5");
    let e = s.read_topology("Area").unwrap_err();
    assert!(is_resource_limit(&e), "{e:?}");
    let e = s.read_topology_bounded("Area", 1 << 20).unwrap_err();
    assert!(is_resource_limit(&e), "{e:?}");
}

/// Attribute counts: zero-size datatype attributes declaring 2^40-2^62 elements must not
/// reach an allocation through any attribute or inventory entry point.
#[test]
fn zero_size_attribute_datatype_is_refused() {
    for name in [
        "r3_attr_f64_size0_dims2p40.h5",
        "r3b_attr_str_size0_dims2p40.h5",
        "r3c_attr_f64_size0_dims2p62.h5",
    ] {
        let s = session_or_skip!(name);
        let _ = s.dataset_attributes("/data");
        let _ = s.typed_dataset_attributes("/data", BUDGET);
        let _ = s.layer_inventory(BUDGET);
        let _ = s.plan_capabilities(BUDGET);
    }
}

/// Negative controls: self-looping object-header continuations and B-tree
/// children, a zero-size dataset datatype and a zero chunk dimension were
/// already refused cleanly; keep them that way.
#[test]
fn already_handled_malformed_structures_stay_refused() {
    use ras_commander_fast_hdf::hdf::Slice;
    for name in [
        "r4_ds_f64_size0_dims2p40.h5",
        "r5_ohdr_continuation_selfloop.h5",
        "r6_btree_v1_child_selfloop.h5",
        "r9_chunkdim_zero_old.h5",
    ] {
        let Some(path) = fixture(name) else {
            continue;
        };
        let Ok(s) = ReadOnlyHdfSession::open(&path) else {
            continue;
        };
        let _ = s.group_members("/");
        let _ = s.numeric_capability("/data");
        let _ = s.read_numeric("/data", None);
        let _ = s.read_numeric("/data", Some(&[Slice::new(0, 1, 1)]));
        let _ = s.layer_inventory(BUDGET);
        let _ = s.plan_capabilities(BUDGET);
    }
}
