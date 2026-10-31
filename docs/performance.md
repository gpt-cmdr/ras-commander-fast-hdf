# Performance

How fast ras-commander's public HDF methods are today, compared with the last
release before the native engine. The page leads with **the calls that take
noticeable time on large models**, where the engine and the vectorization
work matter; calls that finish in tens of milliseconds are listed separately
with their absolute times.

**What is compared.** *Before* is **ras-commander 0.106.1**, the last release
before the native engine and the vectorization work, installed without the
engine (pure h5py). *Now* is **ras-commander 0.112.0** with
**ras-commander-fast-hdf 0.2.5**, measured as their release candidates
(ras-commander `29a2085ef`, the engine wheel built from `4013987` that the
release gate qualified), in the default `auto` mode and, for the
native-vs-h5py column, in h5py mode. A fifth column runs the same
ras-commander with the published **ras-commander-fast-hdf 0.2.4** in native
mode, to compare the engines alone ([0.2.5 against 0.2.4](#engine-025-against-024)).
All columns used the same Python and libraries (see
[Benchmark method](#benchmark-method)). Measured on October 30, 2026.

## Large-project calls

**Threshold:** a call is listed here when it takes **100 ms or more** with
0.106.1 on the largest test model it was measured on. Times are repeat calls
(warm) on that model, medians of three fresh processes per column, ranked by
the 0.106.1 time.

![Bar chart, log time axis, repeat-call time on each call's largest test model, 0.106.1 against 0.112.0 in auto mode: fluvial-pluvial boundary 164,253 ms to 238 ms (689.9x, BaldEagleDamBrk.p03); on Glaise Creek: cell polygons 37,853 ms to 906 ms (41.8x), maximum face velocity 16,270 ms to 1,242 ms (13.1x); cell faces 7,713 ms to 403 ms (19.2x, JUL2020_D100RAS.p04); on Glaise Creek: maximum water surface 4,509 ms to 440 ms (10.3x), cell points 4,086 ms to 252 ms (16.2x); maximum depth 3,258 ms to 262 ms (12.5x, JUL2020_D100RAS.p04); on BaldEagleDamBrk.p07: mesh summary of maximum face velocity 2,209 ms to 177 ms (12.5x), face-velocity time series 1,521 ms to 187 ms (8.1x), water-surface time series 1,464 ms to 210 ms (7.0x), one cell's water-surface time series 1,348 ms to 188 ms (7.2x); cross sections 100 ms to 86.2 ms (1.2x, JUL2020_D100RAS.p04).](images/large-project-calls-light.svg)

| Method | Largest test model (result rows) | 0.106.1 | 0.112.0 h5py | 0.112.0 `auto` | Speed-up vs 0.106.1 | Native vs h5py (0.112.0) |
|---|---|---:|---:|---:|---:|---:|
| `HdfFluvialPluvial.calculate_fluvial_pluvial_boundary` | BaldEagleDamBrk.p03, 18,066 cells (1,877 lines) | 164,253 ms | 1,765 ms | 238 ms | **690×** | 7.4× |
| `HdfMesh.get_mesh_cell_polygons` | Glaise Creek (582,931 cells) | 37,853 ms | 23,451 ms | 906 ms | **41.8×** | 25.9× |
| `HdfResultsMesh.get_mesh_max_face_v` | Glaise Creek (1,393,564 faces) | 16,270 ms | 1,364 ms | 1,242 ms | **13.1×** | 1.1× |
| `HdfMesh.get_mesh_cell_faces` | JUL2020_D100RAS.p04 (705,332 faces) | 7,713 ms | 446 ms | 403 ms | **19.2×** | 1.1× |
| `HdfResultsMesh.get_mesh_max_ws` | Glaise Creek (582,934 cells) | 4,509 ms | 481 ms | 440 ms | **10.3×** | 1.1× |
| `HdfMesh.get_mesh_cell_points` | Glaise Creek (586,935 cells) | 4,086 ms | 281 ms | 252 ms | **16.2×** | 1.1× |
| `HdfResultsMesh.get_mesh_max_depth` | JUL2020_D100RAS.p04 (340,248 cells) | 3,258 ms | 643 ms | 262 ms | **12.5×** | 2.5× |
| `HdfResultsMesh.get_mesh_summary` (maximum face velocity) | BaldEagleDamBrk.p07 (177,301 faces) | 2,209 ms | 204 ms | 177 ms | **12.5×** | 1.2× |
| `HdfResultsMesh.get_mesh_timeseries` (face velocity) | BaldEagleDamBrk.p07 (865 steps × 177,301 faces) | 1,521 ms | 1,441 ms | 187 ms | **8.1×** | 7.7× |
| `HdfResultsMesh.get_mesh_timeseries` (water surface) | BaldEagleDamBrk.p07 (865 steps × 87,039 cells) | 1,464 ms | 1,372 ms | 210 ms | **7.0×** | 6.5× |
| `HdfResultsMesh.get_mesh_cells_timeseries` (water surface, one cell) | BaldEagleDamBrk.p07 (87,039 cells) | 1,348 ms | 1,371 ms | 188 ms | **7.2×** | 7.3× |
| `HdfXsec.get_cross_sections` | JUL2020_D100RAS.p04 (1,769) | 100 ms | 88.5 ms | 86.2 ms | **1.2×** | 1.0× |

Glaise Creek is the Glaise Creek–White River model (1101001306.p01, HEC-RAS
6.3.1) from FEMA's estimated Base Flood Elevation (eBFE) program;
BaldEagleDamBrk.p03 is the single 2D area plan of HEC's BaldEagleCrkMulti2D
example (HEC-RAS 7.0.1 results).

- **Speed-up vs 0.106.1** is what you notice after upgrading: 0.106.1 time ÷
  0.112.0 `auto` time.
- **Native vs h5py** is 0.112.0 in h5py mode ÷ 0.112.0 in `auto` mode: the part of
  the gain that comes from the Rust engine. Where it is about 1.0× but the
  speed-up vs 0.106.1 is large (summaries, cell points), the gain is from
  vectorizing ras-commander itself, and h5py mode gets it too.
- Other large models show the same pattern; for example
  `get_mesh_cell_polygons` on BaldEagleDamBrk.p07 (87,039 cells) takes
  5,050 ms with 0.106.1 and 153 ms with 0.112.0 (33×), `get_mesh_max_face_v`
  on the same model 2,059 ms and 176 ms (11.7×), and the fluvial-pluvial
  boundary on BaldEagleDamBrk.p18 (4,425 cells) 21,973 ms and 88.7 ms (248×).
- **The fluvial-pluvial boundary on BaldEagleDamBrk.p07** (87,039 cells, 4,569
  boundary lines) did not finish with 0.106.1 within the harness's 1,800 s
  process limit on any of five attempts, so it has no 0.106.1 time. 0.112.0 takes
  8,705 ms in h5py mode and 927 ms in `auto` mode.
- **First calls in a new process** (cold) show the same order of gains on large
  models, for example `get_mesh_max_face_v` on Glaise Creek: 16,078 ms with
  0.106.1, 1,109 ms with 0.112.0 `auto`.
- Every 0.112.0 case returned identical data in every mode run, and identical
  to ras-commander-fast-hdf 0.2.4 (each result hashed: values, dtypes, columns,
  index, CRS, geometry and attributes). 0.106.1 returns different output for
  `get_mesh_cell_polygons` and `calculate_fluvial_pluvial_boundary` on every
  model here, and for `get_structures` on BaldEagleDamBrk.p07 (later releases
  changed those results), so those comparisons are of the same call, not of
  identical output.

### Cell polygons by mesh size

![Bar chart, log time axis: HdfMesh.get_mesh_cell_polygons repeat-call time. 582,931 cells (Glaise Creek): 37,853 ms with 0.106.1, 23,451 ms with 0.112.0 in h5py mode, 906 ms with 0.112.0 in auto mode. 340,246 cells (JUL2020_D100RAS.p04): 20,923, 13,357 and 463 ms. 87,039 cells: 5,050, 3,219 and 153 ms. 4,425 cells: 291, 179 and 24.9 ms. 3,314 cells: 231, 134 and 17.3 ms.](images/mesh-polygons-light.svg)

## Small calls (under 100 ms)

These calls take tens of milliseconds or less even on the largest test
models, so there is little left to optimize: most of the time is opening the
file, resolving the CRS and building the pandas result, which is the same work
in every mode. A multiplier on a few milliseconds mostly shows noise, so only
absolute times are given (repeat calls, largest test model for each method).

| Method | Largest test model (result rows) | 0.106.1 | 0.112.0 h5py | 0.112.0 `auto` |
|---|---|---:|---:|---:|
| `HdfStruc.get_structures` | BaldEagleDamBrk.p07 (4) | 39 ms | 29 ms | 32 ms |
| `HdfResultsMesh.get_boundary_conditions_timeseries` | BaldEagleDamBrk.p18 (4) | 16 ms | 18 ms | 22 ms |
| `HdfBndry.get_bc_lines` | AustinOyster.p01 (2) | 16 ms | 4.1 ms | 5.5 ms |
| `HdfResultsXsec.get_ref_lines_timeseries` | Homewood.p02 (11) | 14 ms | 14 ms | 21 ms |
| `HdfResultsXsec.get_xsec_timeseries` | OrleansEx.p02 (5) | 8.6 ms | 8.6 ms | 15 ms |
| `HdfBase.get_unsteady_timestamps` | BaldEagleDamBrk.p18 (865) | 6.6 ms | 6.7 ms | 7.0 ms |
| `HdfResultsPlan.get_runtime_data` | BaldEagle.p01 (1) | 2.6 ms | 2.9 ms | 3.9 ms |
| `HdfMesh.get_mesh_area_attributes` | BaldEagleDamBrk.p18 (0) | 2.4 ms | 1.3 ms | 1.5 ms |
| `HdfResultsPlan.get_volume_accounting` | BaldEagle.p01 (1) | 1.2 ms | 1.2 ms | 1.3 ms |

The same methods on small models (for example `get_mesh_max_ws` on a
3,314-cell mesh: 60 ms with 0.106.1, 12 ms with 0.112.0) are also in this
range.

## Engine 0.2.5 against 0.2.4

The fifth column runs the same ras-commander (`29a2085ef`) with the published
ras-commander-fast-hdf 0.2.4 wheel in native mode, so it differs from the
0.112.0 native column only in the engine. Over all 58 method × file cases the
geometric mean of the repeat-call ratios (0.2.5 ÷ 0.2.4) is **0.98**, and
every case returned identical data with both engines. 0.2.5 adds an in-flight
decode budget to every read; it costs about 0.1 µs per read, which no public
call here shows.

Two cases came out slower beyond the three-process spread: `get_mesh_max_ws`
on AustinOyster.p01 (12.6 against 11.8 ms) and `get_mesh_cells_timeseries`
on BaldEagleDamBrk.p07 (231 against 183 ms). Timed again in eight alternating
rounds of fresh processes, both are within noise: 12.1 against 12.2 ms (0.2.5
slower in 4 of 8 rounds) and 185 against 182 ms (5 of 8).

| Method | Model | 0.2.4 native | 0.2.5 native |
|---|---|---:|---:|
| `calculate_fluvial_pluvial_boundary` | BaldEagleDamBrk.p03 | 230 ms | 230 ms |
| `calculate_fluvial_pluvial_boundary` | BaldEagleDamBrk.p07 | 916 ms | 912 ms |
| `get_mesh_cell_polygons` | Glaise Creek | 832 ms | 874 ms |
| `get_mesh_max_face_v` | Glaise Creek | 1,234 ms | 1,233 ms |
| `get_mesh_max_ws` | Glaise Creek | 441 ms | 443 ms |
| `get_mesh_cell_faces` | JUL2020_D100RAS.p04 | 399 ms | 395 ms |
| `get_mesh_max_depth` | JUL2020_D100RAS.p04 | 279 ms | 265 ms |
| `get_mesh_timeseries` (water surface) | BaldEagleDamBrk.p07 | 185 ms | 198 ms |
| `get_mesh_timeseries` (face velocity) | BaldEagleDamBrk.p07 | 208 ms | 191 ms |

(Repeat calls, medians of three processes. `get_mesh_cell_polygons` on Glaise
Creek ranged 745–918 ms with 0.2.4 and 872–910 ms with 0.2.5.)

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

For example, `get_mesh_max_face_v` on JUL2020_D100RAS.p04 (705,332 faces)
takes 78 ms with `include_geometry=False` in `auto` mode, against 560 ms with
geometry. 0.106.1 has no such option, so the table output has no 0.106.1 time.

## Benchmark method

- **Harness:** each measurement runs in a fresh Python process, and the mode
  is chosen only through `RAS_COMMANDER_FAST_HDF` (`0` = h5py, `auto`,
  `native`). Each process records the first call and one or two repeat calls,
  and hashes the result. Per case there is one untimed first-touch process per
  column, then three rounds of timed processes; within each round the five
  columns (0.106.1 h5py, 0.112.0 h5py, `auto` and `native`, and 0.2.4
  `native`) run in a rotated order, so drift affects them equally. 864 timed
  and 288 first-touch processes completed; the one error is the 0.106.1
  fluvial-pluvial case on BaldEagleDamBrk.p07, which timed out (1,800 s); after
  five timeouts its remaining 0.106.1 processes were skipped.
- **Cases:** 20 public methods on 9 public HEC-RAS result files, from a
  3,314-cell mesh to Glaise Creek (582,934 cells, 1,393,564 faces): 58
  method × file cases.
- **Software:** every column on Python 3.13.9 with numpy 2.3.3, pandas 2.3.2,
  h5py 3.14.0 (HDF5 1.14.6), Shapely 2.1.2, GeoPandas 1.1.1, pyproj 3.7.2 and
  xarray 2026.4.0. 0.106.1 was installed from PyPI without
  ras-commander-fast-hdf. 0.112.0 is ras-commander `29a2085ef` (its wheel
  built from that commit) with the ras-commander-fast-hdf wheel built from
  `4013987` (SHA-256 `8184c59c…52541`), the files the final release gate
  qualified. 0.112.0 also contains ras-commander #750 (HEC-RAS 7.0.2
  single-plan execution and terrain export), merged after this run; it
  changes none of the methods measured here. The 0.2.4 column uses the PyPI wheel (SHA-256 `a625348e…a968`,
  matching PyPI).
- **Machine:** CLB01, Windows 11 Pro for Workstations, Intel Core i7-11700K
  (8 logical CPUs available to the VM), 48 GB RAM, on October 30, 2026,
  10:02–14:47 EDT. A quiet-machine check passed before each segment (CPU 7%,
  6% and 7% average). The controller refused to start a case while any build,
  test or HEC-RAS process ran. Total CPU, logged every 30 s, had a median of
  19% (about one core: the benchmark) and a 90th percentile of 34%.
- **Load during timed processes:** HEC-RAS runs and Rust builds of other work
  started during 71 timed processes in six cases; those six cases were timed
  again at 14:43–14:47 with nothing else running, and those times are the
  ones shown. Windows Defender (MsMpEng) used 50–100% of one core during 103
  timed processes in ten cases; every column of those cases ran in the same
  rounds.
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
