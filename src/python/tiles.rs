use crate::freestiler_core::{
    geo_types::{Coord, LineString, MultiPolygon, Polygon},
    tiler::{Feature, Geometry, LayerData, PropertyValue},
};
use numpy::{PyReadonlyArray1, PyReadonlyArray2};
use pyo3::exceptions::{PyOSError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyInt, PyList, PyTuple};
use std::collections::HashSet;

fn required<'py>(d: &Bound<'py, PyDict>, key: &str) -> PyResult<Bound<'py, PyAny>> {
    d.get_item(key)?
        .ok_or_else(|| PyValueError::new_err(format!("missing layer key: {key}")))
}

/// Every level is a CSR-style offset array, including a terminal sentinel.
/// Reject empty components before indexing; an entirely empty layer uses [0].
fn spans(d: &Bound<'_, PyDict>, key: &str, children: usize) -> PyResult<Vec<usize>> {
    let obj = required(d, key)?;
    let values: PyReadonlyArray1<i64> = obj.extract()?;
    let values = values.as_array();
    let offsets = values
        .iter()
        .map(|&v| {
            usize::try_from(v).map_err(|_| PyValueError::new_err(format!("{key}: negative offset")))
        })
        .collect::<PyResult<Vec<_>>>()?;
    if offsets.first() != Some(&0)
        || offsets.last() != Some(&children)
        || offsets.windows(2).any(|w| w[0] >= w[1])
    {
        return Err(PyValueError::new_err(format!(
            "{key}: offsets must span all children strictly monotonically from zero"
        )));
    }
    Ok(offsets)
}
fn property(value: &Bound<'_, PyAny>) -> PyResult<PropertyValue> {
    if value.is_none() {
        return Ok(PropertyValue::Null);
    }
    if value.is_instance_of::<PyBool>() {
        return Ok(PropertyValue::Bool(value.extract()?));
    }
    if value.is_instance_of::<PyInt>() {
        return Ok(PropertyValue::Int(value.extract::<i64>()?));
    }
    if let Ok(v) = value.extract::<String>() {
        return Ok(PropertyValue::String(v));
    }
    if let Ok(v) = value.extract::<f64>() {
        return Ok(finite(v));
    }
    Err(PyValueError::new_err(
        "properties must be strings, signed 64-bit integers, floats, bools or None",
    ))
}

/// Non-finite floats are null, as for list properties.
fn finite(v: f64) -> PropertyValue {
    if v.is_finite() {
        PropertyValue::Double(v)
    } else {
        PropertyValue::Null
    }
}

/// One-dimensional NumPy array of a supported dtype, read in Rust without a
/// Python object per element. Unsigned 64-bit values must fit signed 64-bit.
fn numpy_column(value: &Bound<'_, PyAny>, key: &str) -> PyResult<Option<Vec<PropertyValue>>> {
    macro_rules! try_dtype {
        ($t:ty, $convert:expr) => {
            if let Ok(array) = value.extract::<PyReadonlyArray1<$t>>() {
                let convert = $convert;
                return array
                    .as_array()
                    .iter()
                    .map(|v| convert(*v))
                    .collect::<PyResult<Vec<_>>>()
                    .map(Some);
            }
        };
    }
    try_dtype!(f64, |v: f64| -> PyResult<_> { Ok(finite(v)) });
    try_dtype!(f32, |v: f32| -> PyResult<_> { Ok(finite(f64::from(v))) });
    try_dtype!(i64, |v: i64| -> PyResult<_> { Ok(PropertyValue::Int(v)) });
    try_dtype!(i32, |v: i32| -> PyResult<_> {
        Ok(PropertyValue::Int(i64::from(v)))
    });
    try_dtype!(u64, |v: u64| -> PyResult<_> {
        i64::try_from(v).map(PropertyValue::Int).map_err(|_| {
            PyValueError::new_err(format!("{key}: unsigned value exceeds signed 64-bit"))
        })
    });
    try_dtype!(u32, |v: u32| -> PyResult<_> {
        Ok(PropertyValue::Int(i64::from(v)))
    });
    try_dtype!(bool, |v: bool| -> PyResult<_> {
        Ok(PropertyValue::Bool(v))
    });
    Ok(None)
}

