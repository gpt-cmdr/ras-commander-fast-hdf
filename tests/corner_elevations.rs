//! 2D mesh corner ground elevations (`ReadFamily::MeshCornerElevations`).
//! Synthetic fixtures reproduce the stored schema observed in HEC-RAS 7.0/7.0.1 geometry and
//! plan HDFs with invented values: `tests/generate_corner_elevation_fixtures.py`. Qualified
//! arrays must return exactly the bytes h5py returns for `dataset[()]` (NaN payloads, signed
//! zero, infinities, subnormals and sentinels included); every other layout is refused.
#![cfg(feature = "hdf")]
use ras_commander_fast_hdf::{
    hdf::{
        corner_elevations::{
            CornerCountSource, CornerElevationLayout, CACHE_DATASET, CACHE_ROOT, GEOMETRY_ROOT,
        },
        families::ReadFamily,
        stored::StoredValues,
        NumericValues, ReadOnlyHdfSession, Slice,
    },
    Capability, ErrorKind, UnsupportedCode,
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MAX: usize = 1 << 20;
const FAMILY: ReadFamily = ReadFamily::MeshCornerElevations;
const GEOMETRY: CornerElevationLayout = CornerElevationLayout::Geometry;
const CACHE: CornerElevationLayout = CornerElevationLayout::MappingCache;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/corner_elevations")
}

fn manifest() -> Vec<serde_json::Value> {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir().join("manifest.json")).unwrap()).unwrap();
    m.as_array().unwrap().clone()
}

fn open(name: &str) -> ReadOnlyHdfSession {
    let entry = manifest().into_iter().find(|e| e["name"] == name).unwrap();
    let path = dir().join(name);
    assert_eq!(
        format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap())),
        entry["sha256"].as_str().unwrap(),
        "{}",
        path.display()
    );
    ReadOnlyHdfSession::open(path).unwrap()
}

fn le_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn status(c: &Capability) -> (&'static str, Option<&'static str>) {
    (c.as_str(), c.code().map(|c| c.as_str()))
}

#[test]
fn inventory_matches_the_manifest_and_qualified_reads_match_h5py_bytes() {
    for entry in manifest() {
        let name = entry["name"].as_str().unwrap();
        let s = open(name);
        let inv = s.read_family_inventory(FAMILY, MAX).unwrap();
        assert!(inv.capability.is_supported(), "{name}");
        let expected = entry["datasets"].as_array().unwrap();
        let paths: Vec<_> = inv
            .datasets
            .iter()
            .map(|d| d.stored.path.as_str())
            .collect();
        let want: Vec<_> = expected
            .iter()
            .map(|d| d["path"].as_str().unwrap())
            .collect();
        assert_eq!(paths, want, "{name}");
        for (d, e) in inv.datasets.iter().zip(expected) {
            let (st, code) = status(&d.stored.capability);
            assert_eq!(st, e["status"].as_str().unwrap(), "{}", d.stored.path);
            assert_eq!(code, e["code"].as_str(), "{}", d.stored.path);
            assert_eq!(d.domain, "mesh_corner");
            let cache = d.stored.path.starts_with(CACHE_ROOT);
            assert_eq!(
                d.role,
                if cache {
                    "mapping_cache_corner_elevation"
                } else {
                    "geometry_corner_elevation"
                }
            );
            let read = s.read_family_dataset(FAMILY, &d.stored.path, None, MAX, MAX);
            if st != "supported" {
                let err = read.expect_err(&d.stored.path);
                assert_eq!(
                    err.kind,
                    if st == "corrupt" {
                        ErrorKind::Corrupt
                    } else {
                        ErrorKind::Unsupported
                    },
                    "{}: {err}",
                    d.stored.path
                );
                continue;
            }
            let mesh = d.stored.path.split('/').nth(2).unwrap();
            assert_eq!(
                d.identity_table.as_deref(),
                Some(format!("{GEOMETRY_ROOT}/{mesh}/FacePoints Coordinate").as_str())
            );
            assert_eq!(d.stored.shape, [e["corners"].as_u64().unwrap()]);
            let StoredValues::Numeric(a) = read.unwrap() else {
                panic!("numeric")
            };
            let NumericValues::F32(v) = a.values else {
                panic!("f32")
            };
            assert_eq!(e["h5py_dtype"], "<f4");
            assert_eq!(hex(&le_bytes(&v)), e["h5py_hex"].as_str().unwrap());
        }
    }
}

