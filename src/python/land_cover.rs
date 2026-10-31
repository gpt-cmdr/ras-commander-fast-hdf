//! Python surface of the Final n Values rule (`crate::land_cover`) and gridded precipitation
//! rasters (`crate::precipitation`). The engine reads and computes; the caller reads the
//! land-cover class TIFF window and writes the GeoTIFFs (terrio or rasterio).
use crate::hdf::ReadOnlyHdfSession;
use crate::land_cover::{self as lc, ClassWindow, Grid, TileCorner};
use crate::precipitation::{self as precip, PrecipitationGrid, Source};
use crate::python::reader::err;
use ndarray::Array2;
use numpy::{IntoPyArray, PyArray2, PyReadonlyArray2};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

type GridTuple = (f64, f64, f64, usize, usize);

fn grid(t: GridTuple) -> PyResult<Grid> {
    let (min_x, max_y, cell_size, rows, cols) = t;
    if !(min_x.is_finite() && max_y.is_finite() && cell_size.is_finite() && cell_size > 0.0)
        || rows == 0
        || cols == 0
    {
        return Err(PyValueError::new_err(
            "grid must be (min_x, max_y, cell_size > 0, rows > 0, cols > 0)",
        ));
    }
    Ok(Grid {
        min_x,
        max_y,
        cell_size,
        rows,
        cols,
    })
}
fn tuple(g: &Grid) -> GridTuple {
    (g.min_x, g.max_y, g.cell_size, g.rows, g.cols)
}

/// RAS Mapper 7.0.1's "Final n Values" rule for one geometry (pure 2D geometries).
///
/// `FinalManningsModel(geometry_hdf, land_cover_hdf=None)` reads the geometry's land-cover
/// association, classification, calibration table and regions and 2D flow areas, or raises
/// `UnsupportedError` for layouts it does not reproduce (cross sections, pipes, virtual
/// land cover, multi-part polygons, ...). Grids are `(min_x, max_y, cell_size, rows, cols)`.
#[pyclass(module = "ras_commander_fast_hdf", frozen)]
pub struct FinalManningsModel {
    inner: lc::FinalManningsModel,
}

