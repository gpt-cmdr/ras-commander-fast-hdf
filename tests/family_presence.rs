#![cfg(all(feature = "hdf", feature = "mesh"))]
//! `family_presence` gives the verdict of each family's full reader, from metadata only.
//!
//! The full-read verdict is the one a vector driver lists layers by: stored unless the reader
//! reports canonical absence (`Reported::Unavailable`) or `NotFound`; host errors propagate.
//! Synthetic cases (generator: tests/generate_presence_fixtures.py) have expected verdicts; every
//! committed HDF5 fixture, and every file below `RAS_PRESENCE_CORPUS` when set, is compared.
use ras_commander_fast_hdf::hdf::lines::GeometryFamily;
use ras_commander_fast_hdf::hdf::presence::{FamilyPresence, LayerFamily, LAYER_FAMILIES};
use ras_commander_fast_hdf::hdf::xs_geometry::OneDLineFamily;
use ras_commander_fast_hdf::hdf::{ReadOnlyHdfSession, DEFAULT_MAX_BYTES};
use ras_commander_fast_hdf::{ErrorKind, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn host(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::Io | ErrorKind::Internal | ErrorKind::FileNotFound
    )
}

/// The full-read verdict: `Ok(true)` when stored.
fn read_verdict<T>(r: Result<ras_commander_fast_hdf::Reported<T>>) -> Result<bool> {
    verdict(r.map(|r| !r.is_unavailable()))
}
fn verdict(r: Result<bool>) -> Result<bool> {
    match r {
        Ok(stored) => Ok(stored),
        Err(e) if e.kind == ErrorKind::NotFound => Ok(false),
        Err(e) if host(e.kind) => Err(e),
        Err(_) => Ok(true),
    }
}

fn full_read(s: &ReadOnlyHdfSession, family: LayerFamily, max_bytes: usize) -> Result<bool> {
    match family {
        LayerFamily::OneDLine(f) => read_verdict(s.one_d_line_family(f, max_bytes)),
        LayerFamily::Geometry(f) => read_verdict(s.geometry_family(f, max_bytes)),
        LayerFamily::ReferencePoints => read_verdict(s.reference_point_geometry(max_bytes)),
        LayerFamily::RiverJunctions => read_verdict(s.river_junctions(max_bytes)),
        LayerFamily::PumpStations => read_verdict(s.pump_stations(max_bytes)),
        LayerFamily::PipeNodes => read_verdict(s.pipe_nodes(max_bytes)),
        LayerFamily::Meshes => verdict(s.mesh_names().map(|_| true)),
        LayerFamily::MeshPerimeters => {
            if !full_read(s, LayerFamily::Meshes, max_bytes)? {
                return Ok(false);
            }
            let Ok(meshes) = s.mesh_names() else {
                return Ok(true);
            };
            for mesh in meshes {
                if !verdict(s.read_mesh_perimeter(&mesh, max_bytes).map(|_| true))? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        other => panic!("no full reader for {other:?}"),
    }
}

/// Compare every family of one file at a tiny and at the default budget.
/// `None` when the file does not open (malformed fixtures).
fn compare(path: &Path) -> Option<usize> {
    let s = ReadOnlyHdfSession::open(path).ok()?;
    let presence = s.layer_family_presence().unwrap();
    assert_eq!(presence.len(), LAYER_FAMILIES.len());
    let mut stored = 0;
    for (family, p) in &presence {
        for max_bytes in [1, DEFAULT_MAX_BYTES] {
            let full = full_read(&s, *family, max_bytes).unwrap();
            assert_eq!(
                p.is_stored(),
                full,
                "{}: {} at max_bytes {max_bytes}: presence {p:?}",
                path.display(),
                family.as_str()
            );
        }
        stored += usize::from(p.is_stored());
    }
    Some(stored)
}

fn hdf5_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            hdf5_files(&path, out);
        } else if let Ok(mut f) = std::fs::File::open(&path) {
            use std::io::Read;
            let mut sig = [0u8; 8];
            if f.read_exact(&mut sig).is_ok() && &sig == b"\x89HDF\r\n\x1a\n" {
                out.push(path);
            }
        }
    }
}

