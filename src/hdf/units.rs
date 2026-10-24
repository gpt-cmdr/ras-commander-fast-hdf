//! Reported result units and unit-system evidence. Nothing here converts, infers or
//! normalizes a unit: text is returned exactly as proven from the stored attribute.
use super::{
    encoding::map_error,
    metadata::{typed_attribute, MetadataValue},
    xs1d::{STEADY, UNSTEADY},
    ReadOnlyHdfSession,
};
use crate::capability::absent_ok;
use crate::{Result, UnsupportedCode};

/// Result datasets whose `Units` attribute is reported (2D mesh areas).
pub const MESH2D_UNIT_VARIABLES: [&str; 4] =
    ["Water Surface", "Depth", "Velocity", "Face Velocity"];
/// 1D cross-section result variables whose unit evidence is reported.
pub const XS_UNIT_VARIABLES: [&str; 6] = [
    "Water Surface",
    "Flow",
    "Flow Lateral",
    "Flow Volume Cumulative",
    "Velocity Channel",
    "Velocity Total",
];
/// Scalar fixed-string attribute names that real HEC-RAS result datasets use as quantity labels
/// whose text is the unit (e.g. `WSEL = Feet`). Evidence only: the attribute name and raw text
/// are recorded, nothing is inferred or converted. Exactly one must be present to be reported.
pub const KNOWN_QUANTITY_LABELS: [&str; 9] = [
    "WSEL",
    "Velocity",
    "Lateral Flow",
    "Cumulative Volumetric Flow",
    "Depth",
    "Face Velocity",
    "Flow",
    "Water Surface",
    "Stage",
];
const VARIABLE_UNITS: &str = "Variable Units";
const VARIABLE_NAME: &str = "Variable Name";
const VARIABLE_PAIR: &str = "Variable";
const UNITS_ATTRIBUTE: &str = "Units";
const SYSTEM_ATTRIBUTE: &str = "Units System";
/// Groups checked (in order) for a `Units System` attribute, each reported separately.
pub const UNIT_SYSTEM_SOURCES: [&str; 2] = ["/", "/Geometry"];

