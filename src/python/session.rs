//! Explicit read-only session. Closing waits for in-flight reads and drops mappings.
use crate::hdf::{ReadOnlyHdfSession, Slice};
use crate::python::domain::identity;
use crate::python::fim::{inventory, selection, slice};
use numpy::IntoPyArray;
use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};
use std::{
    path::PathBuf,
    sync::{Mutex, MutexGuard, PoisonError},
};
macro_rules! dict { ($py:expr, $($k:expr => $v:expr),* $(,)?) => {{ let d=PyDict::new($py); $(d.set_item($k,$v)?;)* d }}; }
#[pyclass]
pub(crate) struct HdfSession {
    inner: Mutex<Option<ReadOnlyHdfSession>>,
}
impl HdfSession {
    /// The session slot. Engine calls are panic-contained while the lock is
    /// held, so it is not poisoned in practice; a poisoned lock (a panic
    /// elsewhere) is still recovered, because the session is read-only and
    /// stays consistent, so later reads and `close()` keep working.
    fn slot(&self) -> MutexGuard<'_, Option<ReadOnlyHdfSession>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
    pub(crate) fn with<T: Send>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&ReadOnlyHdfSession) -> crate::Result<T> + Send,
    ) -> PyResult<T> {
        let outcome = py.detach(|| {
            let guard = self.slot();
            let s = guard.as_ref()?;
            // Catch a panic before the lock guard unwinds; build the Python
            // error after the lock is released and the GIL is re-acquired.
            Some(crate::python::panic_guard::contain(|| f(s)))
        });
        match outcome {
            None => Err(PyValueError::new_err("HDF session is closed")),
            Some(Ok(result)) => result.map_err(crate::python::reader::err),
            Some(Err(message)) => Err(crate::python::panic_guard::panic_error_from(message)),
        }
    }
}
#[pymethods]
impl HdfSession {
    #[new]
    #[pyo3(signature=(path,threads=None,parallel=true,*,cache_bytes=None,cache_slots=None))]
    fn new(
        py: Python<'_>,
        path: PathBuf,
        threads: Option<usize>,
        parallel: bool,
        cache_bytes: Option<usize>,
        cache_slots: Option<usize>,
    ) -> PyResult<Self> {
        crate::python::panic_guard::guard(|| {
            let count = crate::python::reader::threads(threads)?;
            let s = py
                .detach(|| {
                    ReadOnlyHdfSession::with_cache_options(
                        path,
                        count,
                        parallel,
                        cache_bytes.unwrap_or(crate::hdf::DEFAULT_CHUNK_CACHE_BYTES),
                        cache_slots.unwrap_or(crate::hdf::DEFAULT_CHUNK_CACHE_SLOTS),
                    )
                })
                .map_err(crate::python::reader::err)?;
            Ok(Self {
                inner: Mutex::new(Some(s)),
            })
        })
    }
    fn cache_stats<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        crate::python::panic_guard::guard(|| {
            let c = self.with(py, |s| Ok(s.chunk_cache_stats()))?;
            Ok(
                dict!(py,"hits"=>c.hits,"misses"=>c.misses,"inserts"=>c.inserts,"evictions"=>c.evictions,"retained_bytes"=>c.retained_bytes,"entries"=>c.entries,"max_bytes"=>c.max_bytes,"max_slots"=>c.max_slots),
            )
        })
    }
    #[pyo3(signature=(area,variable,elements=None,*,metadata_max_bytes=67108864,max_bytes=67108864))]
    fn read_mesh_summary_values<'py>(
        &self,
        py: Python<'py>,
        area: &str,
        variable: &str,
        elements: Option<numpy::PyReadonlyArray1<'_, u64>>,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        crate::python::panic_guard::guard(|| {
            let elements = elements
                .map(|a| a.as_slice().map(|s| s.to_vec()))
                .transpose()?;
            let a = self.with(py, |s| {
                s.read_mesh_summary_values(
                    area,
                    variable,
                    elements.as_deref(),
                    metadata_max_bytes,
                    max_bytes,
                )
            })?;
            crate::python::reader::array(py, a)
        })
    }
    #[pyo3(signature=(area,rows,names=None,*,metadata_max_bytes=67108864,max_bytes=67108864))]
    fn read_mesh_boundary_batch<'py>(
        &self,
        py: Python<'py>,
        area: &str,
        rows: (u64, u64, u64),
        names: Option<Vec<String>>,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> PyResult<Bound<'py, PyDict>> {
        crate::python::panic_guard::guard(|| {
            let names = names
                .as_ref()
                .map(|n| n.iter().map(String::as_str).collect::<Vec<_>>());
            let result = self.with(py, |s| {
                s.read_mesh_boundary_batch(
                    area,
                    names.as_deref(),
                    slice(rows),
                    metadata_max_bytes,
                    max_bytes,
                )
            })?;
            let d = PyDict::new(py);
            for (name, r) in result {
                d.set_item(name, selection(py, r)?)?;
            }
            Ok(d)
        })
    }
    /// Every face of mesh `area` as packed polylines (GeoArrow linestring layout):
    /// `coordinates` float64 `(N, 2)`, `offsets` int64 `(faces + 1,)` and `face_ids`
    /// int64; line `i` is `coordinates[offsets[i]:offsets[i + 1]]`.
    #[pyo3(signature=(area,max_bytes=crate::hdf::DEFAULT_READ_MAX_BYTES))]
    fn read_mesh_face_lines<'py>(
        &self,
        py: Python<'py>,
        area: &str,
        max_bytes: usize,
    ) -> PyResult<Bound<'py, PyDict>> {
        crate::python::panic_guard::guard(|| {
            let lines = self.with(py, |s| s.read_mesh_face_lines(area, max_bytes))?;
            let rows = lines.coordinates.len();
            let coordinates =
                ndarray::Array2::from_shape_vec((rows, 2), lines.coordinates.into_flattened())
                    .map_err(|e| PyValueError::new_err(e.to_string()))?;
            let offsets = lines
                .offsets
                .into_iter()
                .map(|n| {
                    i64::try_from(n).map_err(|_| PyValueError::new_err("coordinate count overflow"))
                })
                .collect::<PyResult<Vec<_>>>()?;
            Ok(dict!(py,
                "coordinates"=>coordinates.into_pyarray(py),
                "offsets"=>offsets.into_pyarray(py),
                "face_ids"=>lines.face_ids.into_pyarray(py)))
        })
    }
    /// Test hook (feature `test-hooks`): panic inside an engine call while the
    /// session lock is held.
    #[cfg(feature = "test-hooks")]
    fn _panic_for_test(&self, py: Python<'_>) -> PyResult<()> {
        crate::python::panic_guard::guard(|| {
            self.with(py, |_| -> crate::Result<()> {
                panic!("injected panic while the session lock is held")
            })
        })
    }
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        crate::python::panic_guard::guard(|| {
            py.detach(|| {
                self.slot().take();
                Ok(())
            })
        })
    }
    #[getter]
    fn closed(&self) -> PyResult<bool> {
        crate::python::panic_guard::guard(|| Ok(self.slot().is_none()))
    }
    fn __enter__(slf: PyRef<'_, Self>) -> PyResult<PyRef<'_, Self>> {
        crate::python::panic_guard::guard(|| {
            if slf.closed()? {
                return Err(PyValueError::new_err("HDF session is closed"));
            }
            Ok(slf)
        })
    }
    fn __exit__(
        &self,
        py: Python<'_>,
        _ty: &Bound<'_, PyAny>,
        _value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        crate::python::panic_guard::guard(|| self.close(py))
    }
    #[pyo3(signature=(dataset_path,selection=None))]
    fn read_numeric<'py>(
        &self,
        py: Python<'py>,
        dataset_path: &str,
        selection: Option<Vec<(u64, u64, u64)>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        crate::python::panic_guard::guard(|| {
            let sel = selection.map(|s| {
                s.into_iter()
                    .map(|(a, b, c)| Slice::new(a, b, c))
                    .collect::<Vec<_>>()
            });
            let a = self.with(py, |s| s.read_numeric(dataset_path, sel.as_deref()))?;
            crate::python::reader::array(py, a)
        })
    }
    /// Fill `out` with one full numeric dataset without replacing its allocation.
    /// `out` must have the exact native dtype and shape, be aligned, C-contiguous
    /// and writable. The caller must prevent concurrent access to `out` or its
    /// aliases, including NumPy/native operations that already released the GIL.
    /// Preflight rejection leaves `out` untouched; a decode/I/O failure may leave
    /// partial writes, so treat the whole buffer as invalid after such an error.
    #[pyo3(signature=(dataset_path,out,*,max_bytes=crate::hdf::DEFAULT_READ_MAX_BYTES))]
    fn read_numeric_into(
        &self,
        dataset_path: &str,
        out: &Bound<'_, PyAny>,
        max_bytes: usize,
    ) -> PyResult<()> {
        use numpy::{PyArrayMethods, PyUntypedArrayMethods};
        crate::python::panic_guard::guard(|| {
            macro_rules! fill {
                ($ty:ty,$kind:ident) => {
                    if let Ok(array) = out.cast::<numpy::PyArrayDyn<$ty>>() {
                        if !array.is_c_contiguous() || !array.is_aligned() {
                            return Err(PyValueError::new_err(
                                "out must be aligned and C-contiguous",
                            ));
                        }
                        let shape = array.shape().iter().map(|&n| n as u64).collect::<Vec<_>>();
                        let mut borrowed = array
                            .try_readwrite()
                            .map_err(|e| PyValueError::new_err(e.to_string()))?;
                        let destination = borrowed.as_slice_mut()?;
                        // Unlike with(), keep the GIL while writing borrowed NumPy
                        // memory. The caller's exclusive-access contract is still required.
                        let guard = self.slot();
                        let session = guard
                            .as_ref()
                            .ok_or_else(|| PyValueError::new_err("HDF session is closed"))?;
                        let result = crate::python::panic_guard::contain(|| {
                            session.read_numeric_into_slice(
                                dataset_path,
                                &shape,
                                crate::hdf::NumericKind::$kind,
                                destination,
                                max_bytes,
                            )
                        });
                        return match result {
                            Ok(result) => result.map_err(crate::python::reader::err),
                            Err(message) => {
                                Err(crate::python::panic_guard::panic_error_from(message))
                            }
                        };
                    }
                };
            }
            fill!(i8, I8);
            fill!(i16, I16);
            fill!(i32, I32);
            fill!(i64, I64);
            fill!(u8, U8);
            fill!(u16, U16);
            fill!(u32, U32);
            fill!(u64, U64);
            fill!(f32, F32);
            fill!(f64, F64);
            Err(pyo3::exceptions::PyTypeError::new_err(
                "out must be a NumPy array with an exact native numeric dtype",
            ))
        })
    }
    fn read_numeric_batch<'py>(
        &self,
        py: Python<'py>,
        dataset_paths: Vec<String>,
    ) -> PyResult<Bound<'py, PyDict>> {
        crate::python::panic_guard::guard(|| {
            let names = dataset_paths.iter().map(String::as_str).collect::<Vec<_>>();
            let arrays = self.with(py, |s| s.read_numeric_batch(&names))?;
            let d = PyDict::new(py);
            for (n, a) in arrays {
                d.set_item(n, crate::python::reader::array(py, a)?)?;
            }
            Ok(d)
        })
    }
    #[pyo3(signature=(area,max_bytes=67108864))]
    fn mesh_summary_inventory<'py>(
        &self,
        py: Python<'py>,
        area: &str,
        max_bytes: usize,
    ) -> PyResult<Bound<'py, PyDict>> {
        crate::python::panic_guard::guard(|| {
            let i = self.with(py, |session| {
                session.mesh_summary_inventory(area, max_bytes)
            })?;
            inventory(py, i)
        })
    }
    #[pyo3(signature=(area,max_bytes=67108864))]
    fn mesh_series_inventory<'py>(
        &self,
        py: Python<'py>,
        area: &str,
        max_bytes: usize,
    ) -> PyResult<Bound<'py, PyDict>> {
        crate::python::panic_guard::guard(|| {
            let i = self.with(py, |session| session.mesh_series_inventory(area, max_bytes))?;
            inventory(py, i)
        })
    }
    #[pyo3(signature=(area,max_bytes=67108864))]
    fn mesh_boundary_inventory<'py>(
        &self,
        py: Python<'py>,
        area: &str,
        max_bytes: usize,
    ) -> PyResult<Bound<'py, PyDict>> {
        crate::python::panic_guard::guard(|| {
            let i = self.with(py, |session| {
                session.mesh_boundary_inventory(area, max_bytes)
            })?;
            inventory(py, i)
        })
    }
    #[pyo3(signature=(area,variable,elements=None,*,metadata_max_bytes=67108864,max_bytes=67108864))]
    fn read_mesh_summary<'py>(
        &self,
        py: Python<'py>,
        area: &str,
        variable: &str,
        elements: Option<numpy::PyReadonlyArray1<'_, u64>>,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> PyResult<Bound<'py, PyDict>> {
        crate::python::panic_guard::guard(|| {
            let elements = elements
                .map(|a| a.as_slice().map(|s| s.to_vec()))
                .transpose()?;
            let r = self.with(py, |session| {
                session.read_mesh_summary(
                    area,
                    variable,
                    elements.as_deref(),
                    metadata_max_bytes,
                    max_bytes,
                )
            })?;
            selection(py, r)
        })
    }
    #[pyo3(signature=(area,variable,rows,elements=None,*,metadata_max_bytes=67108864,max_bytes=67108864))]
    #[allow(clippy::too_many_arguments)]
    fn read_mesh_series<'py>(
        &self,
        py: Python<'py>,
        area: &str,
        variable: &str,
        rows: (u64, u64, u64),
        elements: Option<numpy::PyReadonlyArray1<'_, u64>>,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> PyResult<Bound<'py, PyDict>> {
        crate::python::panic_guard::guard(|| {
            let elements = elements
                .map(|a| a.as_slice().map(|s| s.to_vec()))
                .transpose()?;
            let r = self.with(py, |session| {
                session.read_mesh_series(
                    area,
                    variable,
                    slice(rows),
                    elements.as_deref(),
                    metadata_max_bytes,
                    max_bytes,
                )
            })?;
            selection(py, r)
        })
    }
    #[pyo3(signature=(area,variable,rows,elements=None,*,metadata_max_bytes=67108864,max_bytes=67108864))]
    #[allow(clippy::too_many_arguments)]
    fn read_mesh_boundary_series<'py>(
        &self,
        py: Python<'py>,
        area: &str,
        variable: &str,
        rows: (u64, u64, u64),
        elements: Option<numpy::PyReadonlyArray1<'_, u64>>,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> PyResult<Bound<'py, PyDict>> {
        crate::python::panic_guard::guard(|| {
            let elements = elements
                .map(|a| a.as_slice().map(|s| s.to_vec()))
                .transpose()?;
            let r = self.with(py, |session| {
                session.read_mesh_boundary_series(
                    area,
                    variable,
                    slice(rows),
                    elements.as_deref(),
                    metadata_max_bytes,
                    max_bytes,
                )
            })?;
            selection(py, r)
        })
    }
    #[pyo3(signature=(area,variable,rows,elements=None,*,metadata_max_bytes=67108864,max_bytes=67108864))]
    #[allow(clippy::too_many_arguments)]
    fn reduce_mesh_series<'py>(
        &self,
        py: Python<'py>,
        area: &str,
        variable: &str,
        rows: (u64, u64, u64),
        elements: Option<numpy::PyReadonlyArray1<'_, u64>>,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> PyResult<Bound<'py, PyDict>> {
        crate::python::panic_guard::guard(|| {
            let elements = elements
                .map(|a| a.as_slice().map(|s| s.to_vec()))
                .transpose()?;
            let r = self.with(py, |session| {
                session.reduce_mesh_series(
                    area,
                    variable,
                    slice(rows),
                    elements.as_deref(),
                    metadata_max_bytes,
                    max_bytes,
                )
            })?;
            Ok(
                dict!(py,"identity"=>identity(py,&r.id)?,"elements"=>r.elements.into_pyarray(py),"values"=>crate::python::reader::array(py,r.values)?,"time_indices"=>r.time_indices.into_pyarray(py)),
            )
        })
    }
}

