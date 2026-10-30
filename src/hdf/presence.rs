//! Which layer families a file stores, from group membership and object headers only.
//!
//! A family is absent exactly when its reader reports canonical absence
//! (`Reported::Unavailable`): its group is missing, or the dataset the reader anchors on is.
//! [`ReadOnlyHdfSession::family_presence`] makes those same checks, in the reader's order, and
//! nothing else. A family that stores some of its datasets but not all is stored: its reader
//! refuses it (`NotFound`, `Unsupported` or `Corrupt`, as before), and a caller that lists it
//! shows that refusal instead of treating the file as having no such data. The readers are not
//! changed. `tests/family_presence.rs` checks the verdict against the readers.
use super::{
    lines::{GeometryFamily, GEOMETRY_FAMILIES},
    xs_geometry::{OneDLineFamily, ONE_D_LINE_FAMILIES, PIPE_LINE_FAMILIES},
    ReadOnlyHdfSession,
};
use crate::{Error, ErrorKind, Result};

const MESH_GROUP: &str = "Geometry/2D Flow Areas";

/// A family of features a vector driver can list as one layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayerFamily {
    /// A packed 1D line or polygon family ([`ReadOnlyHdfSession::one_d_line_family`]).
    OneDLine(OneDLineFamily),
    /// A packed 2D geometry family ([`ReadOnlyHdfSession::geometry_family`]).
    /// `GeometryFamily::ReferencePoints` is not packed: use [`LayerFamily::ReferencePoints`].
    Geometry(GeometryFamily),
    /// `Geometry/Reference Points` ([`ReadOnlyHdfSession::reference_point_geometry`]).
    ReferencePoints,
    /// `Geometry/Junctions` ([`ReadOnlyHdfSession::river_junctions`]).
    RiverJunctions,
    /// `Geometry/Pump Stations` ([`ReadOnlyHdfSession::pump_stations`]).
    PumpStations,
    /// `Geometry/Pipe Nodes` ([`ReadOnlyHdfSession::pipe_nodes`]).
    PipeNodes,
    /// 2D flow areas ([`ReadOnlyHdfSession::mesh_names`]).
    Meshes,
    /// A `Perimeter` dataset in every 2D flow area (`read_mesh_perimeter`).
    MeshPerimeters,
}

/// Every [`LayerFamily`], in a stable order: 1D line families, pipe line families, packed 2D
/// geometry families (without reference points), then reference points, the point families
/// and the mesh families.
pub const LAYER_FAMILIES: &[LayerFamily] = &{
    const N: usize =
        ONE_D_LINE_FAMILIES.len() + PIPE_LINE_FAMILIES.len() + GEOMETRY_FAMILIES.len() - 1 + 6;
    let mut out = [LayerFamily::Meshes; N];
    let mut i = 0;
    let mut j = 0;
    while j < ONE_D_LINE_FAMILIES.len() {
        out[i] = LayerFamily::OneDLine(ONE_D_LINE_FAMILIES[j]);
        i += 1;
        j += 1;
    }
    j = 0;
    while j < PIPE_LINE_FAMILIES.len() {
        out[i] = LayerFamily::OneDLine(PIPE_LINE_FAMILIES[j]);
        i += 1;
        j += 1;
    }
    j = 0;
    while j < GEOMETRY_FAMILIES.len() {
        if !matches!(GEOMETRY_FAMILIES[j], GeometryFamily::ReferencePoints) {
            out[i] = LayerFamily::Geometry(GEOMETRY_FAMILIES[j]);
            i += 1;
        }
        j += 1;
    }
    out[i] = LayerFamily::ReferencePoints;
    out[i + 1] = LayerFamily::RiverJunctions;
    out[i + 2] = LayerFamily::PumpStations;
    out[i + 3] = LayerFamily::PipeNodes;
    out[i + 4] = LayerFamily::Meshes;
    out[i + 5] = LayerFamily::MeshPerimeters;
    out
};