/// One stored text attribute, exactly as reported, with a single status. Supported: exactly
/// one proven text value (`value()`; `raw_bytes()` is the stored payload). Unavailable: the
/// object exists and carries no such attribute. Unsupported: present but not one proven text
/// value (non-ASCII in ASCII storage, numeric, array...). Corrupt: damaged or duplicated.
pub type UnitsEvidence = crate::Reported<String>;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnitsFamily {
    Mesh2D,
    CrossSectionUnsteady,
    CrossSectionSteady,
}
/// Which stored attribute produced the unit evidence, in priority order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnitsSource {
    /// A scalar `Units` attribute.
    UnitsAttribute,
    /// A scalar `Variable Units` attribute (quantity from `Variable Name` when present).
    VariableUnits,
    /// The two-element `Variable` = [name, unit] attribute.
    VariablePair,
    /// A scalar attribute whose name is a known quantity label (text is the unit).
    QuantityLabel { attribute: String },
    /// No unit-bearing attribute exists.
    None,
}
/// Unit evidence for one dataset with its provenance.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct DatasetUnits {
    pub units: UnitsEvidence,
    pub source: UnitsSource,
    /// Quantity name from `Variable Name` or the first `Variable` element, when stored.
    pub quantity_name: Option<String>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct VariableUnits {
    pub family: UnitsFamily,
    /// 2D flow area name; None for 1D families.
    pub area: Option<String>,
    pub variable: String,
    pub dataset: String,
    pub units: UnitsEvidence,
    pub source: UnitsSource,
    pub quantity_name: Option<String>,
}
/// Exact-match hint only; any other text stays [`UnitSystemKind::Unrecognized`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnitSystemKind {
    /// Stored text is exactly `US Customary`.
    UsCustomary,
    /// Stored text is exactly `SI Metric`.
    SiMetric,
    Unrecognized,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct UnitSystemEvidence {
    /// Group the attribute was read from.
    pub source: String,
    pub value: UnitsEvidence,
    /// Some only when `value` is Declared.
    pub kind: Option<UnitSystemKind>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ResultUnits {
    pub variables: Vec<VariableUnits>,
    /// Reported evidence from each source group; conflicts are left visible, not resolved.
    pub unit_system: Vec<UnitSystemEvidence>,
}
impl UnitsFamily {
    /// Same spellings as the matching `ResultFamily` (`mesh_2d`, `cross_sections_unsteady`,
    /// and `steady_profiles` for the steady XS results).
    /// Stable snake_case spelling (never the Debug form).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mesh2D => "mesh_2d",
            Self::CrossSectionUnsteady => "cross_sections_unsteady",
            Self::CrossSectionSteady => "steady_profiles",
        }
    }
}
impl UnitSystemKind {
    /// Stable snake_case spelling (never the Debug form).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UsCustomary => "us_customary",
            Self::SiMetric => "si_metric",
            Self::Unrecognized => "unrecognized",
        }
    }
}
impl DatasetUnits {
    /// The single status of the unit evidence.
    pub fn capability(&self) -> &crate::Capability {
        self.units.capability()
    }
}
impl VariableUnits {
    /// The single status of the unit evidence.
    pub fn capability(&self) -> &crate::Capability {
        self.units.capability()
    }
}
impl UnitSystemEvidence {
    /// The single status of the stored `Units System` text.
    pub fn capability(&self) -> &crate::Capability {
        self.value.capability()
    }
}
/// Attribute payload over the caller budget: reported, never retained or decoded.
fn over_budget(raw_len: usize, max_bytes: usize, name: &str) -> Option<UnitsEvidence> {
    (raw_len > max_bytes).then(|| {
        UnitsEvidence::unsupported(
            UnsupportedCode::ResourceLimit,
            format!("{name} attribute exceeds max_bytes"),
        )
    })
}
pub(super) fn evidence(
    attrs: &[crate::hdf5_reader::messages::attribute::AttributeMessage],
    name: &str,
    max_bytes: usize,
) -> Result<UnitsEvidence> {
    let mut found = attrs.iter().filter(|a| a.name == name);
    let Some(first) = found.next() else {
        return Ok(UnitsEvidence::unavailable(format!("no {name} attribute")));
    };
    if found.next().is_some() {
        return Ok(
            UnitsEvidence::corrupt(format!("duplicate {name} attribute"))
                .with_raw_bytes(first.raw_data.clone()),
        );
    }
    if let Some(over) = over_budget(first.raw_data.len(), max_bytes, name) {
        return Ok(over);
    }
    let t = typed_attribute(first.clone())?;
    let raw = t.raw_bytes;
    Ok(match t.value {
        MetadataValue::Strings(mut v) if v.len() == 1 && t.shape.iter().product::<u64>() <= 1 => {
            UnitsEvidence::supported(v.remove(0)).with_raw_bytes(raw)
        }
        MetadataValue::Corrupt { reason } => UnitsEvidence::corrupt(reason).with_raw_bytes(raw),
        MetadataValue::Unsupported { code, reason } => {
            UnitsEvidence::unsupported(code, reason).with_raw_bytes(raw)
        }
        _ => UnitsEvidence::unsupported(
            UnsupportedCode::Encoding,
            format!("{name} is not exactly one proven text value"),
        )
        .with_raw_bytes(raw),
    })
}
fn declared(e: &UnitsEvidence) -> Option<String> {
    e.value().cloned()
}
fn unit_evidence_from_attrs(
    attrs: &[crate::hdf5_reader::messages::attribute::AttributeMessage],
    max_bytes: usize,
    allow_labels: bool,
) -> Result<DatasetUnits> {
    let quantity = evidence(attrs, VARIABLE_NAME, max_bytes)?;
    let mut quantity_name = declared(&quantity);
    for (name, source) in [
        (UNITS_ATTRIBUTE, UnitsSource::UnitsAttribute),
        (VARIABLE_UNITS, UnitsSource::VariableUnits),
    ] {
        let units = evidence(attrs, name, max_bytes)?;
        if !units.is_unavailable() {
            return Ok(DatasetUnits {
                units,
                source,
                quantity_name,
            });
        }
    }
    if let Some(a) = attrs.iter().find(|a| a.name == VARIABLE_PAIR) {
        if let Some(over) = over_budget(a.raw_data.len(), max_bytes, VARIABLE_PAIR) {
            return Ok(DatasetUnits {
                units: over,
                source: UnitsSource::VariablePair,
                quantity_name,
            });
        }
        let t = typed_attribute(a.clone())?;
        let raw = t.raw_bytes;
        let units = match t.value {
            MetadataValue::Strings(mut v) if v.len() == 2 && t.shape == vec![2] => {
                quantity_name = quantity_name.or_else(|| Some(v[0].clone()));
                UnitsEvidence::supported(v.remove(1)).with_raw_bytes(raw)
            }
            MetadataValue::Corrupt { reason } => UnitsEvidence::corrupt(reason).with_raw_bytes(raw),
            MetadataValue::Unsupported { code, reason } => {
                UnitsEvidence::unsupported(code, reason).with_raw_bytes(raw)
            }
            _ => UnitsEvidence::unsupported(
                UnsupportedCode::Encoding,
                "Variable is not a two-element fixed-text pair",
            )
            .with_raw_bytes(raw),
        };
        return Ok(DatasetUnits {
            units,
            source: UnitsSource::VariablePair,
            quantity_name,
        });
    }
    let labels: Vec<&str> = KNOWN_QUANTITY_LABELS
        .iter()
        .copied()
        .filter(|l| allow_labels && attrs.iter().any(|a| a.name == *l))
        .collect();
    match labels.as_slice() {
        [] => Ok(DatasetUnits {
            units: UnitsEvidence::unavailable("no unit-bearing attribute"),
            source: UnitsSource::None,
            quantity_name,
        }),
        [one] => Ok(DatasetUnits {
            units: evidence(attrs, one, max_bytes)?,
            source: UnitsSource::QuantityLabel {
                attribute: (*one).into(),
            },
            quantity_name,
        }),
        many => {
            let first = attrs.iter().find(|a| a.name == many[0]).unwrap();
            Ok(DatasetUnits {
                units: UnitsEvidence::unsupported(
                    UnsupportedCode::Schema,
                    format!("ambiguous quantity-label attributes: {many:?}"),
                )
                .with_raw_bytes(first.raw_data.clone()),
                source: UnitsSource::None,
                quantity_name,
            })
        }
    }
}
/// The quantity-label rule (`WSEL = Feet`) is sound only where the same-named attribute holds the
/// unit: XS and 2D result datasets and the family-level storage-area datasets. Structure result
/// datasets carry same-named DESCRIPTION attributes (`Flow = "Flow Across the given Segment..."`),
/// which the rule would misreport as units, so every other location is attribute-chain only.
fn labels_allowed(dataset: &str) -> bool {
    dataset.contains("/Cross Sections/")
        || dataset.contains("/2D Flow Areas/")
        || dataset
            .split_once("/Storage Areas/")
            .is_some_and(|(_, rest)| !rest.is_empty() && !rest.contains('/'))
}
impl ReadOnlyHdfSession {
    /// Unit evidence for a dataset in priority order: `Units`, `Variable Units`, the
    /// `Variable` [name, unit] pair, then (only for XS, 2D result and family-level storage-area
    /// datasets, where the attribute holds the unit) a single known quantity-label attribute.
    /// Below a structure group the label rule is never applied, so descriptive same-named
    /// attributes cannot be misreported as units. The source attribute is reported; nothing is
    /// inferred or converted. Unavailable only when none exists.
    pub fn dataset_units(&self, dataset: &str) -> Result<DatasetUnits> {
        self.unit_evidence_bounded(dataset, super::DEFAULT_MAX_BYTES, labels_allowed(dataset))
    }
    #[deprecated(note = "renamed dataset_units (same DatasetUnits result, structure-guarded)")]
    pub fn dataset_unit_evidence(&self, dataset: &str) -> Result<DatasetUnits> {
        self.dataset_units(dataset)
    }
    pub(crate) fn unit_evidence_bounded(
        &self,
        dataset: &str,
        max_bytes: usize,
        allow_labels: bool,
    ) -> Result<DatasetUnits> {
        self.memo(
            format!(
                "unit_evidence_bounded:{:?}",
                (dataset, max_bytes, allow_labels,)
            ),
            || {
                let ds = self.file.dataset(dataset).map_err(map_error)?;
                unit_evidence_from_attrs(ds.raw_attributes(), max_bytes, allow_labels)
            },
        )
    }
    /// Exact `Units` attribute of a dataset only (Unavailable when the dataset has none).
    pub fn dataset_units_attribute(&self, dataset: &str) -> Result<UnitsEvidence> {
        let ds = self.file.dataset(dataset).map_err(map_error)?;
        evidence(
            ds.raw_attributes(),
            UNITS_ATTRIBUTE,
            super::DEFAULT_MAX_BYTES,
        )
    }
    /// Exact `Units System` attribute of a group (Absent when the group has none).
    pub fn unit_system_evidence(&self, group: &str) -> Result<UnitSystemEvidence> {
        self.unit_system_bounded(group, super::DEFAULT_MAX_BYTES)
    }
    fn unit_system_bounded(&self, group: &str, max_bytes: usize) -> Result<UnitSystemEvidence> {
        let attrs = self
            .file
            .group(group)
            .map_err(map_error)?
            .raw_attributes()
            .map_err(map_error)?;
        let value = evidence(&attrs, SYSTEM_ATTRIBUTE, max_bytes)?;
        let kind = value.value().map(|text| match text.as_str() {
            "US Customary" => UnitSystemKind::UsCustomary,
            "SI Metric" => UnitSystemKind::SiMetric,
            _ => UnitSystemKind::Unrecognized,
        });
        Ok(UnitSystemEvidence {
            source: group.into(),
            value,
            kind,
        })
    }
    /// Per-variable `Units` for every known 2D and 1D result dataset that exists, plus the
    /// reported `Units System` evidence. Missing datasets are omitted; malformed storage
    /// propagates. `max_bytes` bounds each attribute payload consulted (an oversized one is
    /// reported Unsupported(ResourceLimit) without its bytes). No value is converted or guessed.
    pub fn result_units(&self, max_bytes: usize) -> Result<ResultUnits> {
        let mut variables = Vec::new();
        let areas = format!("{UNSTEADY}/2D Flow Areas");
        if let Some(m) = absent_ok(self.group_members(&areas))? {
            for area in &m.groups {
                let group = format!("{areas}/{area}");
                let members = self.group_members(&group)?;
                for v in MESH2D_UNIT_VARIABLES {
                    if members.datasets.iter().any(|d| d == v) {
                        let dataset = format!("{group}/{v}");
                        let d = self.unit_evidence_bounded(&dataset, max_bytes, true)?;
                        variables.push(VariableUnits {
                            family: UnitsFamily::Mesh2D,
                            area: Some(area.clone()),
                            variable: v.into(),
                            units: d.units,
                            source: d.source,
                            quantity_name: d.quantity_name,
                            dataset,
                        });
                    }
                }
            }
        }
        for (family, base) in [
            (UnitsFamily::CrossSectionUnsteady, UNSTEADY),
            (UnitsFamily::CrossSectionSteady, STEADY),
        ] {
            let group = format!("{base}/Cross Sections");
            let Some(members) = absent_ok(self.group_members(&group))? else {
                continue;
            };
            for v in XS_UNIT_VARIABLES {
                if members.datasets.iter().any(|d| d == v) {
                    let dataset = format!("{group}/{v}");
                    let d = self.unit_evidence_bounded(&dataset, max_bytes, true)?;
                    variables.push(VariableUnits {
                        family,
                        area: None,
                        variable: v.into(),
                        units: d.units,
                        source: d.source,
                        quantity_name: d.quantity_name,
                        dataset,
                    });
                }
            }
        }
        let mut unit_system = Vec::new();
        for source in UNIT_SYSTEM_SOURCES {
            if let Some(e) = absent_ok(self.unit_system_bounded(source, max_bytes))? {
                unit_system.push(e);
            }
        }
        Ok(ResultUnits {
            variables,
            unit_system,
        })
    }
}
