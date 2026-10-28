//! Contract v1: XY OGC WKB + nullable typed Arrow fields; no implicit CRS defaults.
use crate::exports::{fail, Result};
use arrow_array::{Array, BinaryArray, Int64Array, RecordBatch, StringArray};
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
        // Names differing only in case (`River` and `river`) are distinct in
        // Arrow and Parquet; formats with case-insensitive column names refuse
        // them with `require_case_insensitive_names`.
        let mut names = HashSet::new();
        for field in schema.fields() {
            if field.name().is_empty()
                || field.name().contains('\0')
                || !names.insert(field.name().as_str())
            {
                return Err(fail("empty/NUL/duplicate field name"));
            }
            if ["fid", "_ras_fid"].contains(&field.name().to_lowercase().as_str()) {
                return Err(fail("reserved GeoPackage primary key name"));
            }
            if !supported_type(field.data_type()) {
                return Err(fail(format!(
                    "unsupported field type: {}",
                    field.data_type()
                )));
            }
            if field.data_type() == &DataType::Null && !field.is_nullable() {
                return Err(fail("Null-typed fields must be nullable"));
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
            if !all_finite(array.as_ref()) {
                return Err(fail("nonfinite attribute: supply null explicitly"));
            }
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

/// Refuse field names that differ only in case, for formats whose column
/// names are case-insensitive (GeoPackage/SQLite, FlatGeobuf read by GDAL,
/// GeoJSON and MVT consumers).
pub fn require_case_insensitive_names(schema: &arrow_schema::Schema) -> Result<()> {
    let mut names = HashSet::new();
    for field in schema.fields() {
        if !names.insert(field.name().to_lowercase()) {
            return Err(fail(format!(
                "field names differing only in case ({}) need a format with \
                 case-sensitive names; use GeoParquet or Arrow IPC",
                field.name()
            )));
        }
    }
    Ok(())
}

/// Attribute types contract v1 accepts. The scalar types are written by every
/// format; float32, `Null`, microsecond/nanosecond timestamps, `List` and
/// `Struct` are exact in Arrow IPC and GeoParquet only, and the other writers
/// convert or refuse them explicitly (`exports/README.md`). Dictionary,
/// LargeList, FixedSizeList, Map, LargeUtf8/LargeBinary, narrower integers and
/// timezone-qualified timestamps still reject.
pub fn supported_type(data_type: &DataType) -> bool {
    match data_type {
        DataType::Int64
        | DataType::UInt64
        | DataType::Float64
        | DataType::Float32
        | DataType::Boolean
        | DataType::Utf8
        | DataType::Binary
        | DataType::Null
        | DataType::Timestamp(
            TimeUnit::Millisecond | TimeUnit::Microsecond | TimeUnit::Nanosecond,
            None,
        ) => true,
        DataType::List(child) => child.is_nullable() && supported_type(child.data_type()),
        DataType::Struct(fields) => {
            let mut names = HashSet::new();
            !fields.is_empty()
                && fields.iter().all(|f| {
                    !f.name().is_empty()
                        && !f.name().contains('\0')
                        && names.insert(f.name().as_str())
                        && f.is_nullable()
                        && supported_type(f.data_type())
                })
        }
        _ => false,
    }
}

/// The scalar types every writer stores without conversion (contract v1 as
/// first published).
pub fn is_v1_scalar(data_type: &DataType) -> bool {
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

/// True unless a non-null float, at any nesting depth, is NaN or infinite.
/// List children are checked over the range the offsets reference.
fn all_finite(array: &dyn Array) -> bool {
    use arrow_array::cast::AsArray;
    match array.data_type() {
        DataType::Float64 => array
            .as_primitive::<arrow_array::types::Float64Type>()
            .iter()
            .flatten()
            .all(f64::is_finite),
        DataType::Float32 => array
            .as_primitive::<arrow_array::types::Float32Type>()
            .iter()
            .flatten()
            .all(f32::is_finite),
        DataType::List(_) => {
            let list = array.as_list::<i32>();
            let offsets = list.value_offsets();
            let (start, end) = (offsets[0] as usize, offsets[offsets.len() - 1] as usize);
            all_finite(list.values().slice(start, end - start).as_ref())
        }
        DataType::Struct(_) => array
            .as_struct()
            .columns()
            .iter()
            .all(|c| all_finite(c.as_ref())),
        _ => true,
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
