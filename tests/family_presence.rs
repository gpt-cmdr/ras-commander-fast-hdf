#![cfg(all(feature = "hdf", feature = "mesh"))]
//! `family_presence` gives the readers' canonical-absence verdict from metadata only, and the
//! readers keep their own outcomes.
//!
//! A family is absent exactly when its reader reports `Reported::Unavailable` (for the mesh
//! families, which have none: `mesh_names` is `NotFound`, or no mesh stores a `Perimeter`).
//! A family with some of its datasets but not all is stored, and its reader refuses it.
//! Synthetic cases (generator: tests/generate_presence_fixtures.py) pin every reader's exact
//! outcome; every committed HDF5 fixture, and every file below `RAS_PRESENCE_CORPUS` when set,
//! is compared.
use ras_commander_fast_hdf::hdf::presence::{FamilyPresence, LayerFamily, LAYER_FAMILIES};
use ras_commander_fast_hdf::hdf::{ReadOnlyHdfSession, DEFAULT_MAX_BYTES};
use ras_commander_fast_hdf::{Reported, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// What a family's full read gives: `unavailable`, `supported`, a `Reported` refusal
/// (`reported:<status>`), or the error kind and code (`not_found`, `unsupported/identity_layout`).
fn outcome<T>(r: Result<Reported<T>>) -> String {
    match r {
        Ok(r) if r.is_unavailable() => "unavailable".into(),
        Ok(r) if r.value().is_some() => "supported".into(),
        Ok(r) => format!("reported:{}", r.capability().as_str()),
        Err(e) => error(&e),
    }
}
fn error(e: &ras_commander_fast_hdf::Error) -> String {
    match e.code {
        Some(code) => format!("{}/{}", e.kind.as_str(), code.as_str()),
        None => e.kind.as_str().into(),
    }
}

fn full_read(s: &ReadOnlyHdfSession, family: LayerFamily, max_bytes: usize) -> String {
    match family {
        LayerFamily::OneDLine(f) => outcome(s.one_d_line_family(f, max_bytes)),
        LayerFamily::Geometry(f) => outcome(s.geometry_family(f, max_bytes)),
        LayerFamily::ReferencePoints => outcome(s.reference_point_geometry(max_bytes)),
        LayerFamily::RiverJunctions => outcome(s.river_junctions(max_bytes)),
        LayerFamily::PumpStations => outcome(s.pump_stations(max_bytes)),
        LayerFamily::PipeNodes => outcome(s.pipe_nodes(max_bytes)),
        LayerFamily::Meshes => match s.mesh_names() {
            Ok(_) => "supported".into(),
            Err(e) => error(&e),
        },
        LayerFamily::MeshPerimeters => match s.mesh_names() {
            Err(e) => format!("meshes:{}", error(&e)),
            Ok(meshes) => meshes
                .iter()
                .map(|m| match s.read_mesh_perimeter(m, max_bytes) {
                    Ok(_) => "supported".to_owned(),
                    Err(e) => error(&e),
                })
                .collect::<Vec<_>>()
                .join(","),
        },
        other => panic!("no full reader for {other:?}"),
    }
}

/// The canonical-absence verdict of a full-read outcome.
fn absent(family: LayerFamily, outcome: &str) -> bool {
    match family {
        LayerFamily::Meshes => outcome == "not_found",
        LayerFamily::MeshPerimeters => {
            outcome == "meshes:not_found" || outcome.split(',').all(|o| o == "not_found")
        }
        _ => outcome == "unavailable",
    }
}

/// Compare every family of one file at a tiny and at the default budget. `None` when the file
/// does not open (malformed fixtures).
fn compare(path: &Path) -> Option<usize> {
    let s = ReadOnlyHdfSession::open(path).ok()?;
    let presence = s.layer_family_presence().unwrap();
    assert_eq!(presence.len(), LAYER_FAMILIES.len());
    let mut stored = 0;
    for (family, p) in &presence {
        for max_bytes in [1, DEFAULT_MAX_BYTES] {
            let full = full_read(&s, *family, max_bytes);
            assert_eq!(
                !p.is_stored(),
                absent(*family, &full),
                "{}: {} at max_bytes {max_bytes}: presence {p:?}, full read {full}",
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

/// `family presence full-read` lines of every family of a fixture, full read at the default
/// budget.
fn table(name: &str) -> Vec<String> {
    let s = fixture(name);
    s.layer_family_presence()
        .unwrap()
        .into_iter()
        .map(|(f, p)| {
            format!(
                "{} {} {}",
                f.as_str(),
                if p.is_stored() { "stored" } else { "absent" },
                full_read(&s, f, DEFAULT_MAX_BYTES)
            )
        })
        .collect()
}

const CASES: [&str; 8] = [
    "presence_stored",
    "presence_empty",
    "presence_drop_attributes",
    "presence_drop_info",
    "presence_drop_parts",
    "presence_drop_points",
    "presence_incomplete",
    "presence_legacy_xs",
];

/// Every synthetic case: `case family presence full-read`, pinned in
/// `tests/fixtures/presence/outcomes.txt`. The readers are unchanged from engine 0.2.4, so the
/// full-read column is also their outcome before `family_presence` existed. Regenerate with
/// `PRESENCE_PRINT=1 cargo test --test family_presence -- --nocapture synthetic`.
#[test]
fn synthetic_cases_match_the_pinned_outcomes() {
    let lines: Vec<String> = CASES
        .iter()
        .flat_map(|name| table(name).into_iter().map(move |l| format!("{name} {l}")))
        .collect();
    if std::env::var_os("PRESENCE_PRINT").is_some() {
        for l in &lines {
            println!("{l}");
        }
    }
    let pinned = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/presence/outcomes.txt"),
    )
    .unwrap();
    let pinned: Vec<&str> = pinned.lines().collect();
    assert_eq!(lines, pinned);
    // A family with some datasets but not all is never absent: absent is exactly the
    // reader's Unavailable (NotFound of the mesh list, or no Perimeter in any mesh).
    for l in &lines {
        let f: Vec<&str> = l.split(' ').collect();
        let absent = f[3] == "unavailable"
            || f[3] == "meshes:not_found"
            || (f[1] == "meshes" && f[3] == "not_found")
            || (f[1] == "mesh_perimeters" && f[3].split(',').all(|o| o == "not_found"));
        assert_eq!(f[2] == "absent", absent, "{l}");
    }
}

#[test]
fn presence_agrees_with_full_reads_on_every_committed_fixture() {
    let mut files = Vec::new();
    hdf5_files(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        &mut files,
    );
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

#[test]
fn reasons_are_the_readers() {
    let s = fixture("presence_drop_info");
    assert_eq!(
        s.family_presence(LayerFamily::OneDLine(
            ras_commander_fast_hdf::hdf::xs_geometry::OneDLineFamily::RiverCenterlines
        ))
        .unwrap(),
        FamilyPresence::Absent {
            reason: "packed line datasets absent".into()
        }
    );
}
