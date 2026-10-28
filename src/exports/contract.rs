//! Contract v1: XY OGC WKB + nullable typed Arrow fields; no implicit CRS defaults.
use crate::exports::{fail, Result};
use arrow_array::{
    Array, BinaryArray, FixedSizeListArray, Float32Array, Float64Array, Int64Array, LargeListArray,
    ListArray, RecordBatch, StringArray, StructArray,
};
use arrow_schema::{DataType, SchemaRef, TimeUnit};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const CONTRACT_KEY: &str = "ras:export:v1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Crs {
    pub wkt: String,
    pub projjson: serde_json::Value,
    pub units: String,
    /// All geometry buffers use x/y, regardless of authority axis ordering.
    pub axis_order: String,
    /// Filled only after semantic CRS comparison at the pyproj/resolver boundary.
    pub resolved: bool,
    /// True only for WGS84 longitude/latitude coordinates, after transformation.
    pub geographic_wgs84: bool,
}

/// Stable row identity: a nonnullable, nonempty Utf8 namespace and an
/// optional nonnullable, nonnegative Int64 local ID. Their composite must be
/// unique within each batch. Contract v1's default is `(mesh_name, cell_id)`;
/// other producers name their own (for example `(mesh_name, face_id)`, or a
/// namespace alone for one-row-per-name layers such as 2D flow areas).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub namespace: String,
    pub local: Option<String>,
}

impl Default for Identity {
    fn default() -> Self {
        Self {
            namespace: "mesh_name".into(),
            local: Some("cell_id".into()),
        }
    }
}

/// Which attribute types a writer accepts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Attributes {
    /// Contract v1 scalars, which every writer can store faithfully.
    #[default]
    Portable,
    /// Every type Parquet stores and returns unchanged through the Arrow
    /// schema it embeds: contract v1 scalars plus Float32, Timestamp in
    /// milliseconds, microseconds or nanoseconds (time zone kept as given),
    /// an all-null `Null` column, and List, LargeList, FixedSizeList and
    /// nonempty Struct nested to any depth over Int64, UInt64, Float64,
    /// Float32, Boolean, Utf8, Binary or those timestamps. Field names may
    /// differ only in case (Parquet names are case-sensitive). Only the
    /// GeoParquet archive profile accepts these; it passes them through with
    /// no widening or conversion.
    Archive,
}

fn portable_type(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Int64
            | DataType::UInt64
            | DataType::Float64
            | DataType::Boolean
            | DataType::Utf8
            | DataType::Binary
            | DataType::Timestamp(TimeUnit::Millisecond, None)
    )
}

fn archive_type(data_type: &DataType, top_level: bool) -> bool {
    match data_type {
        DataType::Int64
        | DataType::UInt64
        | DataType::Float64
        | DataType::Float32
        | DataType::Boolean
        | DataType::Utf8
        | DataType::Binary
        | DataType::Timestamp(
            TimeUnit::Millisecond | TimeUnit::Microsecond | TimeUnit::Nanosecond,
            _,
        ) => true,
        DataType::Null => top_level,
        DataType::List(item) | DataType::LargeList(item) | DataType::FixedSizeList(item, _) => {
            archive_type(item.data_type(), false)
        }
        DataType::Struct(fields) => {
            !fields.is_empty() && fields.iter().all(|f| archive_type(f.data_type(), false))
        }
        _ => false,
    }
}

impl Attributes {
    fn accepts(self, data_type: &DataType) -> bool {
        match self {
            Self::Portable => portable_type(data_type),
            Self::Archive => archive_type(data_type, true),
        }
    }
}

