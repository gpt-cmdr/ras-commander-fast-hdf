mod analysis;
#[cfg(feature = "python-arrow")]
#[forbid(unsafe_code)]
mod arrow_stream;
mod domain;
mod fim;
#[allow(dead_code)] // pure-Rust kernels; parts are reached only from tests
pub mod fluvial;
mod fluvial_bindings;
#[allow(dead_code)] // pure-Rust kernels; parts are reached only from tests
pub mod fluvial_geometry;
#[allow(dead_code)] // pure-Rust kernels; parts are reached only from tests
pub mod fluvial_topology;
mod land_cover;
mod mesh;
mod one_d;
mod panic_guard;
#[allow(dead_code, unused_imports)] // parser re-exports; parts are reached only from tests
pub mod ras_text;
mod reader;
mod session;
mod structures;
mod text_bindings;
mod tile_engine;
mod tiles;
mod wave3;

pub(crate) static REGISTERED_BINDINGS: OnceLock<Vec<String>> = OnceLock::new();

pub(crate) const API_VERSION: u32 = 11;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

pyo3::create_exception!(ras_commander_fast_hdf, UnsupportedError, PyValueError);
pyo3::create_exception!(ras_commander_fast_hdf, TopologyError, PyValueError);
pyo3::create_exception!(ras_commander_fast_hdf, CorruptDataError, PyValueError);

pub(crate) fn pool(threads: Option<usize>) -> PyResult<Arc<rayon::ThreadPool>> {
    let count = reader::threads(threads)?;
    static POOLS: OnceLock<Mutex<HashMap<usize, Arc<rayon::ThreadPool>>>> = OnceLock::new();
    // Bound implicit concurrency; caller-level archive workers also consume CPUs.

    let mut cache = POOLS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        // Pools are immutable once built; a panic elsewhere cannot corrupt the map.
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(pool) = cache.get(&count) {
        return Ok(pool.clone());
    }
    let pool = Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(count)
            .build()
            .map_err(|e| PyValueError::new_err(e.to_string()))?,
    );
    // Bound retained pools even when applications vary worker counts repeatedly.
    if cache.len() >= 8 {
        if let Some(key) = cache.keys().next().copied() {
            cache.remove(&key);
        }
    }
    cache.insert(count, pool.clone());
    Ok(pool)
}

#[cfg(feature = "exports-arrow")]
#[pyfunction]
#[pyo3(signature = (input, output, format, max_batch_rows=65536))]
fn export_ipc_file(
    py: Python<'_>,
    input: std::path::PathBuf,
    output: std::path::PathBuf,
    format: &str,
    max_batch_rows: usize,
) -> PyResult<String> {
    crate::python::panic_guard::guard(|| {
        let format = format
            .parse::<crate::exports::Format>()
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        py.detach(move || {
            let metrics = crate::exports::export_ipc(&input, &output, format, max_batch_rows)
                .map_err(|e| PyValueError::new_err(e.to_string()))?;
            serde_json::to_string(&metrics).map_err(|e| PyValueError::new_err(e.to_string()))
        })
    })
}

#[cfg(feature = "exports-pmtiles")]
#[pyfunction]
#[pyo3(signature = (input, output, options, threads=None))]
fn export_pmtiles_ipc_file(
    py: Python<'_>,
    input: std::path::PathBuf,
    output: std::path::PathBuf,
    options: &str,
    threads: Option<usize>,
) -> PyResult<String> {
    crate::python::panic_guard::guard(|| {
        let options: crate::exports::pmtiles_bridge::Options =
            serde_json::from_str(options).map_err(|e| PyValueError::new_err(e.to_string()))?;
        let pool = pool(threads)?;
        py.detach(move || {
            pool.install(|| {
                let metrics = crate::exports::pmtiles_bridge::export_ipc(&input, &output, &options)
                    .map_err(|e| PyValueError::new_err(e.to_string()))?;
                serde_json::to_string(&metrics).map_err(|e| PyValueError::new_err(e.to_string()))
            })
        })
    })
}