/// A property column in any accepted form:
/// - a list of scalars (one Python object per value);
/// - a 1-D NumPy array (float64/32, int64/32, uint64/32, bool), non-finite
///   floats null;
/// - a `(values, valid)` tuple of a NumPy array and a boolean NumPy mask of
///   the same length, `False` meaning null;
/// - a `{"codes": int array, "categories": list of str}` dict for strings,
///   code `-1` meaning null.
fn column(value: &Bound<'_, PyAny>, key: &str) -> PyResult<Vec<PropertyValue>> {
    if let Ok(list) = value.cast::<PyList>() {
        return list.iter().map(|v| property(&v)).collect();
    }
    if let Ok(tuple) = value.cast::<PyTuple>() {
        if tuple.len() != 2 {
            return Err(PyValueError::new_err(format!(
                "{key}: a masked column is a (values, valid) pair"
            )));
        }
        let values = numpy_column(&tuple.get_item(0)?, key)?.ok_or_else(|| {
            PyValueError::new_err(format!(
                "{key}: masked values must be a numeric or bool NumPy array"
            ))
        })?;
        let valid: PyReadonlyArray1<bool> = tuple.get_item(1)?.extract().map_err(|_| {
            PyValueError::new_err(format!(
                "{key}: the validity mask must be a bool NumPy array"
            ))
        })?;
        let valid = valid.as_array();
        if valid.len() != values.len() {
            return Err(PyValueError::new_err(format!(
                "{key}: validity mask length differs from values"
            )));
        }
        return Ok(values
            .into_iter()
            .zip(valid.iter())
            .map(|(v, &ok)| if ok { v } else { PropertyValue::Null })
            .collect());
    }
    if let Ok(d) = value.cast::<PyDict>() {
        if d.len() != 2 || !d.contains("codes")? || !d.contains("categories")? {
            return Err(PyValueError::new_err(format!(
                "{key}: a categorical column has exactly 'codes' and 'categories'"
            )));
        }
        let categories: Vec<String> = required(d, "categories")?.extract()?;
        let codes: PyReadonlyArray1<i64> = required(d, "codes")?.extract().map_err(|_| {
            PyValueError::new_err(format!(
                "{key}: categorical codes must be an int64 NumPy array"
            ))
        })?;
        return codes
            .as_array()
            .iter()
            .map(|&c| match c {
                -1 => Ok(PropertyValue::Null),
                c => usize::try_from(c)
                    .ok()
                    .and_then(|c| categories.get(c))
                    .map(|s| PropertyValue::String(s.clone()))
                    .ok_or_else(|| {
                        PyValueError::new_err(format!("{key}: categorical code {c} out of range"))
                    }),
            })
            .collect();
    }
    numpy_column(value, key)?.ok_or_else(|| {
        PyValueError::new_err(format!(
            "{key}: properties must be lists, NumPy arrays, (values, valid) pairs or categorical dicts"
        ))
    })
}

/// Select one value per feature. `rows[i]` is the source row of feature `i`,
/// or `-1` for a feature without a source row (null).
fn gather(
    values: Vec<PropertyValue>,
    rows: Option<&[i64]>,
    key: &str,
) -> Result<Vec<PropertyValue>, TileInputError> {
    let Some(rows) = rows else {
        return Ok(values);
    };
    rows.iter()
        .map(|&r| match r {
            -1 => Ok(PropertyValue::Null),
            r => usize::try_from(r)
                .ok()
                .and_then(|r| values.get(r))
                .cloned()
                .ok_or_else(|| {
                    TileInputError::Value(format!("{key}: property row {r} outside the column"))
                }),
        })
        .collect()
}

#[derive(Debug)]
enum TileInputError {
    Value(String),
    Topology(String),
}

impl From<TileInputError> for PyErr {
    fn from(e: TileInputError) -> Self {
        match e {
            TileInputError::Value(m) => PyValueError::new_err(m),
            TileInputError::Topology(m) => crate::python::TopologyError::new_err(m),
        }
    }
}

/// Packed polygons of one layer, already extracted from Python.
struct Packed<'a> {
    name: String,
    coords: ndarray::ArrayView2<'a, f64>,
    ring_offsets: Vec<usize>,
    polygon_offsets: Vec<usize>,
    feature_offsets: Vec<usize>,
    ids: ndarray::ArrayView1<'a, i64>,
}