#[test]
fn typed_reader_preserves_every_bit_and_reports_provenance() {
    let s = open("geometry.h5");
    let r = s
        .mesh_corner_elevations("Mesh A", GEOMETRY, None, MAX)
        .unwrap();
    assert!(r.is_supported());
    let c = r.into_value().unwrap();
    let raw: Vec<u32> = c.values.iter().map(|v| v.to_bits()).collect();
    assert_eq!(
        raw,
        [
            0x7FC0_0001,
            0x8000_0000,
            0x7F80_0000,
            0xFF80_0000,
            0x0000_0001,
            0xC61C_3C00,
            0x42C8_0000,
            0x447A_0000
        ]
    );
    assert_eq!(c.values[5], -9999.0);
    assert_eq!(c.mesh, "Mesh A");
    assert_eq!(c.layout, GEOMETRY);
    assert_eq!(c.corner_count, 8);
    assert_eq!(
        c.corner_count_source,
        CornerCountSource::Coordinates {
            path: format!("{GEOMETRY_ROOT}/Mesh A/FacePoints Coordinate")
        }
    );
    assert_eq!(
        c.dataset.path,
        format!("{GEOMETRY_ROOT}/Mesh A/FacePoints Elevation")
    );
    assert_eq!(c.dataset.id.source, dir().join("geometry.h5"));
    assert!(c.dataset.attributes.iter().any(|a| a.name == "Can Plot"));
    // A caller count that agrees is accepted; one that disagrees is a corrupt pairing.
    assert!(s
        .mesh_corner_elevations("Mesh A", GEOMETRY, Some(8), MAX)
        .unwrap()
        .is_supported());
    let r = s
        .mesh_corner_elevations("Mesh A", GEOMETRY, Some(9), MAX)
        .unwrap();
    assert_eq!(status(r.capability()), ("corrupt", None));
    assert!(r
        .capability()
        .reason()
        .unwrap()
        .contains("not the expected 9"));
    // Contiguous storage reads identically.
    let c = s
        .mesh_corner_elevations("Mesh C", GEOMETRY, None, MAX)
        .unwrap()
        .into_value()
        .unwrap();
    assert_eq!(c.values, [10.5, 11.25, 12.0]);
    // Selections on the family read are exact sub-arrays.
    let StoredValues::Numeric(a) = s
        .read_family_dataset(
            FAMILY,
            &format!("{GEOMETRY_ROOT}/Mesh A/FacePoints Elevation"),
            Some(&[Slice::new(5, 8, 1)]),
            MAX,
            MAX,
        )
        .unwrap()
    else {
        panic!("numeric")
    };
    assert_eq!(a.values, NumericValues::F32(vec![-9999.0, 100.0, 1000.0]));
}

#[test]
fn missing_data_is_unavailable_and_requests_are_validated() {
    let s = open("geometry.h5");
    for (mesh, layout) in [
        ("Mesh B", GEOMETRY),
        ("No Such Mesh", GEOMETRY),
        ("Mesh A", CACHE),
    ] {
        let r = s.mesh_corner_elevations(mesh, layout, None, MAX).unwrap();
        assert_eq!(status(r.capability()), ("unavailable", None), "{mesh}");
        assert!(r.value().is_none());
    }
    for bad in ["", ".", "..", "a/b", "a\\b"] {
        let e = s
            .mesh_corner_elevations(bad, GEOMETRY, None, MAX)
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidRequest, "{bad:?}");
    }
    for bad in [
        "Geometry/2D Flow Areas/Mesh A/FacePoints Coordinate",
        "Geometry/2D Flow Areas/Mesh A/FacePoint Elevation",
        "Geometry/2D Flow Areas/../FacePoints Elevation",
    ] {
        let e = s
            .read_family_dataset(FAMILY, bad, None, MAX, MAX)
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidRequest, "{bad}");
    }
    let e = s
        .read_family_dataset(
            FAMILY,
            "Geometry/2D Flow Areas/Mesh B/FacePoints Elevation",
            None,
            MAX,
            MAX,
        )
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
    // A budget below the array refuses before allocating.
    let r = s
        .mesh_corner_elevations("Mesh A", GEOMETRY, None, 16)
        .unwrap_err();
    assert_eq!(r.code, Some(UnsupportedCode::ResourceLimit));
}

#[test]
fn files_without_corner_elevations_report_the_family_unavailable() {
    let path = dir().join("../terrain_modifications/valid.h5");
    let s = ReadOnlyHdfSession::open(path).unwrap();
    let inv = s.read_family_inventory(FAMILY, MAX).unwrap();
    assert!(inv.datasets.is_empty());
    assert_eq!(status(&inv.capability), ("unavailable", None));
}

