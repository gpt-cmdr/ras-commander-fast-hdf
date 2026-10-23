//! Reader-level regression tests for crafted malformed files from the
//! 2026-10-21 ras-commander-rust release review. The fixtures live with the
//! engine crate; a test skips (and passes) when its fixture is absent, so this
//! crate still tests cleanly when vendored on its own.

use std::path::PathBuf;

use hdf5_reader::error::Error;
use hdf5_reader::Hdf5File;

fn fixture(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/ras-commander-rust/tests/fixtures/malformed")
        .join(name);
    if path.is_file() {
        Some(path)
    } else {
        eprintln!("skipping: malformed fixture {name} is absent");
        None
    }
}

fn is_resource_limit(err: &Error) -> bool {
    match err {
        Error::Context { source, .. } => is_resource_limit(source),
        Error::ResourceLimit(_) => true,
        _ => false,
    }
}

macro_rules! file_or_skip {
    ($name:expr) => {{
        let Some(path) = fixture($name) else {
            return;
        };
        Hdf5File::open(&path).expect("crafted fixture opens")
    }};
}

#[test]
fn soft_link_through_itself_does_not_recurse() {
    for name in [
        "r7b_softlink_intermediate_self.h5",
        "r7c_softlink_intermediate_self_old.h5",
    ] {
        let file = file_or_skip!(name);
        let root = file.root_group().unwrap();
        let _ = root.members();
        let _ = root.member_names();
        assert!(root.group("s").is_err(), "{name}");
        assert!(file.dataset("/s/x").is_err(), "{name}");
        assert!(file.group("/s/x").is_err(), "{name}");
    }
}

#[test]
fn soft_link_loop_does_not_recurse() {
    let file = file_or_skip!("r7_softlink_loop.h5");
    let root = file.root_group().unwrap();
    let _ = root.members();
    let _ = root.groups();
}

#[test]
fn oversized_declared_chunk_is_refused_before_allocation() {
    for name in [
        "r1_deflate_chunkdims_2p40.h5",
        "r1b_deflate_chunkdims_2GiB.h5",
    ] {
        let file = file_or_skip!(name);
        let ds = file.dataset("/data").unwrap();
        let err = ds.read_array::<f64>().unwrap_err();
        assert!(is_resource_limit(&err), "{name}: {err}");
    }
}

#[test]
fn chunk_limit_is_configurable() {
    let Some(path) = fixture("r1_deflate_chunkdims_2p40.h5") else {
        return;
    };
    // An explicit limit above the format maximum is clamped to 4 GiB, which
    // still refuses this 8 TiB chunk.
    let options = hdf5_reader::OpenOptions {
        max_chunk_bytes: usize::MAX,
        ..Default::default()
    };
    let file = Hdf5File::open_with_options(&path, options).unwrap();
    let err = file
        .dataset("/data")
        .unwrap()
        .read_array::<f64>()
        .unwrap_err();
    assert!(is_resource_limit(&err), "{err}");
}

#[test]
fn fixed_array_entry_count_overflow_is_refused() {
    for name in [
        "r8b_fixed_array_nelmts_wrap.h5",
        "r8a_fixed_array_paged_nelmts2p62.h5",
    ] {
        let file = file_or_skip!(name);
        let ds = file.dataset("/data").unwrap();
        assert!(ds.read_array::<u8>().is_err(), "{name}");
    }
}

/// A zero-size datatype lets an attribute declare 2^40 (r3, r3b) or 2^62
/// (r3c) elements with no data; decoding must refuse instead of reserving
/// terabytes or overflowing the capacity.
#[test]
fn zero_size_attribute_datatype_is_refused_before_allocation() {
    for name in [
        "r3_attr_f64_size0_dims2p40.h5",
        "r3b_attr_str_size0_dims2p40.h5",
        "r3c_attr_f64_size0_dims2p62.h5",
    ] {
        let file = file_or_skip!(name);
        let ds = file.dataset("/data").unwrap();
        let attrs = ds.attributes();
        assert!(!attrs.is_empty(), "{name}");
        for attr in attrs {
            let _ = attr.num_elements();
            let declared = attr.num_elements().unwrap_or(u64::MAX);
            if declared > 1 << 30 {
                assert!(attr.read_1d::<f64>().is_err(), "{name}");
            }
            let _ = attr.read_1d::<f64>();
            let _ = attr.read_strings();
        }
    }
}