// Validate original Arrow WKB buffers before a reprojection library can
// normalize away unsupported encodings or trailing bytes. Arrays borrow Arrow
// storage; no per-feature Python objects or geometry copies are needed.
#[cfg(feature = "exports-arrow")]
#[pyfunction]
fn validate_export_wkb(
    offsets: numpy::PyReadonlyArray1<'_, i32>,
    data: numpy::PyReadonlyArray1<'_, u8>,
    validity: Option<numpy::PyReadonlyArray1<'_, u8>>,
    array_offset: usize,
    count: usize,
) -> PyResult<()> {
    crate::python::panic_guard::guard(|| {
        let offsets = offsets.as_slice()?;
        let data = data.as_slice()?;
        let validity = validity.as_ref().map(|v| v.as_slice()).transpose()?;
        let end = array_offset
            .checked_add(count)
            .ok_or_else(|| PyValueError::new_err("WKB array offset overflow"))?;
        if end >= offsets.len() {
            return Err(PyValueError::new_err("WKB offsets outside buffer"));
        }
        if let Some(bits) = validity {
            if count > 0 && (end - 1) / 8 >= bits.len() {
                return Err(PyValueError::new_err("WKB validity outside buffer"));
            }
        }
        for i in array_offset..end {
            let start = usize::try_from(offsets[i])
                .map_err(|_| PyValueError::new_err("negative WKB offset"))?;
            let stop = usize::try_from(offsets[i + 1])
                .map_err(|_| PyValueError::new_err("negative WKB offset"))?;
            let bytes = data
                .get(start..stop)
                .ok_or_else(|| PyValueError::new_err("WKB offsets outside data"))?;
            if validity.is_none_or(|bits| bits[i / 8] & (1 << (i % 8)) != 0) {
                crate::exports::contract::validate_wkb(bytes, false)
                    .map_err(|e| PyValueError::new_err(e.to_string()))?;
            }
        }
        Ok(())
    })
}

#[pymodule]
fn ras_commander_fast_hdf(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Contained panics raise a Python exception carrying the message; do not
    // also print Rust's default panic message.
    panic_guard::install_quiet_hook();
    m.add("ENGINE_COMMIT", env!("RAS_ENGINE_COMMIT"))?;
    m.add(
        "ENGINE_FEATURES",
        pyo3::types::PyTuple::new(
            m.py(),
            env!("RAS_ENGINE_FEATURES").split(',').collect::<Vec<_>>(),
        )?,
    )?;
    m.add("API_VERSION", API_VERSION)?;
    // Additive (API 11 unchanged): path-based typed entry points also accept an
    // open HdfSession as their first argument (see session::Source).
    m.add("SESSION_SOURCES", true)?;
    m.add("CorruptDataError", m.py().get_type::<CorruptDataError>())?;
    panic_guard::register(m)?;
    // Panic-injection hook for ras-commander's panic-containment tests; release
    // wheels are built without "test-hooks".
    #[cfg(feature = "test-hooks")]
    m.add_function(wrap_pyfunction!(panic_guard::_panic_for_test, m)?)?;
    domain::register(m)?;
    fim::register(m)?;
    session::register(m)?;
    structures::register(m)?;
    one_d::register(m)?;
    wave3::register(m)?;
    #[cfg(feature = "exports-arrow")]
    {
        m.add_function(wrap_pyfunction!(export_ipc_file, m)?)?;
        m.add_function(wrap_pyfunction!(validate_export_wkb, m)?)?;
    }
    #[cfg(feature = "python-arrow")]
    arrow_stream::register(m)?;
    #[cfg(feature = "exports-pmtiles")]
    m.add_function(wrap_pyfunction!(export_pmtiles_ipc_file, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("UnsupportedError", m.py().get_type::<UnsupportedError>())?;
    m.add("TopologyError", m.py().get_type::<TopologyError>())?;
    m.add_function(wrap_pyfunction!(reader::read_numeric, m)?)?;
    m.add_function(wrap_pyfunction!(reader::read_numeric_batch, m)?)?;
    m.add_function(wrap_pyfunction!(reader::numeric_capability, m)?)?;
    m.add_function(wrap_pyfunction!(mesh::assemble_cells, m)?)?;
    m.add_function(wrap_pyfunction!(analysis::bindings::temporal_reduce, m)?)?;
    m.add_function(wrap_pyfunction!(tiles::write_pmtiles, m)?)?;
    fluvial_bindings::register(m)?;
    land_cover::register(m)?;
    text_bindings::register(m)?;
    let mut names = Vec::new();
    for (name, value) in m.dict().iter() {
        let is_exception = if let Ok(t) = value.cast::<pyo3::types::PyType>() {
            t.is_subclass_of::<pyo3::exceptions::PyBaseException>()?
        } else {
            false
        };
        if value.is_callable() && !is_exception {
            names.push(name.extract::<String>()?);
        }
    }
    names.sort();
    if let Some(previous) = REGISTERED_BINDINGS.get() {
        if previous != &names {
            return Err(pyo3::exceptions::PyRuntimeError::new_err(
                "module registrations changed",
            ));
        }
    } else {
        let _ = REGISTERED_BINDINGS.set(names);
    }

    Ok(())
}

#[cfg(test)]
mod mesh_attribution;

#[cfg(test)]
mod perf3_attribution;
