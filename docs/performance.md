# Performance

This page compares the speed of ras-commander's public methods with the
native engine and with h5py, measured for the 0.2.0 release (with ras-commander 0.108.0).

The first chart shows the native speed-up for each benchmarked method; the
second shows how the cell-polygon gain changes with mesh size.

![Dot chart of the native speed-up for 19 public methods: cell polygons 18.0×, mesh time series 6.0× to 8.1×, fluvial-pluvial boundary 1.6×, most bulk summary reads about 1.0×, and small reads 0.6× to 1.1×.](images/speedup-by-method-light.svg)

![Bar chart: HdfMesh.get_mesh_cell_polygons takes 17.3 s with h5py and 0.90 s natively on a 340,246-cell mesh, 4.32 s and 0.26 s on 87,039 cells, 0.28 s and 0.06 s on 4,425 cells, 0.23 s and 0.06 s on 3,314 cells.](images/mesh-polygons-light.svg)

## Results

Repeat calls (warm) on the heaviest test file for each method, medians of
three fresh processes. *Native* is the default `auto` mode; `native` mode gave
the same timings within noise. Speed-up is h5py time divided by native time.

| Method | Result rows | h5py | Native | Speed-up |
|---|---:|---:|---:|---:|
| `HdfMesh.get_mesh_cell_polygons` | 340,246 | 17,252 ms | 960 ms | 18.0× |
| `HdfResultsMesh.get_mesh_cells_timeseries` (water surface) | 1 | 1,405 ms | 173 ms | 8.1× |
| `HdfResultsMesh.get_mesh_timeseries` (water surface) | 865 | 1,433 ms | 193 ms | 7.4× |
| `HdfResultsMesh.get_mesh_timeseries` (face velocity) | 865 | 1,516 ms | 253 ms | 6.0× |
| `HdfFluvialPluvial.calculate_fluvial_pluvial_boundary` | 4,569 | 10,638 ms | 6,856 ms | 1.6× |
| `HdfStruc.get_structures` | 835 | 36 ms | 34 ms | 1.1× |
| `HdfXsec.get_cross_sections` | 1,769 | 82 ms | 79 ms | 1.0× |
| `HdfMesh.get_mesh_cell_points` | 352,141 | 260 ms | 255 ms | 1.0× |
| `HdfResultsMesh.get_mesh_max_face_v` | 705,332 | 7,101 ms | 6,977 ms | 1.0× |
| `HdfResultsMesh.get_mesh_summary` (maximum face velocity) | 177,301 | 1,743 ms | 1,718 ms | 1.0× |
| `HdfResultsMesh.get_mesh_max_ws` | 340,248 | 506 ms | 506 ms | 1.0× |
| `HdfBndry.get_bc_lines` | 2 | 12 ms | 13 ms | 1.0× |
| `HdfResultsPlan.get_runtime_data` | 1 | 2.8 ms | 3.0 ms | 1.0× |
| `HdfMesh.get_mesh_area_attributes` | 0 | 1.4 ms | 1.5 ms | 0.9× |
| `HdfResultsPlan.get_volume_accounting` | 1 | 1.1 ms | 1.2 ms | 0.9× |
| `HdfResultsMesh.get_boundary_conditions_timeseries` | 4 | 16 ms | 18 ms | 0.9× |
| `HdfBase.get_unsteady_timestamps` | 865 | 5.4 ms | 6.2 ms | 0.9× |
| `HdfResultsXsec.get_ref_lines_timeseries` | 11 | 13 ms | 19 ms | 0.7× |
| `HdfResultsXsec.get_xsec_timeseries` | 5 | 8.8 ms | 15 ms | 0.6× |

- Across all 50 method × file cases (geometric mean), the native engine is
  **1.17×** faster on the first call in a process and **1.41×** faster on
  repeat calls (0.1.1 on the same machine and run: 1.07× and 1.27×).
- Every case returned identical data in `h5py`, `auto` and `native` mode: the
  benchmark hashed the normalized result (values, dtypes, columns, index, CRS
  and geometry) of every run. There were 0 errors in 1,200 runs.
- Results with few rows can still be expensive: `get_mesh_cells_timeseries`
  returns one cell's series but must locate it in a large compressed dataset.

## Benchmark method

- **Harness:** each measurement runs in a fresh Python process. The mode is
  chosen only through `RAS_COMMANDER_FAST_HDF` (`h5py`, `auto`, `native`). The
  three modes are interleaved (h5py, auto, native, native, auto, h5py, ...) so
  drift affects them equally. Each process records the first call and two
  repeat calls. The 0.2.0 run interleaved, process by process, this release
  (engine 0.2.0 code, ras-commander 0.108.0 adapter) with 0.1.1 and the
  ras-commander it shipped with: 3 timed processes per mode and version per
  case, 1,200 processes in all, data and attribute hashes identical across
  every process of a case.
- **Cases:** 19 public methods on 7 HEC-RAS result files, from a 3,314-cell
  mesh to a mesh of about 340,000 cells and 705,000 faces.
- **Machine:** Windows 11, Intel Core i7-11700K (8 logical CPUs available to
  the VM), 48 GB RAM, Python 3.13, h5py 3.14 (HDF5 1.14.6), numpy 2.3, pandas
  2.3. No other heavy process ran during the 56-minute run: a quiet check
  before it, no build, test or HEC-RAS process before each case, and CPU
  monitored every 30 s (median 18% of the machine, about one core: the
  benchmark itself).
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
  (proving the layout and encoding of each dataset read). Since 0.2.0 a public
  call opens each HDF once and shares that session across its reads, which
  removed most of this cost; what remains is up to about 7 ms on repeat calls
  and 37 ms on the first call in a process (`get_cross_sections` on the
  largest cross-section model).