#[pymethods]
impl FinalManningsModel {
    #[new]
    #[pyo3(signature = (geometry_hdf, land_cover_hdf=None))]
    fn new(
        py: Python<'_>,
        geometry_hdf: PathBuf,
        land_cover_hdf: Option<PathBuf>,
    ) -> PyResult<Self> {
        crate::python::panic_guard::guard(|| {
            let inner = py
                .detach(|| lc::FinalManningsModel::open(&geometry_hdf, land_cover_hdf.as_deref()))
                .map_err(err)?;
            Ok(Self { inner })
        })
    }
    #[getter]
    fn geometry_hdf(&self) -> String {
        self.inner.geometry_hdf.to_string_lossy().into_owned()
    }
    #[getter]
    fn land_cover_hdf(&self) -> String {
        self.inner.land_cover_hdf.to_string_lossy().into_owned()
    }
    /// The classification raster RAS Mapper reads: the land-cover HDF with extension `.tif`.
    #[getter]
    fn land_cover_tif(&self) -> String {
        self.inner.land_cover_tif.to_string_lossy().into_owned()
    }
    /// Geometry extent `(min_x, max_x, min_y, max_y)`.
    #[getter]
    fn extent(&self) -> (f64, f64, f64, f64) {
        let e = self.inner.extent;
        (e[0], e[1], e[2], e[3])
    }
    #[getter]
    fn region_names(&self) -> Vec<String> {
        self.inner.region_names.clone()
    }
    /// `[{"id", "name", "land_cover_n", "base_n"}]` in RAS Mapper's classification order.
    #[getter]
    fn classes<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        self.inner
            .classes
            .iter()
            .map(|c| {
                let d = PyDict::new(py);
                d.set_item("id", c.id)?;
                d.set_item("name", &c.name)?;
                d.set_item("land_cover_n", c.land_cover_n)?;
                d.set_item("base_n", c.base_n)?;
                Ok(d)
            })
            .collect()
    }
    /// `[{"name", "mannings_n", "cell_count"}]`.
    #[getter]
    fn flow_areas<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        self.inner
            .flow_areas
            .iter()
            .map(|a| {
                let d = PyDict::new(py);
                d.set_item("name", &a.name)?;
                d.set_item("mannings_n", a.mannings_n)?;
                d.set_item("cell_count", a.cell_count)?;
                Ok(d)
            })
            .collect()
    }
    /// RAS Mapper's default Export Raster grid for `cell_size` (the smallest land-cover TIFF
    /// cell size): `(min_x, max_y, cell_size, rows, cols)`.
    fn export_grid(&self, cell_size: f64) -> PyResult<GridTuple> {
        self.inner
            .export_grid(cell_size)
            .map(|g| tuple(&g))
            .map_err(err)
    }
    /// Number of 256-row tile rows of `grid`.
    #[staticmethod]
    fn tile_rows(grid_: GridTuple) -> PyResult<usize> {
        Ok(lc::FinalManningsModel::tile_rows(&grid(grid_)?))
    }
    /// The class-raster window `(row_off, col_off, height, width)` that
    /// `render_tile_row(tile_row)` reads, clipped to `source` (the class raster's grid), or
    /// `None` when that tile row does not overlap the raster.
    #[staticmethod]
    fn class_window(
        grid_: GridTuple,
        tile_row: usize,
        source: GridTuple,
    ) -> PyResult<Option<(usize, usize, usize, usize)>> {
        Ok(lc::FinalManningsModel::class_window(
            &grid(grid_)?,
            tile_row,
            &grid(source)?,
        ))
    }
    /// Evaluate one row of 256 x 256 export tiles: a float32 array of
    /// `min(256, rows - 256*tile_row)` x `cols`, no data -9999.
    ///
    /// `classes` is the int32 class window `class_window()` named, at (`row_off`, `col_off`)
    /// of the class raster `source`; `None` when the tile row does not overlap it.
    /// `rasmapper_tile_corner=True` reproduces RAS Mapper 7.0.1, which leaves the last pixel
    /// of every 256 x 256 tile at its 2D-area default; `False` evaluates that pixel too.
    #[pyo3(signature = (grid_, tile_row, classes=None, row_off=0, col_off=0, source=None, rasmapper_tile_corner=true))]
    #[allow(clippy::too_many_arguments)]
    fn render_tile_row<'py>(
        &self,
        py: Python<'py>,
        grid_: GridTuple,
        tile_row: usize,
        classes: Option<PyReadonlyArray2<'py, i32>>,
        row_off: usize,
        col_off: usize,
        source: Option<GridTuple>,
        rasmapper_tile_corner: bool,
    ) -> PyResult<Bound<'py, PyArray2<f32>>> {
        crate::python::panic_guard::guard(|| {
            let g = grid(grid_)?;
            let corner = if rasmapper_tile_corner {
                TileCorner::RasMapper
            } else {
                TileCorner::Evaluate
            };
            let owned: Option<(Vec<i32>, usize, usize, Grid)> = match (&classes, source) {
                (Some(a), Some(s)) => {
                    let v = a.as_array();
                    let (h, w) = (v.shape()[0], v.shape()[1]);
                    Some((v.iter().copied().collect(), h, w, grid(s)?))
                }
                (None, _) => None,
                (Some(_), None) => {
                    return Err(PyValueError::new_err("classes needs its source grid"))
                }
            };
            let out = py
                .detach(|| {
                    let window = owned.as_ref().map(|(data, h, w, s)| ClassWindow {
                        data,
                        row_off,
                        col_off,
                        height: *h,
                        width: *w,
                        source: *s,
                    });
                    self.inner
                        .render_tile_row(&g, tile_row, window.as_ref(), corner)
                })
                .map_err(err)?;
            let rows = out.len() / g.cols;
            let a = Array2::from_shape_vec((rows, g.cols), out)
                .map_err(|e| PyValueError::new_err(e.to_string()))?;
            Ok(a.into_pyarray(py))
        })
    }
}

fn info_dict<'py>(py: Python<'py>, g: &PrecipitationGrid) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("source", &g.source)?;
    d.set_item("values_path", &g.values_path)?;
    d.set_item("timestamps", &g.timestamps)?;
    d.set_item("rows", g.rows)?;
    d.set_item("cols", g.cols)?;
    d.set_item("cellsize", g.cellsize)?;
    d.set_item("left", g.left)?;
    d.set_item("top", g.top)?;
    d.set_item("projection", &g.projection)?;
    d.set_item("units", &g.units)?;
    d.set_item("data_type", &g.data_type)?;
    d.set_item("nodata", g.nodata)?;
    d.set_item("source_is_cumulative", g.source_is_cumulative)?;
    Ok(d)
}