/// Build one tiler layer. Columns hold one value per feature. Returns `None`
/// for a layer without features.
fn build_layer(
    packed: &Packed<'_>,
    prop_names: Vec<String>,
    columns: Vec<Vec<PropertyValue>>,
    min_zoom: u8,
    max_zoom: u8,
) -> Result<Option<LayerData>, TileInputError> {
    let name = &packed.name;
    let (coords, ids) = (&packed.coords, &packed.ids);
    let (ring_offsets, polygon_offsets, feature_offsets) = (
        &packed.ring_offsets,
        &packed.polygon_offsets,
        &packed.feature_offsets,
    );
    if feature_offsets.len() != ids.len() + 1 {
        return Err(TileInputError::Value(format!(
            "{name}: feature offsets differ from ID count"
        )));
    }
    if coords.rows().into_iter().any(|p| {
        !p[0].is_finite() || !p[1].is_finite() || p[0].abs() > 180. || p[1].abs() > 85.0511287798066
    }) {
        return Err(TileInputError::Value(format!(
            "{name}: coordinates must be finite WGS84 lon/lat within Web Mercator latitude limits"
        )));
    }
    let mut prop_types = Vec::new();
    for values in &columns {
        if values.len() != ids.len() {
            return Err(TileInputError::Value(format!(
                "{name}: property length differs from polygon count"
            )));
        }
        // freestiler expects its input type names, not TileJSON's output
        // labels. Mixed scalar columns have no TileJSON union type.
        let mut kind = None;
        for value in values {
            let next = match value {
                PropertyValue::String(_) => "string",
                PropertyValue::Bool(_) => "boolean",
                PropertyValue::Null => continue,
                _ => "numeric",
            };
            kind = Some(match kind {
                Some(previous) if previous != next => "mixed",
                _ => next,
            });
        }
        prop_types.push(kind.unwrap_or("string").to_string());
    }
    let mut features = Vec::with_capacity(ids.len());
    let mut seen = HashSet::new();
    for i in 0..ids.len() {
        let id = u64::try_from(ids[i])
            .map_err(|_| TileInputError::Value("cell IDs must be nonnegative".into()))?;
        if !seen.insert(id) {
            return Err(TileInputError::Value(format!(
                "{name}: duplicate feature ID {id}"
            )));
        }
        let mut polygons = Vec::new();
        for p in feature_offsets[i]..feature_offsets[i + 1] {
            let mut rings = Vec::new();
            for r in polygon_offsets[p]..polygon_offsets[p + 1] {
                let start = ring_offsets[r];
                let end = ring_offsets[r + 1];
                if end - start < 4 {
                    return Err(TileInputError::Value(format!(
                        "{name}: invalid ring span at feature {id}"
                    )));
                }
                if coords.row(start) != coords.row(end - 1) {
                    return Err(TileInputError::Value(format!(
                        "{name}: unclosed ring at feature {id}"
                    )));
                }
                let ring = LineString::new(
                    (start..end)
                        .map(|r| Coord {
                            x: coords[[r, 0]],
                            y: coords[[r, 1]],
                        })
                        .collect(),
                );
                crate::mesh::validate_ring_geometry(ring.0.iter().map(|c| [c.x, c.y])).map_err(
                    |error| TileInputError::Topology(format!("{name}: feature {id}: {error}")),
                )?;
                rings.push(ring);
            }
            let exterior = rings.remove(0);
            polygons.push(Polygon::new(exterior, rings));
        }
        let geometry = if polygons.len() == 1 {
            Geometry::Polygon(polygons.remove(0))
        } else {
            Geometry::MultiPolygon(MultiPolygon::new(polygons))
        };
        features.push(Feature {
            id: Some(id),
            geometry,
            properties: columns.iter().map(|c| c[i].clone()).collect(),
        });
    }
    if features.is_empty() {
        return Ok(None);
    }
    Ok(Some(LayerData {
        name: name.clone(),
        features,
        prop_names,
        prop_types,
        min_zoom,
        max_zoom,
    }))
}

