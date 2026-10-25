//! RASMapper terrain-modification attribute tables in a terrain HDF.
//!
//! `Modifications/<group>/Attributes` (and the optional
//! `Modifications/<group>/Control Points/Attributes`) are one-dimensional compound tables of
//! fixed-length text, little-endian `f4` and unsigned byte members. The group's stored `Type`
//! attribute selects the member layouts observed for it; the table is qualified only when
//! its stored datatype is exactly one of them (names, order, packed offsets, widths, byte
//! order) and its row count equals the group's feature table. Values are returned as the exact
//! stored compound bytes with per-member type evidence; no text is trimmed, no number is
//! converted and no geometry is built. Geometry arrays of the same groups remain literal
//! generic-reader data and are not part of this family.
use super::{
    encoding::map_error,
    families::{FamilyAttributes, FamilyDataset, FamilyInventory, ReadFamily},
    metadata::MetadataValue,
    metadata_proof::{self as proof, Proof},
    stored::{self, StoredValues},
    AttributeType, ByteOrder, Identity, ReadOnlyHdfSession, Slice, StringCharset,
};
use crate::{
    capability::{absent_ok, error_to_capability},
    Capability, Error, ErrorKind, Result, UnsupportedCode,
};

pub(super) const ROOT: &str = "Modifications";
const CONTROL: &str = "Control Points";
/// Refuse hierarchies wider than any plausible terrain before decoding them.
const MAX_GROUPS: usize = 100_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Member {
    /// Fixed-length text of any proven padding and width (ASCII charset).
    Text,
    /// IEEE little-endian 4-byte float.
    F4,
    /// Unsigned 1-byte integer.
    U1,
}
use Member::{Text, F4, U1};

/// One observed table layout and the sibling dataset whose rows it describes.
struct Layout {
    role: &'static str,
    members: &'static [(&'static str, Member)],
    feature_table: &'static str,
}
const POLYLINE: Layout = Layout {
    role: "polyline_modification_attributes",
    members: &[
        ("Name", Text),
        ("Elevation Type", Text),
        ("Top Elevation", F4),
        ("Top Width", F4),
        ("Left Slope", F4),
        ("Right Slope", F4),
        ("Max Reach", F4),
        ("Elev Pt Tolerance", F4),
    ],
    feature_table: "Polyline Info",
};
/// The polyline layout of HEC-RAS 6.4.1-6.6 era terrains (and of ras-commander's
/// `RasTerrainModWriter`): a leading `System Name` and a `Transition Percent` before
/// `Elev Pt Tolerance`.
const POLYLINE_TRANSITION: Layout = Layout {
    role: "polyline_modification_attributes",
    members: &[
        ("System Name", Text),
        ("Name", Text),
        ("Elevation Type", Text),
        ("Top Elevation", F4),
        ("Top Width", F4),
        ("Left Slope", F4),
        ("Right Slope", F4),
        ("Max Reach", F4),
        ("Transition Percent", F4),
        ("Elev Pt Tolerance", F4),
    ],
    feature_table: "Polyline Info",
};
const POLYGON: Layout = Layout {
    role: "polygon_modification_attributes",
    members: &[
        ("Name", Text),
        ("Elevation Value", F4),
        ("Elevation Type", Text),
        ("Elev Pt Tolerance", F4),
        ("Generate Boundary Elevations", U1),
        ("Use ShapeFile Z Elevations", U1),
    ],
    feature_table: "Polygon Info",
};
const CIRCLE: Layout = Layout {
    role: "circle_modification_attributes",
    members: POINT_MEMBERS,
    feature_table: "Circles",
};
/// Point-feature layouts share the circle member list; only the feature table differs.
const POINT_MEMBERS: &[(&str, Member)] = &[
    ("Name", Text),
    ("Elevation Value", F4),
    ("Elevation Type", Text),
];
const PIER: Layout = Layout {
    role: "pier_modification_attributes",
    members: POINT_MEMBERS,
    feature_table: "Piers",
};
const RECTANGLE: Layout = Layout {
    role: "rectangle_modification_attributes",
    members: POINT_MEMBERS,
    feature_table: "Rectangles",
};
const CONTROL_POINTS: Layout = Layout {
    role: "control_point_attributes",
    members: &[("Name", Text), ("Elevation", F4)],
    feature_table: "Points",
};

fn corrupt(m: impl Into<String>) -> Error {
    Error::new(ErrorKind::Corrupt, m)
}
fn unsupported(m: impl Into<String>) -> Error {
    Error::unsupported(UnsupportedCode::Schema, m)
}
fn invalid(m: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidRequest, m)
}

/// The observed `Type` spellings and the Attributes layouts observed under each. RASMapper
/// stores polyline groups (Channel and Levee subtypes alike) with `Type = Levee`, in the
/// 8-member layout or the 10-member transition layout. `Type = Channel` is written only by
/// ras-commander's `RasTerrainModWriter.add_channel_modification`, always in the transition
/// layout, so only that layout is qualified for it.
fn layouts_for_type(kind: &str) -> Option<&'static [&'static Layout]> {
    match kind {
        "Levee" => Some(&[&POLYLINE, &POLYLINE_TRANSITION]),
        "Channel" => Some(&[&POLYLINE_TRANSITION]),
        "Polygon" => Some(&[&POLYGON]),
        "Circle" => Some(&[&CIRCLE]),
        "Pier" => Some(&[&PIER]),
        "Rectangle" => Some(&[&RECTANGLE]),
        _ => None,
    }
}

