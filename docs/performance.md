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
(see [Benchmark method](#benchmark-method)). Measured on October 26, 2026;
the five rows on Glaise Creek and BaldEagleDamBrk.p03 come from a separate
targeted run that evening with the same harness, software and settings.

## Large-project calls

**Threshold:** a call is listed here when it takes **100 ms or more** with
0.106.1 on the largest test model it was measured on. Times are repeat calls
(warm) on that model, medians of three fresh processes per column, ranked by
the 0.106.1 time.

![Bar chart, log time axis, repeat-call time on each call's largest test model, 0.106.1 against 0.110.1 in auto mode: fluvial-pluvial boundary 169,135 ms to 238 ms (711.0x, BaldEagleDamBrk.p03); on Glaise Creek: cell polygons 40,074 ms to 867 ms (46.2x), maximum face velocity 17,204 ms to 1,272 ms (13.5x), maximum water surface 4,574 ms to 478 ms (9.6x), cell points 4,249 ms to 261 ms (16.3x); on BaldEagleDamBrk.p07: mesh summary of maximum face velocity 2,094 ms to 172 ms (12.1x), face-velocity time series 1,505 ms to 181 ms (8.3x), water-surface time series 1,426 ms to 192 ms (7.4x), one cell's water-surface time series 1,400 ms to 182 ms (7.7x).](images/large-project-calls-light.svg)

| Method | Largest test model (result rows) | 0.106.1 | 0.110.1 h5py | 0.110.1 `auto` | Speed-up vs 0.106.1 | Native vs h5py (0.110.1) |
|---|---|---:|---:|---:|---:|---:|
| `HdfFluvialPluvial.calculate_fluvial_pluvial_boundary` | BaldEagleDamBrk.p03, 18,066 cells (1,877 lines)\* | 169,135 ms | 1,832 ms | 238 ms | **711×** | 7.7× |
| `HdfMesh.get_mesh_cell_polygons` | Glaise Creek (582,931 cells)\* | 40,074 ms | 25,168 ms | 867 ms | **46.2×** | 29.0× |
| `HdfResultsMesh.get_mesh_max_face_v` | Glaise Creek (1,393,564 faces)\* | 17,204 ms | 1,420 ms | 1,272 ms | **13.5×** | 1.1× |
| `HdfResultsMesh.get_mesh_max_ws` | Glaise Creek (582,934 cells)\* | 4,574 ms | 484 ms | 478 ms | **9.6×** | 1.0× |
| `HdfMesh.get_mesh_cell_points` | Glaise Creek (586,935 cells)\* | 4,249 ms | 287 ms | 261 ms | **16.3×** | 1.1× |
| `HdfResultsMesh.get_mesh_summary` (maximum face velocity) | BaldEagleDamBrk.p07 (177,301 faces) | 2,094 ms | 177 ms | 172 ms | **12.1×** | 1.0× |
| `HdfResultsMesh.get_mesh_timeseries` (face velocity) | BaldEagleDamBrk.p07 (865 steps × 177,301 faces) | 1,505 ms | 1,453 ms | 181 ms | **8.3×** | 8.0× |
| `HdfResultsMesh.get_mesh_timeseries` (water surface) | BaldEagleDamBrk.p07 (865 steps × 87,039 cells) | 1,426 ms | 1,369 ms | 192 ms | **7.4×** | 7.1× |
| `HdfResultsMesh.get_mesh_cells_timeseries` (water surface, one cell) | BaldEagleDamBrk.p07 (87,039 cells) | 1,400 ms | 1,370 ms | 182 ms | **7.7×** | 7.5× |

\* Measured in a separate targeted run on October 26, 2026 (21:44–22:24 EDT)
with the same harness, software and settings; see
[Benchmark method](#benchmark-method). Glaise Creek is the Glaise
Creek–White River model (1101001306.p01, HEC-RAS 6.3.1) from FEMA's estimated
Base Flood Elevation (eBFE) program; BaldEagleDamBrk.p03 is the single 2D
area plan of HEC's BaldEagleCrkMulti2D example (HEC-RAS 7.0.1 results).

- **Speed-up vs 0.106.1** is what you notice after upgrading: 0.106.1 time ÷
  0.110.1 `auto` time.
- **Native vs h5py** is 0.110.1 in h5py mode ÷ 0.110.1 in `auto` mode: the part of
  the gain that comes from the Rust engine. Where it is about 1.0× but the
  speed-up vs 0.106.1 is large (summaries, cell points), the gain is from
  vectorizing ras-commander itself, and h5py mode gets it too.
- Other large models show the same pattern; for example
  `get_mesh_cell_polygons` on BaldEagleDamBrk.p07 (87,039 cells) takes
  5,190 ms with 0.106.1 and 153 ms with 0.110.1 (34×), `get_mesh_max_face_v`
  on the same model 2,090 ms and 175 ms (12×), and the fluvial-pluvial boundary
  on BaldEagleDamBrk.p18 (4,425 cells) 21,557 ms and 94.9 ms (227×).
- **First calls in a new process** (cold) show the same order of gains on large
  models, for example `get_mesh_max_face_v` on Glaise Creek: 17,052 ms with
  0.106.1, 1,127 ms with 0.110.1 `auto`.
- Every 0.110.1 case returned identical data in every mode run (each
  result hashed: values, dtypes, columns, index, CRS, geometry and
  attributes). 0.106.1 returns different output for `get_mesh_cell_polygons`
  and `calculate_fluvial_pluvial_boundary` on every model here, and for
  `get_structures` on BaldEagleDamBrk.p07 (later releases changed those
  results), so those comparisons are of the same call, not of identical
  output.

### Cell polygons by mesh size

![Bar chart, log time axis: HdfMesh.get_mesh_cell_polygons repeat-call time. 582,931 cells (Glaise Creek): 40,074 ms with 0.106.1, 25,168 ms with 0.110.1 in h5py mode, 867 ms with 0.110.1 in auto mode. 87,039 cells: 5,190, 3,242 and 153 ms. 4,425 cells: 296, 186 and 24.5 ms. 3,314 cells: 234, 134 and 16.9 ms.](images/mesh-polygons-light.svg)

## Small calls (under 100 ms)

These calls take tens of milliseconds or less even on the largest test
models, so there is little left to optimize: most of the time is opening the
file, resolving the CRS and building the pandas result, which is the same work
in every mode. A multiplier on a few milliseconds mostly shows noise, so only
absolute times are given (repeat calls, largest test model for each method).

| Method | Largest test model (result rows) | 0.106.1 | 0.110.1 h5py | 0.110.1 `auto` |
|---|---|---:|---:|---:|
| `HdfStruc.get_structures` | BaldEagleDamBrk.p07 (4) | 41 ms | 32 ms | 33 ms |
| `HdfXsec.get_cross_sections` | OrleansEx.p02 (335) | 17 ms | 18 ms | 20 ms |
| `HdfResultsMesh.get_boundary_conditions_timeseries` | BaldEagleDamBrk.p18 (4) | 16 ms | 18 ms | 21 ms |
| `HdfBndry.get_bc_lines` | AustinOyster.p01 (2) | 15 ms | 4.9 ms | 5.5 ms |
| `HdfBase.get_unsteady_timestamps` | BaldEagleDamBrk.p18 (865) | 7.0 ms | 6.5 ms | 7.0 ms |
| `HdfMesh.get_mesh_area_attributes` | BaldEagleDamBrk.p18 (0) | 2.8 ms | 1.4 ms | 1.5 ms |
| `HdfResultsPlan.get_runtime_data` | BaldEagle.p01 (1) | 2.7 ms | 2.8 ms | 3.2 ms |
| `HdfResultsPlan.get_volume_accounting` | BaldEagleDamBrk.p07 (1) | 1.5 ms | 1.2 ms | 1.3 ms |

The same methods on small models (for example `get_mesh_max_ws` on a
3,314-cell mesh: 58 ms with 0.106.1, 13 ms with 0.110.1) are also in this
range.

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
  order, so drift affects them equally. 624 processes in the main run, 0
  errors.
- **Targeted run:** the five rows marked \* (Glaise Creek and
  BaldEagleDamBrk.p03) were timed later the same evening with a copy of the
  same harness and the same virtual environments, in three columns (0.106.1
  h5py, 0.110.1 h5py, `auto`): 60 processes, 0 errors.
- **Cases:** 19 public methods on 8 public HEC-RAS result files, from a
  3,314-cell mesh to Glaise Creek (582,934 cells, 1,393,564 faces): 44
  method × file cases.
- **Software:** both variants on Python 3.13.9 with numpy 2.3.3, pandas 2.3.2,
  h5py 3.14.0 (HDF5 1.14.6), Shapely 2.1.2, GeoPandas 1.1.1, pyproj 3.7.2 and
  xarray 2026.4.0. 0.106.1 was installed from PyPI without
  ras-commander-fast-hdf; 0.110.1 and ras-commander-fast-hdf 0.2.3 were
  installed from PyPI (the 0.110.1 wheel's SHA-256 matched PyPI's).
- **Machine:** CLB01, Windows 11 Pro for Workstations, Intel Core i7-11700K
  (8 logical CPUs available to the VM), 48 GB RAM, on October 26, 2026,
  18:49–19:23 EDT (targeted run 21:44–22:24 EDT). A quiet-machine check passed before
  the run (CPU 6% average), the controller refused to time while any build,
  test or HEC-RAS process ran (none did), and total CPU logged every 30 s had a
  median of 17% (about one core: the benchmark). One sample reached 65% during
  the multi-threaded cell-polygon cases, with no other process above 14% of
  one core. Before the targeted run the quiet check passed at 8% average CPU,
  and its CPU log had a median of 18%; another process's build ran during
  its first two untimed first-touch processes and had finished before the
  first timed process started.
- **Cache:** the operating system's file cache was warm for every column;
  "first call" means a fresh process, not a cold disk.

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
