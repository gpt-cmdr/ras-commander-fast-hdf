//! Which layer families a file stores, from group membership and object headers only.
//!
//! Each family reader begins with a prelude, defined here, that decides canonical absence
//! (`Reported::Unavailable`) and then requires every dataset the reader opens unconditionally
//! (`NotFound` when one is missing), before it reads anything. [`ReadOnlyHdfSession::family_presence`]
//! runs the same prelude and stops there, so its verdict is the reader's by construction:
//! absent exactly when the reader reports canonical absence or a missing required dataset,
//! stored when the reader returns data or refuses it (unsupported, corrupt, over budget).
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
    /// The family's reader returns its data or refuses it (unsupported, corrupt, over
    /// budget). Reading it is the only way to know which.
    Stored,
    /// The reader reports canonical absence (`Reported::Unavailable`) or a dataset it needs is
    /// missing (`NotFound`). `reason` is the reader's.
    Absent { reason: String },
}

impl FamilyPresence {
    pub fn is_stored(&self) -> bool {
        matches!(self, Self::Stored)
    }
}

/// Errors of the host or the engine itself, which a presence check propagates rather than
/// reports as a refusal of the family.
fn host(e: &Error) -> bool {
    matches!(
        e.kind,
        ErrorKind::Io | ErrorKind::Internal | ErrorKind::FileNotFound
    )
}

/// The verdict of a prelude: canonical absence, a missing required dataset, or neither.
fn verdict(prelude: Result<Option<&'static str>>) -> Result<FamilyPresence> {
    match prelude {
        Ok(Some(reason)) => Ok(FamilyPresence::Absent {
            reason: reason.into(),
        }),
        Ok(None) => Ok(FamilyPresence::Stored),
        Err(e) if e.kind == ErrorKind::NotFound => Ok(FamilyPresence::Absent { reason: e.message }),
        Err(e) if host(&e) => Err(e),
        // The reader stops here with the same refusal.
        Err(_) => Ok(FamilyPresence::Stored),
    }
}

