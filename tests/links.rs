#![cfg(feature = "hdf")]
//! Review P3-6: soft links to datasets are followed in old-style (symbol-table) and new-style
//! groups, including the root group. Dangling soft links and (never followed) external links
//! read as absent; a link to a group is a group path. Fixture: `generate_links_fixture.py`.
use ras_commander_fast_hdf::{hdf::ReadOnlyHdfSession, ErrorKind};
#[test]
fn soft_linked_datasets_resolve_in_old_and_new_style_groups() {
    for lib in ["earliest", "latest"] {
        let s = ReadOnlyHdfSession::open(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("tests/fixtures/links/links_{lib}.h5")),
        )
        .unwrap();
        let direct = s.read_numeric("g/d", None).unwrap();
        for p in ["g/soft", "soft_root"] {
            assert_eq!(s.read_numeric(p, None).unwrap(), direct, "{lib} {p}");
            let attrs = s.typed_dataset_attributes(p, 1 << 20).unwrap();
            assert!(attrs.is_supported(), "{lib} {p}");
            assert_eq!(attrs.value().unwrap()[0].name, "u");
        }
        let raw = s.raw_fixed_strings("g/soft_str", 1 << 20).unwrap();
        assert_eq!(raw.value().unwrap().raw_bytes, b"ab\0\0".to_vec(), "{lib}");
        for p in ["g/dangling", "g/ext"] {
            assert!(
                s.typed_dataset_attributes(p, 1 << 20)
                    .unwrap()
                    .is_unavailable(),
                "{lib} {p}"
            );
        }
        assert_eq!(
            s.typed_dataset_attributes("g/soft_group", 1 << 20)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidRequest
        );
        let root = s.group_members("/").unwrap();
        assert!(
            root.datasets.iter().any(|n| n == "soft_root"),
            "{lib} {root:?}"
        );
    }
}