/// Gridded precipitation definition of a plan or unsteady-flow HDF: the fields of ras2cng's
/// `PrecipitationGridInfo` (without `hdf_path`) as a dict.
#[pyfunction]
#[pyo3(signature = (path, source="auto"))]
pub fn precipitation_grid_info<'py>(
    py: Python<'py>,
    path: PathBuf,
    source: &str,
) -> PyResult<Bound<'py, PyDict>> {
    crate::python::panic_guard::guard(|| {
        let src = Source::parse(source).map_err(err)?;
        let g = py
            .detach(|| {
                let s = ReadOnlyHdfSession::open(&path)?;
                PrecipitationGrid::open(&s, src)
            })
            .map_err(err)?;
        info_dict(py, &g)
    })
}

/// Incremental and cumulative precipitation grids, one timestep at a time, exactly as ras2cng's
/// `export_precipitation_rasters` computes them.
///
/// `PrecipitationSteps(path, source="auto", indices=None, units="native", max_bytes=None)`
/// iterates `(index, incremental, cumulative)` in time order: float32 `rows x cols` arrays,
/// missing cells NaN, or `NoData` when the grid has one. `units` is `native`, `in` or `mm`.
#[pyclass(module = "ras_commander_fast_hdf", frozen)]
pub struct PrecipitationSteps {
    inner: Mutex<precip::PrecipitationSteps>,
}

#[pymethods]
impl PrecipitationSteps {
    #[new]
    #[pyo3(signature = (path, source="auto", indices=None, units="native", max_bytes=None))]
    fn new(
        py: Python<'_>,
        path: PathBuf,
        source: &str,
        indices: Option<Vec<usize>>,
        units: &str,
        max_bytes: Option<usize>,
    ) -> PyResult<Self> {
        crate::python::panic_guard::guard(|| {
            let src = Source::parse(source).map_err(err)?;
            let budget = max_bytes.unwrap_or(crate::hdf::DEFAULT_READ_MAX_BYTES);
            let inner = py
                .detach(|| {
                    let s = ReadOnlyHdfSession::open(&path)?;
                    let g = PrecipitationGrid::open(&s, src)?;
                    precip::PrecipitationSteps::new(s, g, indices.as_deref(), units, budget)
                })
                .map_err(err)?;
            Ok(Self {
                inner: Mutex::new(inner),
            })
        })
    }
    /// The grid definition, as `precipitation_grid_info`.
    #[getter]
    fn info<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        info_dict(py, self.lock()?.grid())
    }
    /// The selected timesteps, in the requested order.
    #[getter]
    fn indices(&self) -> PyResult<Vec<usize>> {
        Ok(self.lock()?.indices().to_vec())
    }
    /// The conversion's units tag, `None` for `native`.
    #[getter]
    fn units(&self) -> PyResult<Option<String>> {
        Ok(self.lock()?.units().map(String::from))
    }
    fn __iter__(slf: Bound<'_, Self>) -> Bound<'_, Self> {
        slf
    }
    #[allow(clippy::type_complexity)]
    fn __next__<'py>(
        &self,
        py: Python<'py>,
    ) -> PyResult<Option<(usize, Bound<'py, PyArray2<f32>>, Bound<'py, PyArray2<f32>>)>> {
        crate::python::panic_guard::guard(|| {
            let (step, shape) = py.detach(|| {
                let mut s = self.lock()?;
                let shape = (s.grid().rows, s.grid().cols);
                Ok::<_, PyErr>((s.next_step().map_err(err)?, shape))
            })?;
            let Some(step) = step else { return Ok(None) };
            let to_py = |v: Vec<f32>| -> PyResult<Bound<'py, PyArray2<f32>>> {
                Ok(Array2::from_shape_vec(shape, v)
                    .map_err(|e| PyValueError::new_err(e.to_string()))?
                    .into_pyarray(py))
            };
            Ok(Some((
                step.index,
                to_py(step.incremental)?,
                to_py(step.cumulative)?,
            )))
        })
    }
}

impl PrecipitationSteps {
    fn lock(&self) -> PyResult<MutexGuard<'_, precip::PrecipitationSteps>> {
        self.inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("precipitation reader poisoned by a panic"))
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<FinalManningsModel>()?;
    m.add_function(wrap_pyfunction!(precipitation_grid_info, m)?)?;
    m.add_class::<PrecipitationSteps>()?;
    Ok(())
}