/// Write MVT PMTiles directly from packed WGS84 polygon buffers.
/// Caller stages the destination for atomic publication.
/// Legacy `offsets` spans one exterior ring per feature. Alternatively supply
/// all three arrays: `ring_offsets` spans coordinates, `polygon_offsets` spans
/// rings (exterior first, then holes), and `feature_offsets` spans polygons.
/// IDs and property columns are aligned with features, never rings or parts.
/// Property columns are lists, NumPy arrays, `(values, valid)` pairs or
/// categorical dicts (see [`column`]). With the optional `property_rows`
/// (int64, one per feature, `-1` for none) every column is indexed by source
/// row instead, so callers pass whole result arrays without a per-cell take.
#[pyfunction]
#[pyo3(signature=(layers,output,min_zoom,max_zoom,threads=None))]
pub fn write_pmtiles(
    py: Python<'_>,
    layers: &Bound<'_, PyList>,
    output: &str,
    min_zoom: u8,
    max_zoom: u8,
    threads: Option<usize>,
) -> PyResult<()> {
    crate::python::panic_guard::guard(|| {
        if min_zoom > max_zoom || max_zoom > 22 {
            return Err(PyValueError::new_err(
                "zoom range must satisfy 0 <= min_zoom <= max_zoom <= 22",
            ));
        }
        let mut names = HashSet::new();
        let mut parsed = Vec::new();
        for obj in layers.iter() {
            let d = obj.cast::<PyDict>()?;
            let name: String = required(d, "name")?.extract()?;
            if name.is_empty() || !names.insert(name.clone()) {
                return Err(PyValueError::new_err(
                    "layer names must be nonempty and unique",
                ));
            }
            let coords_obj = required(d, "coordinates")?;
            let ids_obj = required(d, "cell_ids")?;
            let coords: PyReadonlyArray2<f64> = coords_obj.extract()?;
            let ids: PyReadonlyArray1<i64> = ids_obj.extract()?;
            let coords = coords.as_array();
            let ids = ids.as_array();
            if coords.ncols() != 2 {
                return Err(PyValueError::new_err(format!(
                    "{name}: invalid packed polygon dimensions"
                )));
            }
            let nested = d.contains("ring_offsets")?
                || d.contains("polygon_offsets")?
                || d.contains("feature_offsets")?;
            let (ring_offsets, polygon_offsets, feature_offsets) = if nested {
                if d.contains("offsets")? {
                    return Err(PyValueError::new_err(
                        "use either offsets or the three nested offset arrays, not both",
                    ));
                }
                let rings = spans(d, "ring_offsets", coords.nrows())?;
                let polygons = spans(d, "polygon_offsets", rings.len() - 1)?;
                let features = spans(d, "feature_offsets", polygons.len() - 1)?;
                (rings, polygons, features)
            } else {
                let rings = spans(d, "offsets", coords.nrows())?;
                let identity: Vec<_> = (0..rings.len()).collect();
                (rings, identity.clone(), identity)
            };
            let rows = match d.get_item("property_rows")? {
                Some(obj) => {
                    let rows: PyReadonlyArray1<i64> = obj.extract()?;
                    let rows = rows.as_array().to_vec();
                    if rows.len() != ids.len() {
                        return Err(PyValueError::new_err(format!(
                            "{name}: property_rows length differs from polygon count"
                        )));
                    }
                    Some(rows)
                }
                None => None,
            };
            let properties = required(d, "properties")?;
            let properties = properties.cast::<PyDict>()?;
            let mut prop_names = Vec::new();
            let mut columns = Vec::new();
            for (key, values) in properties.iter() {
                let key = key.extract::<String>()?;
                let values = column(&values, &key)?;
                columns.push(gather(values, rows.as_deref(), &key)?);
                prop_names.push(key);
            }
            let packed = Packed {
                name,
                coords,
                ring_offsets,
                polygon_offsets,
                feature_offsets,
                ids,
            };
            if let Some(layer) = build_layer(&packed, prop_names, columns, min_zoom, max_zoom)? {
                parsed.push(layer);
            }
        }
        if parsed.is_empty() {
            return Err(PyValueError::new_err(
                "at least one nonempty layer is required",
            ));
        }
        let pool = crate::python::pool(threads)?;
        py.detach(|| {
            pool.install(|| crate::python::tile_engine::write(&parsed, output, min_zoom, max_zoom))
        })
        .map_err(PyOSError::new_err)
    })
}

#[cfg(test)]
#[path = "tiles_tests.rs"]
mod tests;