#[test]
fn a_cache_is_qualified_only_with_the_plan_corner_count() {
    let s = open("cache.h5");
    let r = s
        .mesh_corner_elevations("Mesh A", CACHE, None, MAX)
        .unwrap();
    assert_eq!(
        status(r.capability()),
        ("unsupported", Some("identity_layout"))
    );
    assert!(r.capability().reason().unwrap().contains("corner count"));
    let r = s
        .mesh_corner_elevations("Mesh A", CACHE, Some(9), MAX)
        .unwrap();
    assert_eq!(status(r.capability()), ("corrupt", None));
    assert!(r
        .capability()
        .reason()
        .unwrap()
        .contains("stores 8 corner elevations for 9 corners"));
    let c = s
        .mesh_corner_elevations("Mesh A", CACHE, Some(8), MAX)
        .unwrap()
        .into_value()
        .unwrap();
    assert_eq!(c.layout, CACHE);
    assert_eq!(c.corner_count_source, CornerCountSource::Caller);
    let geometry = open("geometry.h5")
        .mesh_corner_elevations("Mesh A", GEOMETRY, None, MAX)
        .unwrap()
        .into_value()
        .unwrap();
    let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&c.values), bits(&geometry.values));
    // The geometry layout of the same mesh is simply absent from a cache file.
    let r = s
        .mesh_corner_elevations("Mesh A", GEOMETRY, None, MAX)
        .unwrap();
    assert_eq!(status(r.capability()), ("unavailable", None));
    assert_eq!(
        CACHE.dataset_path("Mesh A").unwrap(),
        format!("{CACHE_ROOT}/Mesh A/{CACHE_DATASET}")
    );
}

#[test]
fn refused_layouts_name_their_defect() {
    let s = open("refusals.h5");
    for (mesh, want, text) in [
        (
            "Length Mismatch",
            ("corrupt", None),
            "stores 3 corner elevations for 4 corners",
        ),
        (
            "Wide Float",
            ("unsupported", Some("encoding")),
            "little-endian f4",
        ),
        (
            "Big Endian",
            ("unsupported", Some("encoding")),
            "byte order",
        ),
        (
            "Integer",
            ("unsupported", Some("encoding")),
            "little-endian f4",
        ),
        (
            "Rank Two",
            ("unsupported", Some("schema")),
            "one-dimensional",
        ),
        ("Scalar", ("unsupported", Some("schema")), "one-dimensional"),
        (
            "No Coordinates",
            ("corrupt", None),
            "no FacePoints Coordinate",
        ),
        ("Wide Coordinates", ("corrupt", None), "shape (corners, 2)"),
    ] {
        let r = s.mesh_corner_elevations(mesh, GEOMETRY, None, MAX).unwrap();
        assert_eq!(status(r.capability()), want, "{mesh}");
        let reason = r.capability().reason().unwrap();
        assert!(reason.contains(text), "{mesh}: {reason}");
    }
    assert!(s
        .mesh_corner_elevations("Valid", GEOMETRY, None, MAX)
        .unwrap()
        .is_supported());
}

/// Real-file check against an h5py oracle written by `tests/corner_elevation_oracle.py`:
/// every listed mesh array must read natively, with h5py's exact bytes and length.
#[test]
#[ignore = "requires real HEC-RAS files; run with RAS_CORNER_ELEVATION_ORACLE"]
fn real_files_match_the_h5py_oracle() {
    let oracle: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("RAS_CORNER_ELEVATION_ORACLE").unwrap()).unwrap(),
    )
    .unwrap();
    let mut checked = 0;
    for f in oracle["files"].as_array().unwrap() {
        let s = ReadOnlyHdfSession::open(Path::new(f["path"].as_str().unwrap())).unwrap();
        let inv = s.read_family_inventory(FAMILY, 64 << 20).unwrap();
        for m in f["arrays"].as_array().unwrap() {
            let mesh = m["mesh"].as_str().unwrap();
            let layout = match m["layout"].as_str().unwrap() {
                "geometry" => GEOMETRY,
                _ => CACHE,
            };
            let expected = m["corners"].as_u64();
            let r = s
                .mesh_corner_elevations(mesh, layout, expected, 64 << 20)
                .unwrap();
            let want = m["native_status"].as_str().unwrap_or("supported");
            assert_eq!(r.capability().as_str(), want, "{} {mesh}", f["path"]);
            let row = inv
                .datasets
                .iter()
                .find(|d| d.stored.path == layout.dataset_path(mesh).unwrap())
                .unwrap();
            if want != "supported" {
                continue;
            }
            // A cache file stores no corner coordinates: its inventory row cannot prove the
            // corner count, so only the typed read with the plan's count qualifies it.
            if layout == CACHE {
                assert_eq!(
                    status(&row.stored.capability),
                    ("unsupported", Some("identity_layout"))
                );
            } else {
                assert!(row.stored.capability.is_supported());
            }
            let c = r.into_value().unwrap();
            assert_eq!(c.values.len() as u64, m["corners"].as_u64().unwrap());
            assert_eq!(
                format!("{:x}", Sha256::digest(le_bytes(&c.values))),
                m["h5py_sha256"].as_str().unwrap(),
                "{} {mesh}",
                f["path"]
            );
            checked += 1;
        }
    }
    assert!(checked > 0);
    eprintln!("{checked} real corner-elevation arrays identical to h5py");
}