/// `(group, control)` for an exact family dataset path, or `None` when the path is not an
/// Attributes table of one modification group.
fn split(path: &str) -> Option<(&str, bool)> {
    let rest = path.strip_prefix(ROOT)?.strip_prefix('/')?;
    let parts: Vec<&str> = rest.split('/').collect();
    match parts.as_slice() {
        [group, "Attributes"] => Some((group, false)),
        [group, CONTROL, "Attributes"] => Some((group, true)),
        _ => None,
    }
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != ".." && !s.contains('\\'))
}

fn dataset(
    stored: stored::StoredDataset,
    role: &str,
    identity_table: Option<String>,
) -> FamilyDataset {
    FamilyDataset {
        stored,
        role: role.into(),
        domain: "terrain_modification".into(),
        time_group: None,
        identity_table,
        locations: Vec::new(),
        names: Vec::new(),
        geometry_locations: Vec::new(),
        geometry_link: crate::Capability::unavailable("no geometry link for this family"),
        stored_location_ids: None,
        time_of_extreme_row: None,
    }
}

impl ReadOnlyHdfSession {
    /// Exact Attributes table paths of every modification group, sorted, plus the groups
    /// whose attributes the inventory reports. A missing `Modifications` group is `None`.
    fn modification_paths(&self) -> Result<Option<(Vec<String>, Vec<String>)>> {
        let Some(members) = absent_ok(self.group_members(ROOT))? else {
            return Ok(None);
        };
        if members.groups.len() > MAX_GROUPS {
            return Err(Error::unsupported(
                UnsupportedCode::ResourceLimit,
                "terrain modification group limit",
            ));
        }
        let mut tables = Vec::new();
        let mut groups = vec![ROOT.to_owned()];
        for name in members.groups {
            let group = format!("{ROOT}/{name}");
            let inner = self.group_members(&group)?;
            groups.push(group.clone());
            if inner.datasets.iter().any(|d| d == "Attributes") {
                tables.push(format!("{group}/Attributes"));
            }
            if inner.groups.iter().any(|g| g == CONTROL) {
                let control = format!("{group}/{CONTROL}");
                groups.push(control.clone());
                if self
                    .group_members(&control)?
                    .datasets
                    .iter()
                    .any(|d| d == "Attributes")
                {
                    tables.push(format!("{control}/Attributes"));
                }
            }
        }
        tables.sort();
        groups.sort();
        Ok(Some((tables, groups)))
    }

    pub(super) fn terrain_modification_inventory(&self, max: usize) -> Result<FamilyInventory> {
        let mut out = FamilyInventory {
            family: ReadFamily::TerrainModifications,
            capability: Capability::Supported,
            datasets: Vec::new(),
            groups: Vec::new(),
        };
        let Some((tables, groups)) = self.modification_paths()? else {
            out.capability = Capability::unavailable("terrain modifications absent");
            return Ok(out);
        };
        for path in groups {
            out.groups.push(FamilyAttributes {
                id: Identity::new(self.path(), &path, 0),
                path: path.clone(),
                attributes: stored::attributes(self, &path, true, max)?,
            });
        }
        for path in tables {
            let mut stored = stored::inventory_descriptor(self, &path, max)?;
            let (role, table) = if stored.capability.is_supported() {
                match self.terrain_modification_qualify(&path, max) {
                    Ok((role, table)) => (role, Some(table)),
                    Err(e) => {
                        stored.capability = error_to_capability(e)?;
                        ("unqualified", None)
                    }
                }
            } else {
                ("unqualified", None)
            };
            out.datasets.push(dataset(stored, role, table));
        }
        if out.datasets.is_empty() {
            out.capability = Capability::unavailable("no terrain modification attribute tables");
        }
        Ok(out)
    }

    pub(super) fn terrain_modification_read(
        &self,
        path: &str,
        slices: Option<&[Slice]>,
        metadata_max: usize,
        max: usize,
    ) -> Result<StoredValues> {
        if !valid_path(path) || split(path).is_none() {
            return Err(invalid(
                "expected Modifications/<group>/Attributes or Modifications/<group>/Control Points/Attributes",
            ));
        }
        if slices.is_some() {
            return Err(invalid(
                "terrain modification attribute tables are read whole",
            ));
        }
        if !self.has_dataset(path)? {
            return Err(Error::new(
                ErrorKind::NotFound,
                "dataset absent from terrain modifications",
            )
            .with_subject(path));
        }
        let d = stored::descriptor(self, path, metadata_max)?;
        if !d.capability.is_supported() {
            return stored::read(self, &d, None, max);
        }
        self.terrain_modification_qualify(path, metadata_max)?;
        stored::read(self, &d, None, max)
    }