/// Refuse NaN or infinity in a float column, including float values nested
/// in lists and structs. Only the values each valid row references are checked.
fn check_finite(array: &dyn Array) -> Result<()> {
    fn nonfinite() -> crate::exports::Error {
        fail("nonfinite attribute: supply null explicitly")
    }
    let any = array.as_any();
    if let Some(values) = any.downcast_ref::<Float64Array>() {
        if values.iter().flatten().any(|v| !v.is_finite()) {
            return Err(nonfinite());
        }
    } else if let Some(values) = any.downcast_ref::<Float32Array>() {
        if values.iter().flatten().any(|v| !v.is_finite()) {
            return Err(nonfinite());
        }
    } else if let Some(list) = any.downcast_ref::<ListArray>() {
        for row in list.iter().flatten() {
            check_finite(row.as_ref())?;
        }
    } else if let Some(list) = any.downcast_ref::<LargeListArray>() {
        for row in list.iter().flatten() {
            check_finite(row.as_ref())?;
        }
    } else if let Some(list) = any.downcast_ref::<FixedSizeListArray>() {
        for row in list.iter().flatten() {
            check_finite(row.as_ref())?;
        }
    } else if let Some(fields) = any.downcast_ref::<StructArray>() {
        // Values under a null struct slot are not data: check valid rows only.
        for child in fields.columns() {
            if fields.null_count() == 0 {
                check_finite(child.as_ref())?;
            } else {
                for row in (0..fields.len()).filter(|&row| fields.is_valid(row)) {
                    check_finite(child.slice(row, 1).as_ref())?;
                }
            }
        }
    }
    Ok(())
}

pub struct Contract {
    pub schema: SchemaRef,
    pub crs: Crs,
    pub geometry: usize,
    namespace: usize,
    local: Option<usize>,
}
impl Contract {
    pub fn new(schema: SchemaRef) -> Result<Self> {
        Self::with_identity(schema, &Identity::default())
    }

    pub fn with_identity(schema: SchemaRef, identity: &Identity) -> Result<Self> {
        Self::with_profile(schema, identity, Attributes::Portable)
    }

    pub fn with_profile(
        schema: SchemaRef,
        identity: &Identity,
        attributes: Attributes,
    ) -> Result<Self> {
        let crs: Crs = serde_json::from_str(
            schema
                .metadata()
                .get(CONTRACT_KEY)
                .ok_or_else(|| fail("missing resolved source CRS contract"))?,
        )?;
        if !crs.resolved
            || crs.wkt.trim().is_empty()
            || crs.units.trim().is_empty()
            || !crs.projjson.is_object()
            || crs.axis_order != "xy"
        {
            return Err(fail(
                "CRS evidence unresolved, missing or unsupported axis order",
            ));
        }
        let mut names = HashSet::new();
        for field in schema.fields() {
            if field.name().is_empty()
                || field.name().contains('\0')
                || !names.insert(match attributes {
                    // SQLite (GeoPackage) and Esri formats fold case;
                    // Parquet names are case-sensitive, so `River` and
                    // `river` are two columns there.
                    Attributes::Portable => field.name().to_lowercase(),
                    Attributes::Archive => field.name().clone(),
                })
            {
                return Err(fail(format!(
                    "empty/NUL/duplicate{} field name {:?}",
                    if attributes == Attributes::Portable {
                        " case-insensitive"
                    } else {
                        ""
                    },
                    field.name()
                )));
            }
            if ["fid", "_ras_fid"].contains(&field.name().to_lowercase().as_str()) {
                return Err(fail("reserved GeoPackage primary key name"));
            }
            if !attributes.accepts(field.data_type()) {
                return Err(fail(format!(
                    "unsupported field type for field {}: {}",
                    field.name(),
                    field.data_type()
                )));
            }
        }
        let mut required = vec![
            (identity.namespace.as_str(), DataType::Utf8),
            ("geometry", DataType::Binary),
        ];
        if let Some(local) = &identity.local {
            required.push((local.as_str(), DataType::Int64));
        }
        for (name, dtype) in required {
            let f = schema.field_with_name(name)?;
            if f.data_type() != &dtype {
                return Err(fail(format!("{name} has incorrect type")));
            }
            if name != "geometry" && f.is_nullable() {
                return Err(fail("identity fields must be nonnullable"));
            }
        }
        if identity.local.as_deref() == Some(identity.namespace.as_str())
            || identity.namespace == "geometry"
        {
            return Err(fail(
                "identity fields must be distinct non-geometry columns",
            ));
        }
        let namespace = schema.index_of(&identity.namespace)?;
        let local = identity
            .local
            .as_ref()
            .map(|l| schema.index_of(l))
            .transpose()?;
        let geometry = schema.index_of("geometry")?;
        let field = schema.field(geometry);
        if field
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str)
            != Some("geoarrow.wkb")
        {
            return Err(fail("geometry requires geoarrow.wkb extension"));
        }
        let geo: serde_json::Value = serde_json::from_str(
            field
                .metadata()
                .get("ARROW:extension:metadata")
                .ok_or_else(|| fail("missing GeoArrow CRS"))?,
        )?;
        if geo.get("crs_type").and_then(|v| v.as_str()) != Some("projjson")
            || geo.get("crs") != Some(&crs.projjson)
        {
            return Err(fail("conflicting GeoArrow and source CRS"));
        }
        Ok(Self {
            schema,
            crs,
            geometry,
            namespace,
            local,
        })
    }

    pub fn validate(&self, batch: &RecordBatch, max: usize) -> Result<()> {
        if batch.schema() != self.schema {
            return Err(fail("batch schema mismatch"));
        }
        if batch.num_rows() > max {
            return Err(fail("batch exceeds max_batch_rows"));
        }
        let meshes = batch
            .column(self.namespace)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let cells = self.local.map(|i| {
            batch
                .column(i)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
        });
        let mut identities = HashSet::new();
        for row in 0..batch.num_rows() {
            let local = cells.map(|c| (c.is_null(row), c.value(row)));
            if meshes.is_null(row)
                || local.is_some_and(|(null, value)| null || value < 0)
                || meshes.value(row).is_empty()
                || !identities.insert((meshes.value(row), local.map(|l| l.1)))
            {
                return Err(fail(
                    "null/empty/negative or duplicate within-batch composite identity",
                ));
            }
        }
        for (field, array) in self.schema.fields().iter().zip(batch.columns()) {
            if !field.is_nullable() && array.null_count() > 0 {
                return Err(fail("null in nonnullable field"));
            }
            check_finite(array.as_ref())?;
        }
        let geometry = batch
            .column(self.geometry)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        for wkb in geometry.iter().flatten() {
            validate_wkb(wkb, self.crs.geographic_wgs84)?;
        }
        Ok(())
    }
}