impl LayerFamily {
    /// Stable snake_case spelling (never the Debug form). Line and geometry families use their
    /// own spellings (`cross_section_cut_lines`, `breaklines`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OneDLine(f) => f.as_str(),
            Self::Geometry(f) => f.as_str(),
            Self::ReferencePoints => "reference_points",
            Self::RiverJunctions => "river_junctions",
            Self::PumpStations => "pump_stations",
            Self::PipeNodes => "pipe_nodes",
            Self::Meshes => "meshes",
            Self::MeshPerimeters => "mesh_perimeters",
        }
    }
}

/// Whether a file stores a [`LayerFamily`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FamilyPresence {
    /// The family is not canonically absent. Its reader returns the data or refuses it
    /// (incomplete, unsupported, corrupt or over budget); only reading it says which.
    Stored,
    /// The family's reader reports canonical absence; `reason` is the reader's.
    Absent { reason: String },
}

impl FamilyPresence {
    pub fn is_stored(&self) -> bool {
        matches!(self, Self::Stored)
    }
}

/// Errors of the host or the engine itself, which a presence check propagates.
fn host(e: &Error) -> bool {
    matches!(
        e.kind,
        ErrorKind::Io | ErrorKind::Internal | ErrorKind::FileNotFound
    )
}

/// `Some(reason)`: canonically absent. An error from a check the reader also makes first
/// means the reader refuses the family, so it is stored; host errors propagate.
fn verdict(absence: Result<Option<&'static str>>) -> Result<FamilyPresence> {
    match absence {
        Ok(Some(reason)) => Ok(FamilyPresence::Absent {
            reason: reason.into(),
        }),
        Ok(None) => Ok(FamilyPresence::Stored),
        Err(e) if host(&e) => Err(e),
        Err(_) => Ok(FamilyPresence::Stored),
    }
}

