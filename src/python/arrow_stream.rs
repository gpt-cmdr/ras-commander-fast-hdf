//! Canonical HEC-RAS layers for Python as Arrow C streams (feature `python-arrow`).
//!
//! `arrow_layer(...)` returns one layer of the engine's canonical catalog
//! ([`crate::arrow_layers`]) as an object implementing the Arrow PyCapsule stream interface
//! (`__arrow_c_stream__`). `pyarrow.table(obj)`, `pyarrow.RecordBatchReader.from_stream(obj)`
//! and other PyCapsule consumers (polars, DuckDB, GeoPandas via pyarrow) take the batches
//! without IPC files or per-feature Python objects. No file formats are written here.
//!
//! The only `unsafe` is inside arrow-rs's C data interface export; this module creates the
//! capsule with PyO3's safe constructor, whose destructor releases an unconsumed stream.
use crate::arrow_layers::{self as layers, ArrowLayer, LayerOptions};
use crate::hdf::ReadOnlyHdfSession;
use crate::python::panic_guard::guard;
use crate::python::reader::err;
use arrow_array::ffi_stream::FFI_ArrowArrayStream;
use arrow_array::RecordBatchIterator;
use arrow_schema::ffi::FFI_ArrowSchema;
use arrow_schema::ArrowError;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyCapsule;
use std::ffi::CStr;
use std::path::PathBuf;

const STREAM: &CStr = c"arrow_array_stream";
const SCHEMA: &CStr = c"arrow_schema";

/// One canonical layer, consumable once through `__arrow_c_stream__`.
#[pyclass(module = "ras_commander_fast_hdf", name = "ArrowLayer")]
pub struct PyArrowLayer {
    layer: Option<ArrowLayer>,
    schema: arrow_schema::SchemaRef,
    /// Layer name: the canonical catalog name, or the variable slug for summary results.
    #[pyo3(get)]
    name: String,
    #[pyo3(get)]
    num_rows: usize,
    /// Row identity: `(namespace_field, local_field_or_None)`.
    #[pyo3(get)]
    identity: (String, Option<String>),
}

#[pymethods]
impl PyArrowLayer {
    /// The Arrow PyCapsule stream; the batches move to the consumer (one use).
    #[pyo3(signature = (requested_schema=None))]
    fn __arrow_c_stream__<'py>(
        &mut self,
        py: Python<'py>,
        requested_schema: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyCapsule>> {
        if requested_schema.is_some_and(|s| !s.is_none()) {
            return Err(PyValueError::new_err(
                "ArrowLayer does not cast to a requested schema; cast after reading",
            ));
        }
        let layer = self
            .layer
            .take()
            .ok_or_else(|| PyValueError::new_err("ArrowLayer stream was already consumed"))?;
        let reader = RecordBatchIterator::new(
            layer.batches.into_iter().map(Ok::<_, ArrowError>),
            layer.schema,
        );
        PyCapsule::new_with_value(py, FFI_ArrowArrayStream::new(Box::new(reader)), STREAM)
    }
    /// The Arrow PyCapsule schema (available after the stream is consumed).
    fn __arrow_c_schema__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyCapsule>> {
        let schema = FFI_ArrowSchema::try_from(self.schema.as_ref())
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        PyCapsule::new_with_value(py, schema, SCHEMA)
    }
    fn __repr__(&self) -> String {
        format!("ArrowLayer({:?}, rows={})", self.name, self.num_rows)
    }
}

/// Read one layer of the canonical catalog by canonical name or consumer alias (ras2cng's
/// `mesh_areas`, `centerlines`, ...; see `layer_catalog()`), or `mesh_summary` with `variable`
/// (a stored 2D summary variable such as `Maximum Water Surface`). `geometry=True` appends the
/// cell polygon or face line to a summary layer. Refusals raise the engine's typed errors
/// (`UnsupportedError`, ...); nothing is returned approximately. A family the file does not
/// store raises the engine's not-found error.
#[pyfunction]
#[pyo3(signature = (path, layer, variable=None, batch_rows=65536, threads=None, geometry=false))]
fn arrow_layer(
    py: Python<'_>,
    path: PathBuf,
    layer: &str,
    variable: Option<String>,
    batch_rows: usize,
    threads: Option<usize>,
    geometry: bool,
) -> PyResult<PyArrowLayer> {
    guard(|| {
        let count = crate::python::reader::threads(threads)?;
        let session = ReadOnlyHdfSession::with_options(&path, count, count > 1).map_err(err)?;
        let options = LayerOptions::default()
            .with_batch_rows(batch_rows)
            .with_summary_geometry(geometry);
        let read = py.detach(|| session.read_layer(layer, variable.as_deref(), &options));
        let layer = read.map_err(err)?;
        Ok(PyArrowLayer {
            schema: layer.schema.clone(),
            name: layer.name.clone(),
            num_rows: layer.num_rows(),
            identity: layer.identity.clone(),
            layer: Some(layer),
        })
    })
}

/// The canonical layer catalog as JSON: `layers` (`name`, `geometry`, `identity`, `aliases`
/// (`consumer`, `name`), `status` `arrow` or `planned`) and `field_aliases` (`consumer`,
/// `layer` in the consumer's naming, `canonical`, `alias`).
#[pyfunction]
fn layer_catalog() -> PyResult<String> {
    let aliases: Vec<serde_json::Value> = layers::FIELD_ALIASES
        .iter()
        .map(|a| {
            serde_json::json!({
                "consumer": a.consumer, "layer": a.layer,
                "canonical": a.canonical, "alias": a.alias,
            })
        })
        .collect();
    let rows: Vec<serde_json::Value> = layers::CATALOG
        .iter()
        .map(|d| {
            serde_json::json!({
                "name": d.name,
                "geometry": d.geometry,
                "identity": [d.identity.0, d.identity.1],
                "aliases": d.aliases.iter()
                    .map(|(c, a)| serde_json::json!({"consumer": c, "name": a}))
                    .collect::<Vec<_>>(),
                "status": if d.status == layers::LayerStatus::Arrow { "arrow" } else { "planned" },
            })
        })
        .collect();
    serde_json::to_string(&serde_json::json!({"layers": rows, "field_aliases": aliases}))
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyArrowLayer>()?;
    m.add_function(wrap_pyfunction!(arrow_layer, m)?)?;
    m.add_function(wrap_pyfunction!(layer_catalog, m)?)?;
    Ok(())
}