// Bound-check before third-party decoders. Reject EWKB/SRID, Z/M, collections,
// nonfinite coordinates, unclosed/undersized rings and trailing/truncated input.
// Full topology validity remains the source geometry producer's responsibility.
pub fn validate_wkb(bytes: &[u8], geographic: bool) -> Result<()> {
    validate_wkb_profile(bytes, geographic, false, false).map(|_| ())
}

/// Validate like [`validate_wkb`] and return the XY bounds
/// `[xmin, ymin, xmax, ymax]` over every coordinate (holes and parts
/// included), or `None` for an empty geometry.
pub fn wkb_bounds(bytes: &[u8]) -> Result<Option<[f64; 4]>> {
    validate_wkb_profile(bytes, false, false, false)
}

pub fn validate_geojson_wkb(bytes: &[u8]) -> Result<()> {
    validate_wkb_profile(bytes, true, true, false).map(|_| ())
}

pub fn validate_flatgeobuf_wkb(bytes: &[u8]) -> Result<()> {
    validate_wkb_profile(bytes, false, false, true).map(|_| ())
}

fn validate_wkb_profile(
    bytes: &[u8],
    geographic: bool,
    winding: bool,
    reject_empty: bool,
) -> Result<Option<[f64; 4]>> {
    struct Reader<'a> {
        bytes: &'a [u8],
        pos: usize,
        geographic: bool,
        winding: bool,
        reject_empty: bool,
        bounds: Option<[f64; 4]>,
    }
    impl Reader<'_> {
        fn read<const N: usize>(&mut self) -> Result<[u8; N]> {
            let end = self
                .pos
                .checked_add(N)
                .ok_or_else(|| fail("WKB overflow"))?;
            let slice = self
                .bytes
                .get(self.pos..end)
                .ok_or_else(|| fail("truncated WKB"))?;
            self.pos = end;
            Ok(slice.try_into()?)
        }
        fn uint(&mut self, le: bool) -> Result<usize> {
            let b = self.read()?;
            Ok(if le {
                u32::from_le_bytes(b)
            } else {
                u32::from_be_bytes(b)
            } as usize)
        }
        fn xy(&mut self, le: bool) -> Result<[f64; 2]> {
            let mut xy = [0.; 2];
            for v in &mut xy {
                let b = self.read()?;
                *v = if le {
                    f64::from_le_bytes(b)
                } else {
                    f64::from_be_bytes(b)
                };
            }
            if xy.iter().any(|v| !v.is_finite())
                || (self.geographic && (xy[0].abs() > 180. || xy[1].abs() > 90.))
            {
                return Err(fail(
                    "nonfinite/out-of-domain WKB coordinates; empty points unsupported",
                ));
            }
            let b = self.bounds.get_or_insert([xy[0], xy[1], xy[0], xy[1]]);
            b[0] = b[0].min(xy[0]);
            b[1] = b[1].min(xy[1]);
            b[2] = b[2].max(xy[0]);
            b[3] = b[3].max(xy[1]);
            Ok(xy)
        }
        fn count(&mut self, le: bool) -> Result<usize> {
            let n = self.uint(le)?;
            if n > self.bytes.len().saturating_sub(self.pos) / 4 {
                return Err(fail("WKB count exceeds remaining bytes"));
            }
            Ok(n)
        }
        fn geometry(&mut self, expected: Option<usize>) -> Result<()> {
            let endian = self.read::<1>()?[0];
            if endian > 1 {
                return Err(fail("invalid WKB byte order"));
            }
            let le = endian == 1;
            let kind = self.uint(le)?;
            if expected.is_some_and(|k| k != kind) {
                return Err(fail("multipart WKB member type mismatch"));
            }
            match kind {
                1 => {
                    self.xy(le)?;
                }
                2 => {
                    let n = self.count(le)?;
                    if self.reject_empty && n == 0 {
                        return Err(fail("indexed FlatGeobuf rejects empty geometry"));
                    }
                    if n == 1 {
                        return Err(fail("undersized line"));
                    }
                    for _ in 0..n {
                        self.xy(le)?;
                    }
                }
                3 => {
                    let rings = self.count(le)?;
                    if self.reject_empty && rings == 0 {
                        return Err(fail("indexed FlatGeobuf rejects empty geometry"));
                    }
                    for ring in 0..rings {
                        let n = self.count(le)?;
                        if n < 4 {
                            return Err(fail("undersized polygon ring"));
                        }
                        let first = self.xy(le)?;
                        let mut previous = first;
                        let mut twice_area = 0.;
                        for _ in 1..n {
                            let next = self.xy(le)?;
                            twice_area += previous[0] * next[1] - next[0] * previous[1];
                            previous = next;
                        }
                        if previous != first {
                            return Err(fail("unclosed ring"));
                        }
                        if self.winding
                            && ((ring == 0 && twice_area <= 0.) || (ring > 0 && twice_area >= 0.))
                        {
                            return Err(fail(
                                "GeoJSON requires CCW exterior/CW holes; orient explicitly",
                            ));
                        }
                    }
                }
                4..=6 => {
                    let n = self.count(le)?;
                    if self.reject_empty && n == 0 {
                        return Err(fail("indexed FlatGeobuf rejects empty geometry"));
                    }
                    for _ in 0..n {
                        self.geometry(Some(kind - 3))?;
                    }
                }
                _ => return Err(fail("unsupported WKB type/dimension/SRID")),
            }
            Ok(())
        }
    }
    let mut r = Reader {
        bytes,
        pos: 0,
        geographic,
        winding,
        reject_empty,
        bounds: None,
    };
    r.geometry(None)?;
    if r.pos != bytes.len() {
        return Err(fail("trailing WKB bytes"));
    }
    Ok(r.bounds)
}