    /// Qualify one Attributes table: returns its role and the feature table it is aligned
    /// with. A missing, non-text or unobserved `Type` and every unobserved layout are
    /// Unsupported(schema); a row count that differs from the aligned table is Corrupt.
    fn terrain_modification_qualify(
        &self,
        path: &str,
        max: usize,
    ) -> Result<(&'static str, String)> {
        let (group, control) =
            split(path).ok_or_else(|| unsupported("not a modification table"))?;
        let group_path = format!("{ROOT}/{group}");
        let attrs = stored::attributes(self, &group_path, true, max)?;
        let kind = attrs
            .iter()
            .find(|a| a.name == "Type")
            .ok_or_else(|| unsupported("terrain modification group has no Type"))?;
        let kind = match (&kind.attribute_type, &kind.value) {
            (AttributeType::FixedString { .. }, MetadataValue::Strings(v))
                if kind.shape.is_empty() =>
            {
                v.first().map(String::as_str).unwrap_or_default().to_owned()
            }
            _ => {
                return Err(unsupported(
                    "terrain modification Type is not proven scalar text",
                ))
            }
        };
        let candidates = layouts_for_type(&kind).ok_or_else(|| {
            unsupported(format!("unqualified terrain modification Type {kind:?}"))
        })?;
        let candidates: &[&Layout] = if control {
            &[&CONTROL_POINTS]
        } else {
            candidates
        };
        let table_dir = if control {
            format!("{group_path}/{CONTROL}")
        } else {
            group_path
        };

        let ds = self.file.dataset(path).map_err(map_error)?;
        let shape = ds.shape().to_vec();
        if shape.len() != 1 {
            return Err(unsupported(
                "terrain modification Attributes must be a one-dimensional table",
            ));
        }
        let p = proof::proof(&ds.raw_datatype().map_err(map_error)?)?;
        let layout = select_layout(&p, candidates)?;

        let feature = format!("{table_dir}/{}", layout.feature_table);
        if !self.has_dataset(&feature)? {
            return Err(unsupported(format!(
                "terrain modification feature table {} absent",
                layout.feature_table
            )));
        }
        let rows = self
            .file
            .dataset(&feature)
            .map_err(map_error)?
            .shape()
            .to_vec();
        if rows.len() != 2 || rows[0] != shape[0] {
            return Err(corrupt(format!(
                "terrain modification Attributes rows differ from {}",
                layout.feature_table
            )));
        }
        Ok((layout.role, feature))
    }
}

/// The one candidate layout the stored compound matches exactly. A Type with several observed
/// layouts is told apart by member count alone (the candidates have distinct counts), so a
/// near miss reports the precise defect against the layout of the same count; a count that
/// no candidate has is refused as such.
fn select_layout(p: &Proof, candidates: &[&'static Layout]) -> Result<&'static Layout> {
    let count = match &p.kind {
        proof::Kind::Compound(fields) => fields.len(),
        _ => {
            return Err(unsupported(
                "terrain modification Attributes is not a compound",
            ))
        }
    };
    let layout = candidates
        .iter()
        .find(|l| l.members.len() == count)
        .ok_or_else(|| {
            unsupported(
                "terrain modification Attributes member count differs from every observed layout of its Type",
            )
        })?;
    check_layout(p, layout)?;
    Ok(layout)
}

/// Exact member sequence, packed offsets and total width; every member width/kind proven.
fn check_layout(p: &Proof, layout: &Layout) -> Result<()> {
    let proof::Kind::Compound(fields) = &p.kind else {
        return Err(unsupported(
            "terrain modification Attributes is not a compound",
        ));
    };
    if fields.len() != layout.members.len() {
        return Err(unsupported(
            "terrain modification Attributes member count differs from the observed layout",
        ));
    }
    let mut offset = 0usize;
    for ((name, at, member), (want, kind)) in fields.iter().zip(layout.members) {
        if name != want {
            return Err(unsupported(format!(
                "terrain modification Attributes member {name:?} where {want:?} is expected"
            )));
        }
        if *at != offset {
            return Err(unsupported(format!(
                "terrain modification Attributes member {name:?} is not packed"
            )));
        }
        let ok = match (kind, AttributeType::from_proof(member)) {
            (
                Text,
                AttributeType::FixedString {
                    charset: StringCharset::Ascii,
                    width,
                    ..
                },
            ) => width > 0 && width == member.width,
            (
                F4,
                AttributeType::Float {
                    bytes: 4,
                    order: ByteOrder::Little,
                },
            ) => member.width == 4,
            (
                U1,
                AttributeType::Int {
                    signed: false,
                    bytes: 1,
                    ..
                },
            ) => member.width == 1,
            _ => false,
        };
        if !ok {
            return Err(unsupported(format!(
                "terrain modification Attributes member {name:?} has an unqualified datatype"
            )));
        }
        offset = offset
            .checked_add(member.width)
            .ok_or_else(|| corrupt("terrain modification member width overflow"))?;
    }
    if offset != p.width {
        return Err(unsupported(
            "terrain modification Attributes has unqualified compound padding",
        ));
    }
    Ok(())
}
