# Performance

How fast ras-commander's public HDF methods are today, compared with the last
release before the native engine. The page leads with **the calls that take
noticeable time on large models**, where the engine and the vectorization
work matter; calls that finish in tens of milliseconds are listed separately
with their absolute times.

**What is compared.** *Before* is **ras-commander 0.106.1**, the last release
before the native engine and the vectorization work, installed without the
engine (pure h5py). *Now* is **ras-commander 0.110.1** with
**ras-commander-fast-hdf 0.2.3**, in its default `auto` mode and, for the
native-vs-h5py column, in h5py mode. Both used the same Python and libraries
(see [Benchmark method](#benchmark-method)). Measured on October 26, 2026.

## Large-project calls

**Threshold:** a call is listed here when it takes **100 ms or more** with
0.106.1 on the largest test model it was measured on. Times are repeat calls
(warm) on that model, medians of three fresh processes per column, ranked by
the 0.106.1 time.

![Bar chart, log time axis, repeat-call time on each call's largest test model, 0.106.1 against 0.110.1 in auto mode: fluvial-pluvial boundary 21,557 ms to 94.9 ms (227.1x, BaldEagleDamBrk.p18); cell polygons 20,592 ms to 460 ms (44.8x, JUL2020_D100RAS.p04); maximum face velocity 8,459 ms to 564 ms (15.0x); maximum water surface 2,733 ms to 238 ms (11.5x); cell points 2,459 ms to 139 ms (17.7x); mesh summary of maximum face velocity 2,094 ms to 172 ms (12.1x, BaldEagleDamBrk.p07); face-velocity time series 1,505 ms to 181 ms (8.3x); water-surface time series 1,426 ms to 192 ms (7.4x); one cell's water-surface time series 1,400 ms to 182 ms (7.7x).](images/large-project-calls-light.svg)

| Method | Largest test model (result rows) | 0.106.1 | 0.110.1 h5py | 0.110.1 `auto` | Speed-up vs 0.106.1 | Native vs h5py (0.110.1) |
|---|---|---:|---:|---:|---:|---:|
| `HdfFluvialPluvial.calculate_fluvial_pluvial_boundary` | BaldEagleDamBrk.p18, 4,425 cells (829 lines) | 21,557 ms | 447 ms | 94.9 ms | **227×** | 4.7× |
| `HdfMesh.get_mesh_cell_polygons` | JUL2020_D100RAS.p04 (340,246 cells) | 20,592 ms | 13,354 ms | 460 ms | **44.8×** | 29.0× |
| `HdfResultsMesh.get_mesh_max_face_v` | JUL2020_D100RAS.p04 (705,332 faces) | 8,459 ms | 595 ms | 564 ms | **15.0×** | 1.1× |
| `HdfResultsMesh.get_mesh_max_ws` | JUL2020_D100RAS.p04 (340,248 cells) | 2,733 ms | 247 ms | 238 ms | **11.5×** | 1.0× |
| `HdfMesh.get_mesh_cell_points` | JUL2020_D100RAS.p04 (352,141 cells) | 2,459 ms | 149 ms | 139 ms | **17.7×** | 1.1× |
| `HdfResultsMesh.get_mesh_summary` (maximum face velocity) | BaldEagleDamBrk.p07 (177,301 faces) | 2,094 ms | 177 ms | 172 ms | **12.1×** | 1.0× |
| `HdfResultsMesh.get_mesh_timeseries` (face velocity) | BaldEagleDamBrk.p07 (865 steps × 177,301 faces) | 1,505 ms | 1,453 ms | 181 ms | **8.3×** | 8.0× |
| `HdfResultsMesh.get_mesh_timeseries` (water surface) | BaldEagleDamBrk.p07 (865 steps × 87,039 cells) | 1,426 ms | 1,369 ms | 192 ms | **7.4×** | 7.1× |
| `HdfResultsMesh.get_mesh_cells_timeseries` (water surface, one cell) | BaldEagleDamBrk.p07 (87,039 cells) | 1,400 ms | 1,370 ms | 182 ms | **7.7×** | 7.5× |

- **Speed-up vs 0.106.1** is what you notice after upgrading: 0.106.1 time ÷
  0.110.1 `auto` time.
- **Native vs h5py** is 0.110.1 in h5py mode ÷ 0.110.1 in `auto` mode: the part of
  the gain that comes from the Rust engine. Where it is about 1.0× but the
  speed-up vs 0.106.1 is large (summaries, cell points), the gain is from
  vectorizing ras-commander itself, and h5py mode gets it too.
- **Fluvial-pluvial boundary on the 87,039-cell model was not measured** in
  this run: 0.106.1 did not finish one first call plus one repeat call within
  the harness's 1,800 s per-process limit, and the run was stopped there, so
  the 0.110.1 columns for that model were not reached either. The row above
  uses the largest model that completed.
- Other large-model rows of the same mesh show the same pattern; for example
  `get_mesh_cell_polygons` on BaldEagleDamBrk.p07 (87,039 cells) takes 5,190 ms
  with 0.106.1 and 153 ms with 0.110.1 (34×), and the water-surface time series
  on JUL2020_D100RAS.p04 takes 1,398 ms and 199 ms (7.0×).
- **First calls in a new process** (cold) show the same order of gains on large
  models, for example `get_mesh_max_face_v` on JUL2020_D100RAS.p04: 8,272 ms
  with 0.106.1, 500 ms with 0.110.1 `auto`.
- Every 0.110.1 case returned identical data in `h5py`, `auto` and `native`
  mode (each result hashed: values, dtypes, columns, index, CRS, geometry and
  attributes). 0.106.1 returns different output for `get_mesh_cell_polygons`,
  `calculate_fluvial_pluvial_boundary` and `get_structures` on
  BaldEagleDamBrk.p07 (later releases changed those results), so those
  comparisons are of the same call, not of identical output.

### Cell polygons by mesh size

![Bar chart, log time axis: HdfMesh.get_mesh_cell_polygons repeat-call time. 340,246 cells: 20,592 ms with 0.106.1, 13,354 ms with 0.110.1 in h5py mode, 460 ms with 0.110.1 in auto mode. 87,039 cells: 5,190, 3,242 and 153 ms. 4,425 cells: 296, 186 and 24.5 ms. 3,314 cells: 234, 134 and 16.9 ms.](images/mesh-polygons-light.svg)

## Small calls (under 100 ms)

These calls take tens of milliseconds or less even on the largest test
models, so there is little left to optimize: most of the time is opening the
file, resolving the CRS and building the pandas result, which is the same work
in every mode. A multiplier on a few milliseconds mostly shows noise, so only
absolute times are given (repeat calls, largest test model for each method).

| Method | Largest test model (result rows) | 0.106.1 | 0.110.1 h5py | 0.110.1 `auto` |
|---|---|---:|---:|---:|
| `HdfXsec.get_cross_sections` | JUL2020_D100RAS.p04 (1,769) | 97 ms | 87 ms | 83 ms |
| `HdfStruc.get_structures` | BaldEagleDamBrk.p07 (4) | 41 ms | 32 ms | 33 ms |
| `HdfResultsMesh.get_boundary_conditions_timeseries` | BaldEagleDamBrk.p18 (4) | 16 ms | 18 ms | 21 ms |
| `HdfBndry.get_bc_lines` | JUL2020_D100RAS.p04 (13) | 15 ms | 4.4 ms | 5.4 ms |
| `HdfResultsXsec.get_ref_lines_timeseries` | Homewood.p02 (11) | 13 ms | 15 ms | 71 ms |
| `HdfResultsXsec.get_xsec_timeseries` | OrleansEx.p02 (5) | 8.8 ms | 9.2 ms | 17 ms |
| `HdfBase.get_unsteady_timestamps` | BaldEagleDamBrk.p18 (865) | 7.0 ms | 6.5 ms | 7.0 ms |
| `HdfMesh.get_mesh_area_attributes` | BaldEagleDamBrk.p18 (0) | 2.8 ms | 1.4 ms | 1.5 ms |
| `HdfResultsPlan.get_runtime_data` | BaldEagle.p01 (1) | 2.7 ms | 2.8 ms | 3.2 ms |
| `HdfResultsPlan.get_volume_accounting` | BaldEagleDamBrk.p07 (1) | 1.5 ms | 1.2 ms | 1.3 ms |

The same methods on small models (for example `get_mesh_max_ws` on a
3,314-cell mesh: 58 ms with 0.106.1, 13 ms with 0.110.1) are also in this
range. Native small reads carry a fixed cost per read (proving each dataset's
layout and encoding), which shows on the first call in a process: for example
`get_runtime_data` on OrleansEx.p02 takes 6.3 ms with 0.106.1 and 35 ms with
0.110.1 `auto` on the first call. `get_ref_lines_timeseries` on Homewood.p02
is slower natively than in earlier measurements (71 ms against 19 ms in the
0.2.0 run); this is being investigated.

## Tables without geometry

ras-commander's per-cell and per-face result readers return a GeoDataFrame
by default, building a Shapely point or line for every row. With
`include_geometry=False` (ras-commander 0.109.0 and later) they return a plain
pandas DataFrame with the same rows, order, values, dtypes and `attrs`, keyed
by `mesh_name` and `cell_id` or `face_id` (point readers return `x` and `y`
columns). Use it when you join the values to geometry you already have, or
when you only need the numbers; it builds no geometry at all.

```python
from ras_commander import HdfResultsMesh

max_ws = HdfResultsMesh.get_mesh_max_ws(plan_hdf, include_geometry=False)
max_v = HdfResultsMesh.get_mesh_max_face_v(plan_hdf, include_geometry=False)
```

The table output was not part of this run (0.106.1 has no such option).

## Benchmark method

- **Harness:** each measurement runs in a fresh Python process, and the mode
  is chosen only through `RAS_COMMANDER_FAST_HDF` (`0` = h5py, `auto`,
  `native`). Each process records the first call and one or two repeat calls,
  and hashes the result. Per case there is one untimed first-touch process per
  column, then three rounds of timed processes; within each round the four
  columns (0.106.1 h5py, 0.110.1 h5py, `auto`, `native`) run in a rotated
  order, so drift affects them equally. 784 processes in all, 0 errors.
- **Cases:** 19 public methods on 7 HEC-RAS result files, from a 3,314-cell
  mesh to JUL2020_D100RAS.p04 (340,248 cells, 705,332 faces), the same 50
  method × file cases as the 0.2.0 release run. Three extra large-model cases
  (face lines, maximum depth and the face-velocity table on JUL2020_D100RAS.p04)
  were planned after the fluvial row and were not reached.
- **Software:** both variants on Python 3.13.9 with numpy 2.3.3, pandas 2.3.2,
  h5py 3.14.0 (HDF5 1.14.6), Shapely 2.1.2, GeoPandas 1.1.1, pyproj 3.7.2 and
  xarray 2026.4.0. 0.106.1 was installed from PyPI without
  ras-commander-fast-hdf; 0.110.1 and ras-commander-fast-hdf 0.2.3 were
  installed from PyPI (the 0.110.1 wheel's SHA-256 matched PyPI's).
- **Machine:** CLB01, Windows 11 Pro for Workstations, Intel Core i7-11700K
  (8 logical CPUs available to the VM), 48 GB RAM, on October 26, 2026,
  18:49–19:23 EDT for the completed cases. A quiet-machine check passed before
  the run (CPU 6% average), the controller refused to time while any build,
  test or HEC-RAS process ran (none did), and total CPU logged every 30 s had a
  median of 17% (about one core: the benchmark). One sample reached 65% during
  the multi-threaded cell-polygon cases, with no other process above 14% of
  one core.
- **Cache:** the operating system's file cache was warm for every column;
  "first call" means a fresh process, not a cold disk. Cold-disk reads were
  not measured.

## Reading the numbers

- **Large gains from the Rust engine** (native vs h5py well above 1×): work the
  engine does that ras-commander used to do in Python, namely assembling 2D
  cell polygons from face topology, selecting rows or cells from compressed
  time series without decompressing more than needed, and the graph and ring
  kernels of the native fluvial-pluvial analysis.
- **Large gains in every mode** (speed-up vs 0.106.1 large, native vs h5py about
  1×): vectorization in ras-commander itself, namely face lines and cell points
  built in one Shapely call, geometry joined to values by position,
  summary times converted once per distinct value. h5py mode gains as much as
  `auto` from these.
- **Why some calls are still about 1.0× native vs h5py:** the HDF read is a
  small share of the call (most of a summary with geometry is building the
  Shapely objects, which is the same in both modes), or the call is tiny (see
  [small calls](#small-calls-under-100-ms)).
- **Where the Python path still runs:** `auto` mode falls back to the h5py
  reference wherever the engine refuses a layout. For example, the native
  fluvial-pluvial analysis refuses meshes containing cells with invalid source
  geometry, and those models run at the Python reference's speed.