impl ReadOnlyHdfSession {
    /// The canonical-absence checks of [`Self::one_d_line_family`], in its order.
    fn one_d_line_absence(&self, family: OneDLineFamily) -> Result<Option<&'static str>> {
        let group = family.group_path();
        if !self.has_group(group)? {
            return Ok(Some("canonical 1D line family absent"));
        }
        if !self.has_dataset(&format!("{group}/{} Info", family.prefix()))? {
            return Ok(Some("packed line datasets absent"));
        }
        Ok(None)
    }
    /// The canonical-absence checks of [`Self::geometry_family`], in its order. Reference
    /// points are refused by that reader when their group exists.
    fn geometry_family_absence(&self, family: GeometryFamily) -> Result<Option<&'static str>> {
        if !self.has_group(family.group_path())? {
            return Ok(Some("canonical geometry family absent"));
        }
        if family == GeometryFamily::CalibrationRegions
            && !self.has_dataset(&format!("{}/Polygon Info", family.group_path()))?
        {
            return Ok(Some("calibration polygons absent"));
        }
        Ok(None)
    }
    /// The canonical-absence check of [`Self::reference_point_geometry`].
    fn reference_point_absence(&self) -> Result<Option<&'static str>> {
        if !self.has_group("Geometry/Reference Points")? {
            return Ok(Some("canonical reference-point geometry absent"));
        }
        Ok(None)
    }
    /// The canonical-absence checks of the point families (`river_junctions`,
    /// `pump_stations`, `pipe_nodes`). Only one of `Attributes` and `Points` is Corrupt
    /// there, so stored.
    fn point_family_absence(&self, group: &str) -> Result<Option<&'static str>> {
        if !self.has_group(group)? {
            return Ok(Some("canonical point family absent"));
        }
        let attributes = self.has_dataset(&format!("{group}/Attributes"))?;
        let points = self.has_dataset(&format!("{group}/Points"))?;
        Ok((!attributes && !points).then_some("point family datasets absent"))
    }

    /// Whether the file stores `family`, from group membership and object headers only: no
    /// dataset values are read, at the same cost for every family, whatever its size.
    ///
    /// [`FamilyPresence::Absent`] exactly when the family's reader reports canonical absence
    /// (`Reported::Unavailable`). Every other family is [`FamilyPresence::Stored`], including
    /// one that stores some of its datasets but not all: its reader refuses it (`NotFound`,
    /// `Unsupported` or `Corrupt`), and listing it lets a caller show that refusal rather than
    /// treat a damaged file as having no such data. Errors of the host (`Io`, `FileNotFound`)
    /// and `Internal` errors propagate. The readers are unchanged and the verdict does not
    /// depend on a byte budget.
    ///
    /// The mesh families have no `Unavailable` report: [`LayerFamily::Meshes`] is absent when
    /// `mesh_names` is `NotFound` (no `Geometry/2D Flow Areas` group), and
    /// [`LayerFamily::MeshPerimeters`] is absent when meshes are or when no mesh stores a
    /// `Perimeter`. When some meshes store one and others do not, it is stored, and
    /// `read_mesh_perimeter` refuses the others (`NotFound`).
    pub fn family_presence(&self, family: LayerFamily) -> Result<FamilyPresence> {
        match family {
            LayerFamily::OneDLine(f) => verdict(self.one_d_line_absence(f)),
            LayerFamily::Geometry(f) => verdict(self.geometry_family_absence(f)),
            LayerFamily::ReferencePoints => verdict(self.reference_point_absence()),
            LayerFamily::RiverJunctions => verdict(self.point_family_absence("Geometry/Junctions")),
            LayerFamily::PumpStations => {
                verdict(self.point_family_absence("Geometry/Pump Stations"))
            }
            LayerFamily::PipeNodes => verdict(self.point_family_absence("Geometry/Pipe Nodes")),
            LayerFamily::Meshes => match self.mesh_names() {
                Err(e) if e.kind == ErrorKind::NotFound => {
                    Ok(FamilyPresence::Absent { reason: e.message })
                }
                Err(e) if host(&e) => Err(e),
                _ => Ok(FamilyPresence::Stored),
            },
            LayerFamily::MeshPerimeters => {
                if !self.family_presence(LayerFamily::Meshes)?.is_stored() {
                    return Ok(FamilyPresence::Absent {
                        reason: "no Geometry/2D Flow Areas group".into(),
                    });
                }
                let meshes = match self.mesh_names() {
                    Ok(meshes) => meshes,
                    Err(e) if host(&e) => return Err(e),
                    // The mesh list is refused, and so is every perimeter read.
                    Err(_) => return Ok(FamilyPresence::Stored),
                };
                for mesh in meshes {
                    // read_mesh_perimeter refuses such a name before it reads anything.
                    if mesh.is_empty() || mesh.contains(['/', '\\']) {
                        return Ok(FamilyPresence::Stored);
                    }
                    let path = format!("{MESH_GROUP}/{mesh}/Perimeter");
                    match self.has_dataset(&path) {
                        Ok(false) => {}
                        Err(e) if host(&e) => return Err(e),
                        // Stored, or refused when read.
                        _ => return Ok(FamilyPresence::Stored),
                    }
                }
                Ok(FamilyPresence::Absent {
                    reason: "no 2D flow area stores a Perimeter".into(),
                })
            }
        }
    }
    /// [`Self::family_presence`] for every family of [`LAYER_FAMILIES`], in that order.
    pub fn layer_family_presence(&self) -> Result<Vec<(LayerFamily, FamilyPresence)>> {
        LAYER_FAMILIES
            .iter()
            .map(|&f| Ok((f, self.family_presence(f)?)))
            .collect()
    }
}

/// Bytes read and time of a layer listing by full reads and by `family_presence`.
/// `RAS_PRESENCE_MEASURE` lists HDF files separated by `;`.
/// `cargo test --release --lib -- --ignored --nocapture presence_cost`
#[cfg(all(test, feature = "mesh"))]
mod measure {
    use super::*;
    use crate::hdf::reader::File;
    use hdf5_reader as h5;
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    use std::sync::Arc;