impl ReadOnlyHdfSession {
    /// Require `group/name` for each name: `NotFound` for the first one missing.
    fn require_datasets(&self, group: &str, names: &[&str]) -> Result<()> {
        for name in names {
            let path = format!("{group}/{name}");
            if !self.has_dataset(&path)? {
                return Err(Error::new(
                    ErrorKind::NotFound,
                    format!("required dataset {path} absent"),
                ));
            }
        }
        Ok(())
    }
    /// Prelude of [`Self::one_d_line_family`]: `Some(reason)` when canonically absent.
    pub(super) fn one_d_line_prelude(
        &self,
        family: OneDLineFamily,
    ) -> Result<Option<&'static str>> {
        let group = family.group_path();
        if !self.has_group(group)? {
            return Ok(Some("canonical 1D line family absent"));
        }
        let prefix = family.prefix();
        if !self.has_dataset(&format!("{group}/{prefix} Info"))? {
            return Ok(Some("packed line datasets absent"));
        }
        self.require_datasets(
            group,
            &[&format!("{prefix} Parts"), &format!("{prefix} Points")],
        )?;
        Ok(None)
    }
    /// Prelude of [`Self::geometry_family`]: `Some(reason)` when canonically absent. Reference
    /// points pass (the reader refuses them as not packed).
    pub(super) fn geometry_family_prelude(
        &self,
        family: GeometryFamily,
    ) -> Result<Option<&'static str>> {
        let group = family.group_path();
        if !self.has_group(group)? {
            return Ok(Some("canonical geometry family absent"));
        }
        if family == GeometryFamily::ReferencePoints {
            return Ok(None);
        }
        let prefix = family.prefix();
        // Land cover metadata without calibration polygons is not a region family.
        if family == GeometryFamily::CalibrationRegions
            && !self.has_dataset(&format!("{group}/Polygon Info"))?
        {
            return Ok(Some("calibration polygons absent"));
        }
        self.require_datasets(
            group,
            &[
                "Attributes",
                &format!("{prefix} Info"),
                &format!("{prefix} Parts"),
                &format!("{prefix} Points"),
            ],
        )?;
        Ok(None)
    }
    /// Prelude of [`Self::reference_point_geometry`].
    pub(super) fn reference_point_prelude(&self) -> Result<Option<&'static str>> {
        let root = "Geometry/Reference Points";
        if !self.has_group(root)? {
            return Ok(Some("canonical reference-point geometry absent"));
        }
        self.require_datasets(root, &["Attributes", "Points"])?;
        Ok(None)
    }
    /// Prelude of the point families (`river_junctions`, `pump_stations`, `pipe_nodes`). Only
    /// one of `Attributes` and `Points` stored is Corrupt, which the reader refuses.
    pub(super) fn point_family_prelude(&self, group: &str) -> Result<Option<&'static str>> {
        if !self.has_group(group)? {
            return Ok(Some("canonical point family absent"));
        }
        match (
            self.has_dataset(&format!("{group}/Attributes"))?,
            self.has_dataset(&format!("{group}/Points"))?,
        ) {
            (true, true) => Ok(None),
            (false, false) => Ok(Some("point family datasets absent")),
            _ => Err(Error::new(
                ErrorKind::Corrupt,
                "incomplete Attributes/Points pair",
            )),
        }
    }

    /// Whether the file stores `family`, from group membership and object headers only: no
    /// dataset values are read, at the same cost for every family, whatever its size.
    ///
    /// The verdict is the family reader's (each reader runs the same checks first):
    /// [`FamilyPresence::Absent`] exactly when it reports canonical absence or a dataset it
    /// needs is missing (`NotFound`); [`FamilyPresence::Stored`] when it returns the data or
    /// refuses it. A stored family can still be refused when read; only reading it says so.
    /// For [`LayerFamily::Meshes`] the reader is `mesh_names` (absent: no `2D Flow Areas`
    /// group); for [`LayerFamily::MeshPerimeters`] it is `read_mesh_perimeter` for every mesh
    /// (absent when meshes are absent or any one stores no `Perimeter`; stored when the mesh
    /// list is refused). The verdict does not depend on a byte budget: a family over budget is
    /// stored. Errors of the host (`Io`, `FileNotFound`) and `Internal` errors propagate.
    pub fn family_presence(&self, family: LayerFamily) -> Result<FamilyPresence> {
        match family {
            LayerFamily::OneDLine(f) => verdict(self.one_d_line_prelude(f)),
            LayerFamily::Geometry(f) => verdict(self.geometry_family_prelude(f)),
            LayerFamily::ReferencePoints => verdict(self.reference_point_prelude()),
            LayerFamily::RiverJunctions => verdict(self.point_family_prelude("Geometry/Junctions")),
            LayerFamily::PumpStations => {
                verdict(self.point_family_prelude("Geometry/Pump Stations"))
            }
            LayerFamily::PipeNodes => verdict(self.point_family_prelude("Geometry/Pipe Nodes")),
            LayerFamily::Meshes => verdict(self.mesh_names().map(|_| None)),
            LayerFamily::MeshPerimeters => {
                let meshes = match verdict(self.mesh_names().map(|_| None))? {
                    FamilyPresence::Stored => match self.mesh_names() {
                        Ok(meshes) => meshes,
                        // The mesh list is refused: so is every perimeter read.
                        Err(_) => return Ok(FamilyPresence::Stored),
                    },
                    absent => return Ok(absent),
                };
                for mesh in meshes {
                    // read_mesh_perimeter refuses such a name before it reads anything.
                    if mesh.is_empty() || mesh.contains(['/', '\\']) {
                        continue;
                    }
                    // read_mesh_perimeter's first step: the dataset's numeric metadata.
                    let path = format!("{MESH_GROUP}/{mesh}/Perimeter");
                    if let FamilyPresence::Absent { reason } =
                        verdict(self.numeric_capability(&path).map(|_| None))?
                    {
                        return Ok(FamilyPresence::Absent { reason });
                    }
                }
                Ok(FamilyPresence::Stored)
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
    /// The listing a vector driver made before: each family read whole (metadata budget for
    /// families, the whole-read budget for perimeters), stored unless absent or NotFound.
    fn full_listing(s: &ReadOnlyHdfSession) -> Vec<bool> {
        use crate::hdf::DEFAULT_MAX_BYTES as M;
        fn v<T>(r: Result<crate::Reported<T>>) -> bool {
            match r {
                Ok(r) => !r.is_unavailable(),
                Err(e) => e.kind != ErrorKind::NotFound,
            }
        }
        LAYER_FAMILIES
            .iter()
            .map(|&f| match f {
                LayerFamily::OneDLine(f) => v(s.one_d_line_family(f, M)),
                LayerFamily::Geometry(f) => v(s.geometry_family(f, M)),
                LayerFamily::ReferencePoints => v(s.reference_point_geometry(M)),
                LayerFamily::RiverJunctions => v(s.river_junctions(M)),
                LayerFamily::PumpStations => v(s.pump_stations(M)),
                LayerFamily::PipeNodes => v(s.pipe_nodes(M)),
                LayerFamily::Meshes => {
                    !matches!(s.mesh_names(), Err(e) if e.kind == ErrorKind::NotFound)
                }
                LayerFamily::MeshPerimeters => match s.mesh_names() {
                    Err(e) => e.kind != ErrorKind::NotFound,
                    Ok(meshes) => meshes.iter().all(|m| {
                        !matches!(
                            s.read_mesh_perimeter(m, crate::hdf::DEFAULT_READ_MAX_BYTES),
                            Err(e) if e.kind == ErrorKind::NotFound
                        )
                    }),
                },
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
