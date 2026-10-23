# Performance

How fast ras-commander's public methods are with the native engine compared
with h5py, measured for the 0.1.0 release.

![Native speed-up by public method](images/speedup-by-method-light.svg)

![HdfMesh.get_mesh_cell_polygons: h5py vs native](images/mesh-polygons-light.svg)

## Results

Repeat calls (warm) on the heaviest test file for each method, medians of
three fresh processes. *Native* is the default `auto` mode; `native` mode gave
the same timings within noise. Speed-up is h5py time divided by native time.

| Method | Result rows | h5py | Native | Speed-up |
|---|---:|---:|---:|---:|
| `HdfMesh.get_mesh_cell_polygons` | 340,246 | 17,145 ms | 939 ms | 18.3× |
| `HdfResultsMesh.get_mesh_cells_timeseries` (water surface) | 1 | 1,372 ms | 184 ms | 7.5× |
| `HdfResultsMesh.get_mesh_timeseries` (water surface) | 193 | 1,415 ms | 213 ms | 6.6× |
| `HdfResultsMesh.get_mesh_timeseries` (face velocity) | 865 | 1,486 ms | 291 ms | 5.1× |
| `HdfFluvialPluvial.calculate_fluvial_pluvial_boundary` | 4,569 | 10,452 ms | 6,831 ms | 1.5× |
| `HdfMesh.get_mesh_cell_points` | 352,141 | 262 ms | 251 ms | 1.0× |
| `HdfResultsMesh.get_mesh_max_ws` | 340,248 | 489 ms | 493 ms | 1.0× |
| `HdfResultsMesh.get_mesh_summary` (maximum face velocity) | 177,301 | 1,732 ms | 1,745 ms | 1.0× |
| `HdfResultsMesh.get_mesh_max_face_v` | 705,332 | 6,974 ms | 7,039 ms | 1.0× |
| `HdfXsec.get_cross_sections` | 1,769 | 81 ms | 84 ms | 1.0× |
| `HdfResultsPlan.get_volume_accounting` | 1 | 1.1 ms | 1.2 ms | 0.9× |
| `HdfBase.get_unsteady_timestamps` | 865 | 5.6 ms | 6.3 ms | 0.9× |
| `HdfBndry.get_bc_lines` | 13 | 12 ms | 15 ms | 0.8× |
| `HdfResultsMesh.get_boundary_conditions_timeseries` | 4 | 51 ms | 62 ms | 0.8× |
| `HdfStruc.get_structures` | 835 | 34 ms | 42 ms | 0.8× |
| `HdfResultsPlan.get_runtime_data` | 1 | 2.7 ms | 3.5 ms | 0.8× |
| `HdfMesh.get_mesh_area_attributes` | 0 | 3.5 ms | 6.8 ms | 0.5× |
| `HdfResultsXsec.get_ref_lines_timeseries` | 11 | 13 ms | 27 ms | 0.5× |
| `HdfResultsXsec.get_xsec_timeseries` | 5 | 13 ms | 29 ms | 0.4× |

- Across all 50 method × file cases (geometric mean), the native engine is
  **1.07×** faster on the first call in a process and **1.30×** faster on
  repeat calls.
- Every case returned identical data in `h5py`, `auto` and `native` mode: the
  benchmark hashed the normalised result (values, dtypes, columns, index, CRS
  and geometry) of every run. There were 0 errors in 600 runs.
- Results with few rows can still be expensive: `get_mesh_cells_timeseries`
  returns one cell's series but must locate it in a large compressed dataset.

## Method

- **Harness:** each measurement runs in a fresh Python process. The mode is
  chosen only through `RAS_COMMANDER_FAST_HDF` (`h5py`, `auto`, `native`). The
  three modes are interleaved (h5py, auto, native, native, auto, h5py, ...) so
  drift affects them equally. Each process records the first call and two
  repeat calls.
- **Cases:** 19 public methods on 7 HEC-RAS result files, from a 3,314-cell
  mesh to a 340,248-cell mesh with 705,332 faces.
- **Machine:** Windows 11, Intel Core i7-11700K (8 logical CPUs available to
  the VM), 48 GB RAM, Python 3.13, h5py 3.14 (HDF5 1.14.6), numpy 2.3, pandas
  2.3. No other heavy process ran during the 31-minute run (CPU monitored
  every 30 s).
- **Cache:** the operating system's file cache was warm for every column;
  "first call" means a fresh process, not a cold disk. Cold-disk reads were
  not measured.

## Reading the numbers

- **Large gains** come from work the engine does in Rust that ras-commander
  used to do in Python: assembling 2D cell polygons from face topology, and
  selecting rows or cells from compressed time-series chunks without
  decompressing more than needed.
- **No gain** for whole-array summaries: both paths decompress the same chunks
  with zlib, and that dominates.
- **Small losses** on small reads come from a fixed cost per native read
  (opening the session, proving the layout). They are a few milliseconds per
  call and are the main target for future optimisation.