fn fixture(name: &str) -> ReadOnlyHdfSession {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/presence");
    let provenance: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("provenance.json")).unwrap()).unwrap();
    let file = format!("{name}.h5");
    let bytes = std::fs::read(dir.join(&file)).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        provenance["files"][&file].as_str().unwrap(),
        "{file} hash"
    );
    ReadOnlyHdfSession::open(dir.join(file)).unwrap()
}

fn stored_names(s: &ReadOnlyHdfSession) -> Vec<&'static str> {
    s.layer_family_presence()
        .unwrap()
        .into_iter()
        .filter(|(_, p)| p.is_stored())
        .map(|(f, _)| f.as_str())
        .collect()
}

#[test]
fn synthetic_cases_have_the_expected_verdicts() {
    let all: Vec<_> = LAYER_FAMILIES.iter().map(|f| f.as_str()).collect();
    assert_eq!(stored_names(&fixture("presence_stored")), all);
    assert!(stored_names(&fixture("presence_empty")).is_empty());
    assert_eq!(
        stored_names(&fixture("presence_incomplete")),
        [
            // Optional Attributes absent: read.
            OneDLineFamily::RiverFlowPaths.as_str(),
            // Required Attributes absent, every packed dataset stored: refused.
            OneDLineFamily::StorageAreaPolygons.as_str(),
            // No Name column, every dataset stored: refused.
            GeometryFamily::RefinementRegions.as_str(),
            // Only one of Attributes/Points: corrupt.
            "river_junctions",
            "meshes",
        ]
    );
    let s = fixture("presence_incomplete");
    // Area B stores no Perimeter.
    assert!(matches!(
        s.family_presence(LayerFamily::MeshPerimeters).unwrap(),
        FamilyPresence::Absent { reason } if reason.contains("Area B/Perimeter")
    ));
    // A missing required dataset is reported before any refusal of the data the reader reads
    // first, so the verdict does not depend on dataset values or on the budget.
    match s
        .family_presence(LayerFamily::OneDLine(OneDLineFamily::StructureCenterlines))
        .unwrap()
    {
        FamilyPresence::Absent { reason } => {
            assert_eq!(
                reason,
                "required dataset Geometry/Structures/Centerline Parts absent"
            )
        }
        other => panic!("{other:?}"),
    }
    let e = s
        .geometry_family(GeometryFamily::Breaklines, DEFAULT_MAX_BYTES)
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
    let legacy = fixture("presence_legacy_xs");
    assert_eq!(stored_names(&legacy), ["meshes", "mesh_perimeters"]);
    let e = legacy
        .one_d_line_family(OneDLineFamily::CrossSectionCutLines, DEFAULT_MAX_BYTES)
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound, "{e}");
}

#[test]
fn presence_agrees_with_full_reads_on_every_committed_fixture() {
    let mut files = Vec::new();
    hdf5_files(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        &mut files,
    );
    assert!(files.len() > 100, "{} fixtures", files.len());
    let compared: Vec<usize> = files.iter().filter_map(|f| compare(f)).collect();
    assert!(compared.len() > 100, "{} fixtures open", compared.len());
    assert!(compared.iter().sum::<usize>() > 0);
}

/// Opt-in: every HDF5 file below `RAS_PRESENCE_CORPUS` (the golden corpus).
#[test]
#[ignore = "requires RAS_PRESENCE_CORPUS"]
fn presence_agrees_with_full_reads_on_the_corpus() {
    let root = PathBuf::from(std::env::var("RAS_PRESENCE_CORPUS").unwrap());
    let mut files = Vec::new();
    hdf5_files(&root, &mut files);
    assert!(!files.is_empty());
    for f in &files {
        let stored = compare(f).expect("every corpus file opens");
        println!("{}\t{stored}", f.display());
    }
    println!("{} files", files.len());
}