    struct Counting {
        inner: h5::storage::FileStorage,
        bytes: Arc<AtomicU64>,
        reads: Arc<AtomicU64>,
    }
    impl h5::Storage for Counting {
        fn len(&self) -> u64 {
            self.inner.len()
        }
        fn read_range(&self, offset: u64, len: usize) -> h5::error::Result<h5::StorageBuffer> {
            self.bytes.fetch_add(len as u64, Relaxed);
            self.reads.fetch_add(1, Relaxed);
            self.inner.read_range(offset, len)
        }
    }
    fn session(path: &str) -> (ReadOnlyHdfSession, Arc<AtomicU64>, Arc<AtomicU64>) {
        let (bytes, reads) = (Arc::default(), Arc::default());
        let storage = Counting {
            inner: h5::storage::FileStorage::open(path).unwrap(),
            bytes: Arc::clone(&bytes),
            reads: Arc::clone(&reads),
        };
        let file =
            File::from_storage_with_options(Arc::new(storage), h5::OpenOptions::default()).unwrap();
        let s = ReadOnlyHdfSession {
            file,
            pool: crate::SessionPool::new(1).unwrap(),
            memo: Default::default(),
            path: path.into(),
            parallel: true,
            chunk_cache_bytes: h5::OpenOptions::default().chunk_cache_bytes,
        };
        (s, bytes, reads)
    }
    /// The same listing by full reads: each family read whole (metadata budget for families,
    /// the whole-read budget for perimeters), absent only when canonically absent.
    fn full_listing(s: &ReadOnlyHdfSession) -> Vec<bool> {
        use crate::hdf::DEFAULT_MAX_BYTES as M;
        fn v<T>(r: Result<crate::Reported<T>>) -> bool {
            !matches!(r, Ok(r) if r.is_unavailable())
        }
        let meshes = !matches!(s.mesh_names(), Err(e) if e.kind == ErrorKind::NotFound);
        LAYER_FAMILIES
            .iter()
            .map(|&f| match f {
                LayerFamily::OneDLine(f) => v(s.one_d_line_family(f, M)),
                LayerFamily::Geometry(f) => v(s.geometry_family(f, M)),
                LayerFamily::ReferencePoints => v(s.reference_point_geometry(M)),
                LayerFamily::RiverJunctions => v(s.river_junctions(M)),
                LayerFamily::PumpStations => v(s.pump_stations(M)),
                LayerFamily::PipeNodes => v(s.pipe_nodes(M)),
                LayerFamily::Meshes => meshes,
                LayerFamily::MeshPerimeters => {
                    meshes
                        && match s.mesh_names() {
                            Err(_) => true,
                            Ok(names) => names.iter().any(|m| {
                                !matches!(
                                    s.read_mesh_perimeter(m, crate::hdf::DEFAULT_READ_MAX_BYTES),
                                    Err(e) if e.kind == ErrorKind::NotFound
                                )
                            }),
                        }
                }
            })
            .collect()
    }
    #[test]
    #[ignore = "requires RAS_PRESENCE_MEASURE"]
    fn presence_cost() {
        let files = std::env::var("RAS_PRESENCE_MEASURE").unwrap();
        println!("file	size	full_bytes	full_reads	full_ms	presence_bytes	presence_reads	presence_ms	open_bytes");
        for path in files.split(';').filter(|p| !p.is_empty()) {
            let size = std::fs::metadata(path).unwrap().len();
            let best = |f: &dyn Fn() -> (u64, u64, u64, f64, Vec<bool>)| {
                (0..5)
                    .map(|_| f())
                    .min_by(|a, b| a.3.total_cmp(&b.3))
                    .unwrap()
            };
            let run = |presence: bool| {
                let (s, bytes, reads) = session(path);
                let (open, open_reads) = (bytes.load(Relaxed), reads.load(Relaxed));
                let t = std::time::Instant::now();
                let listed = if presence {
                    s.layer_family_presence()
                        .unwrap()
                        .into_iter()
                        .map(|(_, p)| p.is_stored())
                        .collect()
                } else {
                    full_listing(&s)
                };
                let ms = t.elapsed().as_secs_f64() * 1e3;
                (
                    open,
                    bytes.load(Relaxed) - open,
                    reads.load(Relaxed) - open_reads,
                    ms,
                    listed,
                )
            };
            let full = best(&|| run(false));
            let presence = best(&|| run(true));
            assert_eq!(full.4, presence.4, "{path}");
            println!(
                "{path}	{size}	{}	{}	{:.2}	{}	{}	{:.2}	{}",
                full.1, full.2, full.3, presence.1, presence.2, presence.3, full.0
            );
        }
    }
}
