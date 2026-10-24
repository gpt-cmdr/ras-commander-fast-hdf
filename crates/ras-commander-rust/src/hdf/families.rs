//! Qualified remaining stored read families. No inferred or reconstructed hydraulic values.
use super::{
    encoding::map_error,
    metadata::{MetadataValue, TypedAttribute},
    stored::{self, StoredDataset, StoredValues},
    Identity, ReadOnlyHdfSession, Slice,
};
use crate::{
    capability::{absent_ok, error_to_capability},
    Capability, Error, ErrorKind, Result, UnsupportedCode,
};

const OUTPUT: &str = "Results/Unsteady/Output/Output Blocks";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReadFamily {
    PipeGeometry,
    PipeRunSummary,
    PipeResults,
    ReferenceLines,
    ReferencePoints,
    ReferenceSummaries,
    Breach,
    PlanMetadata,
    LandCover,
    Infiltration,
    Sediment,
    Encroachments,
    Terrain,
    /// RASMapper terrain-modification attribute tables (`Modifications/<group>/Attributes`).
    TerrainModifications,
    EventConditions,
    SteadyOutput,
}
pub const READ_FAMILIES: &[ReadFamily] = &[
    ReadFamily::PipeGeometry,
    ReadFamily::PipeRunSummary,
    ReadFamily::PipeResults,
    ReadFamily::ReferenceLines,
    ReadFamily::ReferencePoints,
    ReadFamily::ReferenceSummaries,
    ReadFamily::Breach,
    ReadFamily::PlanMetadata,
    ReadFamily::LandCover,
    ReadFamily::Infiltration,
    ReadFamily::Sediment,
    ReadFamily::Encroachments,
    ReadFamily::Terrain,
    ReadFamily::TerrainModifications,
    ReadFamily::EventConditions,
    ReadFamily::SteadyOutput,
];
impl ReadFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PipeGeometry => "pipe_geometry",
            Self::PipeRunSummary => "pipe_run_summary",
            Self::PipeResults => "pipe_results",
            Self::ReferenceLines => "reference_line_results",
            Self::ReferencePoints => "reference_point_results",
            Self::ReferenceSummaries => "reference_summaries",
            Self::Breach => "breach",
            Self::PlanMetadata => "plan_metadata",
            Self::LandCover => "land_cover",
            Self::Infiltration => "infiltration",
            Self::Sediment => "sediment",
            Self::Encroachments => "encroachments",
            Self::Terrain => "terrain",
            Self::TerrainModifications => "terrain_modifications",
            Self::EventConditions => "event_conditions",
            Self::SteadyOutput => "steady_output",
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct FamilyDataset {
    pub stored: StoredDataset,
    /// stored_table, geometry_array, time_series, direction_series, summary_rows, one_row_unqualified,
    /// name_axis, breach_progression or metadata. No units or extreme time is invented.
    pub role: String,
    pub domain: String,
    pub time_group: Option<String>,
    pub identity_table: Option<String>,
    /// Row identities on the location axis; empty when there is no qualified identity axis.
    pub locations: Vec<Identity>,
    pub names: Vec<String>,
    /// Exact geometry Attributes identities aligned with result columns when qualified.
    /// Currently populated for reference points; Name-axis identities stay in locations.
    pub geometry_locations: Vec<Identity>,
    /// Exact integer values from a qualified pipe Node/Conduit Indices table, aligned
    /// with locations. Identity.row is the table position, not this stored ID.
    pub stored_location_ids: Option<super::NumericArray>,
    /// Only present if an existing row is explicitly labelled Time.
    pub time_of_extreme_row: Option<u64>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct FamilyAttributes {
    pub id: Identity,
    pub path: String,
    pub attributes: Vec<TypedAttribute>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct FamilyInventory {
    pub family: ReadFamily,
    pub capability: Capability,
    pub datasets: Vec<FamilyDataset>,
    pub groups: Vec<FamilyAttributes>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct OutputBlock {
    pub name: String,
    pub path: String,
    pub datasets: Vec<StoredDataset>,
}
fn corrupt(m: impl Into<String>) -> Error {
    Error::new(ErrorKind::Corrupt, m)
}
fn invalid(m: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidRequest, m)
}
fn refuse() -> Capability {
    Capability::unsupported(
        UnsupportedCode::Schema,
        "unqualified layout: this family has no qualified reader yet",
    )
}
/// Warm the session's `group_members` memo for the hierarchy under `root`, level by level
/// on the session pool, once per session and root. Only successful listings are memoized
/// (never errors), so the sequential walk in [`paths`] returns exactly what it would
/// without this, including its hierarchy-limit refusals; this only spends the header
/// parsing in parallel. Bounded like the walk (depth 24, 100,000 entries).
fn prefetch_members(s: &ReadOnlyHdfSession, root: &str) {
    if !s.parallel_metadata() {
        return;
    }
    let _ = s.memo(format!("group_members_prefetch:{:?}", (root,)), || {
        let mut level = vec![(root.to_owned(), 0usize)];
        let mut seen = 0usize;
        while !level.is_empty() {
            let listed = s.ordered_map(&level, |(group, _)| s.group_members(group).ok());
            let mut next = Vec::new();
            for ((group, depth), members) in level.iter().zip(listed) {
                let Some(members) = members else { continue };
                seen += members.datasets.len() + members.groups.len();
                if *depth >= MAX_WALK_DEPTH || seen > MAX_WALK_ENTRIES {
                    return Ok(());
                }
                next.extend(
                    members
                        .groups
                        .into_iter()
                        .map(|n| (format!("{group}/{n}"), depth + 1)),
                );
            }
            level = next;
        }
        Ok(())
    });
}
/// Hierarchy bounds of the stored-family walk (refused as `ResourceLimit`).
const MAX_WALK_DEPTH: usize = 24;
const MAX_WALK_ENTRIES: usize = 100_000;
fn paths(s: &ReadOnlyHdfSession, root: &str) -> Result<Vec<String>> {
    paths_bounded(s, root, MAX_WALK_ENTRIES, MAX_WALK_DEPTH)
}
fn paths_bounded(
    s: &ReadOnlyHdfSession,
    root: &str,
    max_entries: usize,
    max_depth: usize,
) -> Result<Vec<String>> {
    if absent_ok(s.file.group(root).map_err(map_error))?.is_none() {
        return Ok(Vec::new());
    }
    prefetch_members(s, root);
    let mut pending = vec![(root.to_owned(), 0usize)];
    let mut out = Vec::new();
    while let Some((path, depth)) = pending.pop() {
        if depth > max_depth || out.len() + pending.len() > max_entries {
            return Err(Error::unsupported(
                UnsupportedCode::ResourceLimit,
                "stored family hierarchy limit",
            ));
        }
        let m = s.group_members(&path)?;
        out.extend(m.datasets.into_iter().map(|n| format!("{path}/{n}")));
        pending.extend(
            m.groups
                .into_iter()
                .map(|n| (format!("{path}/{n}"), depth + 1)),
        );
    }
    out.sort();
    Ok(out)
}
fn result_family(path: &str, family: ReadFamily) -> bool {
    match family {
        ReadFamily::PipeResults => path.contains("/Pipe Networks/"),
        ReadFamily::ReferencePoints => {
            path.contains("/Reference Points/") && path.contains("/Unsteady Time Series/")
        }
        ReadFamily::ReferenceLines => {
            path.contains("/Reference Lines/") && path.contains("/Unsteady Time Series/")
        }
        ReadFamily::Breach => path.ends_with("/Breaching Variables"),
        _ => false,
    }
}
fn bytes_for_identities(n: u64, max: usize) -> Result<()> {
    let n = usize::try_from(n)
        .ok()
        .and_then(|n| n.checked_mul(std::mem::size_of::<Identity>()))
        .ok_or_else(|| {
            invalid("identity allocation overflow").with_code(UnsupportedCode::ResourceLimit)
        })?;
    stored::budget(n, max)
}
impl ReadOnlyHdfSession {
    /// Exact stored datasets of every non-Base unsteady output block. This envelope does
    /// not assert mesh/XS/structure semantics; family readers add their own axis checks.
    pub fn output_block_inventory(&self, max_bytes: usize) -> Result<Vec<OutputBlock>> {
        let Some(m) = absent_ok(self.group_members(OUTPUT))? else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for name in m.groups.into_iter().filter(|n| n != "Base Output") {
            let path = format!("{OUTPUT}/{name}");
            // Descriptors are independent; computed on the pool, first error in path order.
            let datasets = self
                .ordered_map(&paths(self, &path)?, |p| {
                    stored::inventory_descriptor(self, p, max_bytes)
                })
                .into_iter()
                .collect::<Result<Vec<_>>>()?;
            out.push(OutputBlock {
                name,
                path,
                datasets,
            });
        }
        Ok(out)
    }
    /// Read one exact relative dataset in a non-Base block, retaining stored type/shape.
    /// Numeric selections bound returned bytes; compound/text reads bound raw stored bytes.
    pub fn read_output_block(
        &self,
        block: &str,
        relative: &str,
        slices: Option<&[Slice]>,
        max_bytes: usize,
    ) -> Result<StoredValues> {
        if matches!(block, "Base Output" | "." | "..")
            || block.is_empty()
            || block.contains(['/', '\\'])
            || relative.is_empty()
            || relative
                .split('/')
                .any(|n| n.is_empty() || n == "." || n == ".." || n.contains('\\'))
        {
            return Err(invalid(
                "expected exact non-Base block and relative dataset path",
            ));
        }
        let d = stored::descriptor(self, &format!("{OUTPUT}/{block}/{relative}"), max_bytes)?;
        stored::read(self, &d, slices, max_bytes)
    }
    /// Family inventories report canonical absence separately from unsupported layouts.
    /// Unknown/no-fixture families never become Supported just because a group exists.
    pub fn read_family_inventory(
        &self,
        family: ReadFamily,
        max_bytes: usize,
    ) -> Result<FamilyInventory> {
        if super::environment::handles(family) {
            return self.environmental_inventory(family, max_bytes);
        }
        if family == ReadFamily::TerrainModifications {
            return self.terrain_modification_inventory(max_bytes);
        }
        let mut out = FamilyInventory {
            family,
            capability: Capability::Supported,
            datasets: Vec::new(),
            groups: Vec::new(),
        };
        if matches!(
            family,
            ReadFamily::PipeRunSummary
                | ReadFamily::ReferenceSummaries
                | ReadFamily::LandCover
                | ReadFamily::Infiltration
                | ReadFamily::Sediment
                | ReadFamily::Encroachments
                | ReadFamily::Terrain
        ) {
            out.capability = refuse();
            return Ok(out);
        }
        let roots: Vec<&str> = match family {
            ReadFamily::PipeGeometry => vec![
                "Geometry/Pipe Networks",
                "Geometry/Pipe Conduits",
                "Geometry/Pipe Nodes",
            ],
            ReadFamily::PlanMetadata => vec![
                "Plan Data",
                "Results/Summary",
                "Results/Unsteady/Summary",
                "Results/Steady/Summary",
            ],
            ReadFamily::Breach => vec![OUTPUT, "Plan Data/Breach Data"],

            _ => vec![OUTPUT],
        };
        for root in roots {
            let members: Vec<String> = paths(self, root)?
                .into_iter()
                .filter(|path| root != OUTPUT || result_family(path, family))
                .collect();
            // Descriptors are independent; computed on the pool, then handled in path order
            // exactly as the sequential loop did (the first propagated error wins).
            let results = self.ordered_map(&members, |path| {
                self.family_descriptor(family, path, max_bytes)
            });
            for (path, result) in members.into_iter().zip(results) {
                let d = match result {
                    Ok(d) => d,
                    Err(e) => {
                        let cap = error_to_capability(e)?;
                        let mut stored = if cap.code() == Some(UnsupportedCode::ResourceLimit) {
                            stored::refused_descriptor(self, &path, cap.clone())?
                        } else {
                            stored::inventory_descriptor(self, &path, max_bytes)?
                        };
                        stored.capability = cap;
                        FamilyDataset {
                            stored,
                            role: "unqualified".into(),
                            domain: "unknown".into(),
                            time_group: None,
                            identity_table: None,
                            locations: Vec::new(),
                            names: Vec::new(),
                            geometry_locations: Vec::new(),
                            stored_location_ids: None,
                            time_of_extreme_row: None,
                        }
                    }
                };
                out.datasets.push(d);
            }
            if family == ReadFamily::PlanMetadata {
                self.family_group_attributes(root, max_bytes, &mut out.groups)?;
            }
        }
        if family == ReadFamily::PlanMetadata {
            for path in [
                "Results",
                "Results/Unsteady",
                "Results/Steady",
                "Results/Steady/Output",
            ] {
                if absent_ok(self.file.group(path).map_err(map_error))?.is_some() {
                    out.groups.push(FamilyAttributes {
                        id: Identity::new(self.path(), path, 0),
                        path: path.into(),
                        attributes: stored::attributes(self, path, true, max_bytes)?,
                    });
                }
            }
            out.groups.push(FamilyAttributes {
                id: Identity::new(self.path(), "/", 0),
                path: "/".into(),
                attributes: stored::attributes(self, "/", true, max_bytes)?,
            });
        }
        if out.datasets.is_empty() && out.groups.is_empty() {
            out.capability = Capability::unavailable("canonical read family absent");
        }
        Ok(out)
    }
    /// Resolve the descriptor afresh by an exact inventory path; caller-edited descriptors
    /// cannot bypass qualification. Requested reads do not decode unrelated siblings.
    pub fn read_family_dataset(
        &self,
        family: ReadFamily,
        path: &str,
        slices: Option<&[Slice]>,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> Result<StoredValues> {
        if super::environment::handles(family) {
            return self.environmental_read(family, path, slices, metadata_max_bytes, max_bytes);
        }
        if family == ReadFamily::TerrainModifications {
            return self.terrain_modification_read(path, slices, metadata_max_bytes, max_bytes);
        }
        if (family == ReadFamily::PipeGeometry && path.starts_with("Geometry/Pipe Inlets/"))
            || (family == ReadFamily::PipeResults
                && path.starts_with("Results/Unsteady/Summary/Pipe Network"))
        {
            return Err(Error::unsupported(
                UnsupportedCode::Schema,
                "pipe layout has no qualified fixture",
            ));
        }
        if family == ReadFamily::ReferencePoints
            && path.contains("/Reference Points/")
            && path.contains("/Summary Output/")
        {
            return Err(Error::unsupported(
                UnsupportedCode::Schema,
                "reference-point summary layout unqualified",
            ));
        }
        let belongs = match family {
            ReadFamily::PipeGeometry => [
                "Geometry/Pipe Networks/",
                "Geometry/Pipe Conduits/",
                "Geometry/Pipe Nodes/",
            ]
            .iter()
            .any(|p| path.starts_with(p)),
            ReadFamily::PipeResults
            | ReadFamily::ReferenceLines
            | ReadFamily::ReferencePoints
            | ReadFamily::Breach => {
                (path.starts_with(&format!("{OUTPUT}/")) && result_family(path, family))
                    || (family == ReadFamily::Breach && path.starts_with("Plan Data/Breach Data/"))
            }
            ReadFamily::PlanMetadata => [
                "Plan Data/",
                "Results/Summary/",
                "Results/Unsteady/Summary/",
                "Results/Steady/Summary/",
            ]
            .iter()
            .any(|p| path.starts_with(p)),
            _ => {
                return Err(Error::unsupported(
                    UnsupportedCode::Schema,
                    "unqualified layout: this family has no qualified reader yet",
                ))
            }
        };
        if !belongs
            || path
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == ".." || s.contains('\\'))
        {
            return Err(invalid("dataset path outside selected family"));
        }
        let d = self.family_descriptor(family, path, metadata_max_bytes)?;
        stored::read(self, &d.stored, slices, max_bytes)
    }
    pub(super) fn read_family_layers(
        &self,
        max_bytes: usize,
    ) -> Result<Vec<super::layers::LayerDescriptor>> {
        let mut out = Vec::new();
        for &family in READ_FAMILIES {
            let inv = match self.read_family_inventory(family, max_bytes) {
                Ok(v) => v,
                Err(e) => {
                    out.push(super::layers::layer(
                        family.as_str(),
                        family.as_str(),
                        "family",
                        "none",
                        error_to_capability(e)?,
                    ));
                    continue;
                }
            };
            let mut layer = super::layers::layer(
                family.as_str(),
                family.as_str(),
                "family",
                "table",
                inv.capability,
            );
            layer.fields = inv
                .datasets
                .into_iter()
                .map(|d| super::layers::LayerField {
                    name: d.stored.path.clone(),
                    dataset: Some(d.stored.path),
                    numeric_kind: d.stored.kind,
                    row_count: d.stored.shape.first().copied(),
                    capability: d.stored.capability,
                })
                .collect();
            out.push(layer);
        }
        let blocks = match self.output_block_inventory(max_bytes) {
            Ok(v) => v,
            Err(e) => {
                out.push(super::layers::layer(
                    "output_block",
                    "output_blocks",
                    "family",
                    "none",
                    error_to_capability(e)?,
                ));
                Vec::new()
            }
        };
        for block in blocks {
            let mut layer = super::layers::layer(
                "output_block",
                &block.name,
                "family",
                "table",
                Capability::Supported,
            );
            layer.fields = block
                .datasets
                .into_iter()
                .map(|d| super::layers::LayerField {
                    name: d.path.clone(),
                    dataset: Some(d.path),
                    numeric_kind: d.kind,
                    row_count: d.shape.first().copied(),
                    capability: d.capability,
                })
                .collect();
            out.push(layer);
        }
        Ok(out)
    }
    pub(super) fn family_time_rows(&self, group: &str, max: usize) -> Result<u64> {
        // DSS stores second-resolution strings in both stamp datasets in some versions.
        // Validate the raw axes without imposing Base Output's civil-string grammar.
        let time = stored::descriptor(self, &format!("{group}/Time"), max)?;
        if time.shape.len() != 1 {
            return Err(corrupt("block Time must be rank one"));
        }
        let days = time.attributes.iter().find(|a| a.name == "Time").is_some_and(|a| {
            matches!(&a.value, MetadataValue::Strings(v) if v.len() == 1 && v[0].eq_ignore_ascii_case("days"))
        });
        if !days {
            return Err(Error::unsupported(
                UnsupportedCode::Schema,
                "block Time requires an explicit days units attribute",
            ));
        }
        let StoredValues::Numeric(a) = stored::read(self, &time, None, max)? else {
            return Err(corrupt("block Time must be numeric"));
        };
        // Repeated stored times are retained. No interval, derivative or civil-time semantics.
        let non_decreasing = match a.values {
            super::NumericValues::F64(v) => {
                v.iter().all(|x| x.is_finite()) && v.windows(2).all(|v| v[0] <= v[1])
            }
            super::NumericValues::F32(v) => {
                v.iter().all(|x| x.is_finite()) && v.windows(2).all(|v| v[0] <= v[1])
            }
            _ => {
                return Err(Error::unsupported(
                    UnsupportedCode::Encoding,
                    "block Time requires stored float days",
                ))
            }
        };
        if !non_decreasing {
            return Err(corrupt("block Time must be finite and non-decreasing"));
        }
        for name in ["Time Date Stamp", "Time Date Stamp (ms)"] {
            let path = format!("{group}/{name}");
            if let Some(ds) = absent_ok(self.file.dataset(&path).map_err(map_error))? {
                if ds.shape() != time.shape {
                    return Err(corrupt("block timestamp length mismatch"));
                }
                stored::budget(ds.raw_byte_len().map_err(map_error)?, max)?;
                self.read_strings(&path)?;
            }
        }
        Ok(time.shape[0])
    }
    fn family_group_attributes(
        &self,
        root: &str,
        max: usize,
        out: &mut Vec<FamilyAttributes>,
    ) -> Result<()> {
        if absent_ok(self.file.group(root).map_err(map_error))?.is_none() {
            return Ok(());
        }
        let mut pending = vec![(root.to_owned(), 0usize)];
        while let Some((path, depth)) = pending.pop() {
            if depth > 24 || out.len() + pending.len() > 100_000 {
                return Err(Error::unsupported(
                    UnsupportedCode::ResourceLimit,
                    "metadata hierarchy limit",
                ));
            }
            let attributes = stored::attributes(self, &path, true, max)?;
            out.push(FamilyAttributes {
                id: Identity::new(self.path(), &path, 0),
                path: path.clone(),
                attributes,
            });
            pending.extend(
                self.group_members(&path)?
                    .groups
                    .into_iter()
                    .map(|n| (format!("{path}/{n}"), depth + 1)),
            );
        }
        Ok(())
    }
    fn qualify_pipe_geometry(&self, path: &str, max: usize) -> Result<()> {
        let (parent, name) = path
            .rsplit_once('/')
            .ok_or_else(|| invalid("pipe geometry path"))?;
        let known = matches!(
            name,
            "Attributes"
                | "Polyline Info"
                | "Polyline Parts"
                | "Polyline Points"
                | "Terrain Profiles Info"
                | "Terrain Profiles Values"
                | "Face Shape Points"
                | "Face Shape Points Info"
                | "Face Shape Points Values"
                | "Points Info"
                | "Points Points"
                | "Points"
                | "Cell Polygons Info"
                | "Cell Polygons Parts"
                | "Cell Polygons Points"
                | "Face Polylines Info"
                | "Face Polylines Parts"
                | "Face Polylines Points"
                | "Cell Property Table"
                | "Cell Property Table Info"
                | "Cell Property Table Values"
                | "Face Property Table"
                | "Face Property Table Info"
                | "Face Property Table Values"
                | "Cells DS Face Indices Info"
                | "Cells DS Face Indices Values"
                | "Cells Face Indices Info"
                | "Cells Face Indices Values"
                | "Cells US Face Indices Info"
                | "Cells US Face Indices Values"
                | "Cells Minimum Elevations"
                | "Cells Node and Conduit IDs"
                | "Conduit Indices"
                | "Faces Conduit ID and Stations"
                | "Node Connectivity Info"
                | "Node Connectivity Values"
                | "Node Indices"
                | "Node Surface Connectivity"
        );
        if !known {
            return Err(Error::unsupported(
                UnsupportedCode::Schema,
                "unqualified pipe geometry dataset",
            ));
        }
        // Validate both halves of every packed range before returning any selected half.
        for prefix in [
            "Polyline",
            "Cell Polygons",
            "Face Polylines",
            "Terrain Profiles",
            "Cell Property Table",
            "Face Property Table",
            "Cells DS Face Indices",
            "Cells Face Indices",
            "Cells US Face Indices",
            "Node Connectivity",
            "Face Shape Points",
            "Points",
        ] {
            if !name.starts_with(&format!("{prefix} ")) {
                continue;
            }
            let packed = matches!(prefix, "Polyline" | "Cell Polygons" | "Face Polylines");
            let info_path = format!("{parent}/{prefix} Info");
            let values_path = format!(
                "{parent}/{prefix} {}",
                if packed || prefix == "Points" {
                    "Points"
                } else {
                    "Values"
                }
            );
            let info = stored::descriptor(self, &info_path, max)?;
            let values = self.file.dataset(&values_path).map_err(map_error)?;
            if info.shape.len() != 2 || info.shape[1] != if packed { 4 } else { 2 } {
                return Err(corrupt("pipe packed Info shape"));
            }
            let StoredValues::Numeric(a) = stored::read(self, &info, None, max)? else {
                return Err(corrupt("pipe Info must be numeric"));
            };
            let super::NumericValues::I32(v) = a.values else {
                return Err(Error::unsupported(
                    UnsupportedCode::Encoding,
                    "pipe packed offsets require i32",
                ));
            };
            let bound = values
                .shape()
                .first()
                .copied()
                .ok_or_else(|| corrupt("pipe values rank"))?;
            let check = |start: i32, count: i32, bound: u64| -> Result<()> {
                let start = u64::try_from(start).map_err(|_| corrupt("negative pipe offset"))?;
                let count = u64::try_from(count).map_err(|_| corrupt("negative pipe count"))?;
                if start.checked_add(count).is_none_or(|end| end > bound) {
                    return Err(corrupt("pipe packed range outside values"));
                }
                Ok(())
            };
            if packed {
                let parts = stored::descriptor(self, &format!("{parent}/{prefix} Parts"), max)?;
                if parts.shape.len() != 2 || parts.shape[1] != 2 {
                    return Err(corrupt("pipe packed Parts shape"));
                }
                if values.shape().len() != 2 || !(2..=3).contains(&values.shape()[1]) {
                    return Err(corrupt("pipe coordinate width"));
                }
                let StoredValues::Numeric(a) = stored::read(self, &parts, None, max)? else {
                    return Err(corrupt("pipe Parts must be numeric"));
                };
                let super::NumericValues::I32(p) = a.values else {
                    return Err(Error::unsupported(
                        UnsupportedCode::Encoding,
                        "pipe parts require i32",
                    ));
                };
                for row in v.as_chunks::<4>().0.iter() {
                    check(row[0], row[1], bound)?;
                    check(row[2], row[3], parts.shape[0])?;
                    for part in p
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .skip(row[2] as usize)
                        .take(row[3] as usize)
                    {
                        check(part[0], part[1], row[1] as u64)?;
                    }
                }
            } else {
                for row in v.as_chunks::<2>().0.iter() {
                    check(row[0], row[1], bound)?;
                }
            }
        }
        Ok(())
    }
    fn family_descriptor(
        &self,
        family: ReadFamily,
        path: &str,
        max: usize,
    ) -> Result<FamilyDataset> {
        let stored = stored::descriptor(self, path, max)?;
        let mut d = FamilyDataset {
            stored,
            role: "metadata".into(),
            domain: "stored".into(),
            time_group: None,
            identity_table: None,
            locations: Vec::new(),
            names: Vec::new(),
            geometry_locations: Vec::new(),
            stored_location_ids: None,
            time_of_extreme_row: None,
        };
        if family == ReadFamily::ReferencePoints {
            self.qualify_reference_point_series(path, &mut d, max)?;
            return Ok(d);
        }
        if family == ReadFamily::PipeGeometry {
            self.qualify_pipe_geometry(path, max)?;
            d.role = if d.stored.storage == "compound" {
                "stored_table"
            } else {
                "geometry_array"
            }
            .into();
            return Ok(d);
        }
        if family == ReadFamily::PlanMetadata || path.starts_with("Plan Data/") {
            return Ok(d);
        }
        if d.stored.storage != "numeric" {
            if path.ends_with("/Name") && family == ReadFamily::ReferenceLines {
                d.role = "name_axis".into();
                return Ok(d);
            }
            return Err(Error::unsupported(
                UnsupportedCode::Schema,
                "unqualified result storage",
            ));
        }
        if d.stored.shape.len() != 2 {
            return Err(corrupt("result must be rank two"));
        }
        let n = d.stored.shape[1];
        if family == ReadFamily::PipeResults {
            let (_, tail) = path
                .split_once("/Pipe Networks/")
                .ok_or_else(|| corrupt("pipe network path"))?;
            let (network, _var) = tail
                .split_once('/')
                .ok_or_else(|| corrupt("pipe variable path"))?;
            let base = format!("Geometry/Pipe Networks/{network}");
            let (domain, identity) = super::location::pipe(&d.stored.attributes)?;
            let table = format!("{base}/{identity}");
            let ids_table =
                absent_ok(self.file.dataset(&table).map_err(map_error))?.ok_or_else(|| {
                    Error::unsupported(
                        UnsupportedCode::IdentityLayout,
                        "pipe geometry table for stored Location absent",
                    )
                })?;
            let shape = ids_table.shape().to_vec();
            let rows = shape
                .first()
                .copied()
                .ok_or_else(|| corrupt("pipe identity table rank"))?;
            if n != rows {
                return Err(corrupt("known pipe Location contradicts geometry count"));
            }
            d.domain = domain.into();
            d.identity_table = Some(table.clone());
            if matches!(domain, "node" | "conduit") {
                if shape != [n] {
                    return Err(corrupt("pipe stored ID table must be rank one"));
                }
                let id_descriptor = stored::descriptor(self, &table, max)?;
                let StoredValues::Numeric(ids) = stored::read(self, &id_descriptor, None, max)?
                else {
                    return Err(Error::unsupported(
                        UnsupportedCode::IdentityLayout,
                        "pipe ID table must store exact integers",
                    ));
                };
                if matches!(
                    &ids.values,
                    super::NumericValues::F32(_) | super::NumericValues::F64(_)
                ) {
                    return Err(Error::unsupported(
                        UnsupportedCode::IdentityLayout,
                        "pipe ID table must store exact integers",
                    ));
                }
                d.stored_location_ids = Some(ids);
            }
        } else if family == ReadFamily::ReferenceLines {
            let parent = path
                .rsplit_once('/')
                .ok_or_else(|| corrupt("reference parent namespace absent"))?
                .0;
            let table = format!("{parent}/Name");
            let names_ds = self.file.dataset(&table).map_err(map_error)?;
            stored::budget(names_ds.raw_byte_len().map_err(map_error)?, max)?;
            d.names = self.read_strings(&table)?;
            if names_ds.shape() != [n] || d.names.len() as u64 != n {
                return Err(corrupt("reference names and result columns disagree"));
            }
            d.identity_table = Some(table);
            d.domain = "reference_line".into();
        } else {
            d.domain = "breach_variable".into();
        }
        if family == ReadFamily::Breach {
            let a = d
                .stored
                .attributes
                .iter()
                .find(|a| a.name == "Variable_Unit")
                .ok_or_else(|| {
                    Error::unsupported(
                        UnsupportedCode::Schema,
                        "breach Variable_Unit labels absent",
                    )
                })?;
            if a.shape != [n, 2] {
                return Err(corrupt("breach variable labels disagree with columns"));
            }
            if let MetadataValue::Strings(v) = &a.value {
                d.names = v.as_chunks::<2>().0.iter().map(|v| v[0].clone()).collect();
            } else {
                return Err(Error::unsupported(
                    UnsupportedCode::Encoding,
                    "breach variable labels unqualified",
                ));
            }
        }
        if let Some(table) = &d.identity_table {
            bytes_for_identities(n, max)?;
            d.locations = (0..n)
                .map(|row| Identity::new(self.path(), table, row))
                .collect();
        }
        if path.contains("/Summary Output/") {
            if !(1..=2).contains(&d.stored.shape[0]) {
                return Err(corrupt("summary requires one or two stored rows"));
            }
            d.role = "summary_rows".into();
            for a in &d.stored.attributes {
                if a.name == "Rows Variables" {
                    if let MetadataValue::Strings(v) = &a.value {
                        if d.stored.shape[0] > 1 && v.get(1).is_some_and(|s| s == "Time") {
                            d.time_of_extreme_row = Some(1);
                        }
                    }
                }
            }
        } else {
            let (prefix, _) = path.split_once("/Unsteady Time Series/").ok_or_else(|| {
                Error::unsupported(
                    UnsupportedCode::Schema,
                    "unqualified non-series result layout",
                )
            })?;
            let group = format!("{prefix}/Unsteady Time Series");
            let time_rows = self.family_time_rows(&group, max)?;
            if d.stored.shape[0] == time_rows {
                d.role = if family == ReadFamily::Breach {
                    "breach_progression"
                } else {
                    "time_series"
                }
                .into();
                d.time_group = Some(group);
            } else if family == ReadFamily::ReferenceLines && d.stored.shape[0] == 1 {
                d.role = "one_row_unqualified".into();
            } else {
                return Err(corrupt("result and block-specific time rows disagree"));
            }
        }
        Ok(d)
    }
}

#[cfg(test)]
mod walk_tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/stored_walk")
            .join(name)
    }
    /// One-thread (sequential), parallel, and parallel-disabled sessions.
    fn sessions(name: &str) -> Vec<ReadOnlyHdfSession> {
        vec![
            ReadOnlyHdfSession::with_options(fixture(name), 1, true).unwrap(),
            ReadOnlyHdfSession::with_options(fixture(name), 8, true).unwrap(),
            ReadOnlyHdfSession::with_options(fixture(name), 8, false).unwrap(),
        ]
    }
    fn render<T: std::fmt::Debug>(r: &Result<T>) -> String {
        match r {
            Ok(v) => format!("Ok({v:?})"),
            Err(e) => format!("Err({:?} {:?} {})", e.kind, e.code, e),
        }
    }

    #[test]
    fn family_and_block_inventories_do_not_depend_on_the_pool() {
        for name in ["stored_walk.h5", "stored_walk_deep.h5"] {
            let all = sessions(name);
            assert!(all[1].parallel_metadata() && !all[0].parallel_metadata());
            let mut reference: Option<Vec<String>> = None;
            for s in &all {
                let mut out: Vec<String> = READ_FAMILIES
                    .iter()
                    .map(|&f| render(&s.read_family_inventory(f, 64 << 20)))
                    .collect();
                out.push(render(&s.output_block_inventory(64 << 20)));
                // A second pass on the same session (memo and prefetch warm) is unchanged.
                out.push(render(&s.output_block_inventory(64 << 20)));
                match &reference {
                    None => reference = Some(out),
                    Some(r) => assert_eq!(r, &out, "{name}"),
                }
            }
        }
    }

    #[test]
    fn inventories_from_rayon_workers_stay_sequential_and_identical() {
        use rayon::prelude::*;
        let inventory = |s: &ReadOnlyHdfSession| -> Vec<String> {
            let mut out: Vec<String> = READ_FAMILIES
                .iter()
                .map(|&f| render(&s.read_family_inventory(f, 64 << 20)))
                .collect();
            out.push(render(&s.output_block_inventory(64 << 20)));
            out
        };
        let reference = inventory(&sessions("stored_walk.h5")[0]);
        let foreign = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap();
        // Fresh sessions, so first calls race to initialize the same memo cells.
        let own = &sessions("stored_walk.h5")[1];
        own.pool.install(|| {
            assert!(!own.parallel_metadata());
            (0..8)
                .into_par_iter()
                .for_each(|_| assert_eq!(inventory(own), reference));
        });
        let shared = &sessions("stored_walk.h5")[1];
        foreign.install(|| {
            assert!(!shared.parallel_metadata());
            (0..8)
                .into_par_iter()
                .for_each(|_| assert_eq!(inventory(shared), reference));
        });
    }

    #[test]
    fn hierarchy_limits_refuse_identically_with_and_without_the_pool() {
        // Depth beyond 24 under Output Blocks: every walk of it is refused.
        for s in sessions("stored_walk_deep.h5") {
            let e = s.output_block_inventory(64 << 20).unwrap_err();
            assert_eq!(e.kind, ErrorKind::Unsupported);
            assert_eq!(e.code, Some(UnsupportedCode::ResourceLimit));
            assert_eq!(
                e.to_string(),
                Error::unsupported(
                    UnsupportedCode::ResourceLimit,
                    "stored family hierarchy limit"
                )
                .to_string()
            );
            assert!(paths(&s, OUTPUT).is_err());
        }
        // The entry limit: every bound around the tree size gives the same outcome.
        let total = paths(&sessions("stored_walk.h5")[0], OUTPUT).unwrap().len();
        let (groups, entries) = (
            0..=3usize,
            [
                0,
                1,
                2,
                10,
                50,
                total - 1,
                total,
                total + 1,
                total + 60,
                MAX_WALK_ENTRIES,
            ],
        );
        for depth in groups.chain([MAX_WALK_DEPTH]) {
            for max in entries {
                let outcomes: Vec<String> = sessions("stored_walk.h5")
                    .iter()
                    .map(|s| render(&paths_bounded(s, OUTPUT, max, depth)))
                    .collect();
                assert!(
                    outcomes.windows(2).all(|w| w[0] == w[1]),
                    "depth {depth} max {max}: {outcomes:?}"
                );
                // A session whose memo the prefetch already warmed gives the same outcome too.
                let warm = &sessions("stored_walk.h5")[1];
                paths(warm, OUTPUT).unwrap();
                assert_eq!(
                    render(&paths_bounded(warm, OUTPUT, max, depth)),
                    outcomes[0]
                );
            }
        }
        let s = &sessions("stored_walk.h5")[1];
        let e = paths_bounded(s, OUTPUT, 10, MAX_WALK_DEPTH).unwrap_err();
        assert_eq!(
            (e.kind, e.code),
            (ErrorKind::Unsupported, Some(UnsupportedCode::ResourceLimit))
        );
        assert!(e.to_string().contains("stored family hierarchy limit"));
        assert!(paths_bounded(s, OUTPUT, total, MAX_WALK_DEPTH).is_ok());
    }
}