/// The source of a path-based typed entry point: a file path (opened and closed by
/// the call, as before) or an open [`HdfSession`], whose mapping, chunk cache and
/// session-local metadata memo are then shared with the caller's other reads.
/// The engine computes the same values either way; only the fixed per-call cost
/// (mapping, superblock and object-header parsing, layout and encoding proofs)
/// is saved. A closed session raises `ValueError("HDF session is closed")`.
pub(crate) enum Source {
    Path(PathBuf),
    Session(Py<HdfSession>),
}
impl<'a, 'py> FromPyObject<'a, 'py> for Source {
    type Error = PyErr;
    fn extract(obj: Borrowed<'a, 'py, PyAny>) -> PyResult<Self> {
        if let Ok(session) = obj.cast::<HdfSession>() {
            return Ok(Self::Session(session.to_owned().unbind()));
        }
        // Anything else keeps the path conversion and its exact TypeError.
        obj.extract::<PathBuf>().map(Self::Path)
    }
}
impl Source {
    /// Run one engine call on this source with the GIL released.
    pub(crate) fn run<T: Send>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&ReadOnlyHdfSession) -> crate::Result<T> + Send,
    ) -> PyResult<T> {
        match self {
            Self::Path(path) => py
                .detach(|| f(&ReadOnlyHdfSession::open(path)?))
                .map_err(crate::python::reader::err),
            Self::Session(session) => session.borrow(py).with(py, f),
        }
    }
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<HdfSession>()?;
    Ok(())
}
