# Performance

How fast ras-commander's public HDF methods are with the native engine, and
where the time goes. The page puts **calls that take noticeable time on large
models** first, because that is where the engine matters; calls that finish in
a few tens of milliseconds are listed separately, with their absolute times.

!!! note "Interim page (October 26, 2026)"
    ras-commander 0.110.1 (released October 26, 2026) removed most of the
    Python overhead that kept whole-array summaries, cell points and the
    native fluvial-pluvial path near 1.0×. A full re-run of the release
    benchmark against the pre-engine release (ras-commander 0.106.1) is in
    progress; this page will be replaced with its results. Until then every
    number below is from one of two earlier measured runs, named in each table.
    Rows whose method 0.110.1 changed say so instead of showing an old number.

## Large-project calls

**Threshold:** a call is listed here when it takes **100 ms or more** (warm,
h5py) on the largest model it was measured on. Times are repeat calls (warm),
medians of three fresh processes.

| Method | Largest model measured (rows) | h5py | Native (`auto`) | Speed-up | Measured in |
|---|---|---:|---:|---:|---|
| `HdfMesh.get_mesh_cell_polygons` | JUL2020_D100RAS.p04 (340,246 cells) | 17,252 ms | 960 ms | 18.0× | 0.2.0 run |
| `HdfResultsMesh.get_mesh_cells_timeseries` (water surface, one cell) | BaldEagleDamBrk.p07 (87,039 cells) | 1,405 ms | 173 ms | 8.1× | 0.2.0 run |
| `HdfResultsMesh.get_mesh_timeseries` (water surface) | BaldEagleDamBrk.p07 (865 steps × 87,039 cells) | 1,433 ms | 193 ms | 7.4× | 0.2.0 run |
| `HdfResultsMesh.get_mesh_timeseries` (face velocity) | BaldEagleDamBrk.p07 (865 steps × 177,301 faces) | 1,516 ms | 253 ms | 6.0× | 0.2.0 run |
| `HdfResultsMesh.get_mesh_max_face_v` (with face lines) | BaldEagleDamBrk.p07 (177,301 faces) | 2,085 ms → 304 ms | 2,106 ms → 297 ms | 7.1× | 0.2.3 run (before → after) |
| `HdfResultsMesh.get_mesh_max_face_v` (with face lines) | Homewood.p02 (161,683 faces) | 1,967 ms → 273 ms | 1,977 ms → 267 ms | 7.4× | 0.2.3 run (before → after) |
| `HdfFluvialPluvial.calculate_fluvial_pluvial_boundary` | BaldEagleDamBrk.p07 (87,039 cells) | 10,638 ms | changed in 0.110.1: pending re-run | – | 0.2.0 run (h5py) |
| `HdfResultsMesh.get_mesh_max_ws`, `get_mesh_summary`, `get_mesh_max_depth`, `HdfMesh.get_mesh_cell_points`, `get_mesh_cell_faces` | – | changed in 0.110.1: pending re-run | | | |

- **Speed-up** for the 0.2.0 rows is h5py ÷ native in the same release
  (ras-commander 0.108.0 with engine 0.2.0). For the `get_mesh_max_face_v`
  rows it is **before ÷ after in `auto` mode**: before is ras-commander `main`
  9817f9484 with engine 0.2.2, after is ras-commander 0.109.0 with engine 0.2.3.
  In that change h5py mode gained as much as `auto` (2,085 → 304 ms), because
  the gain is in building the face lines in one vectorized call, not in the
  read; in the same release native and h5py are within 3%.
- The earlier headline rows that showed about 1.0× for `get_mesh_max_face_v`
  (705,332 faces) and `get_mesh_summary` (maximum face velocity) are removed:
  both calls build the same face lines and are now about 7× faster than those
  rows showed. Their new times come with the re-run.
- **Fluvial-pluvial boundary:** 0.2.0 measured 6,856 ms natively. ras-commander
  0.110.1 builds the native snapshot with NumPy instead of per-face Python
  loops; its measured time will replace this row.

![Bar chart: HdfMesh.get_mesh_cell_polygons takes 17.3 s with h5py and 0.90 s natively on a 340,246-cell mesh, 4.32 s and 0.26 s on 87,039 cells, 0.28 s and 0.06 s on 4,425 cells, 0.23 s and 0.06 s on 3,314 cells.](images/mesh-polygons-light.svg)

*Cell polygons by mesh size, 0.2.0 run.*

## Small calls (under 100 ms)

These calls take tens of milliseconds or less even on the largest test
models. Most of that is opening the file, resolving the CRS and building the
pandas result, which are the same in both modes, so there is little for the
engine to win and a multiplier would only show noise. Absolute times, 0.2.0
run, repeat calls on the heaviest test file for each method:

| Method | File (rows) | h5py | Native (`auto`) |
|---|---|---:|---:|
| `HdfXsec.get_cross_sections` | JUL2020_D100RAS.p04 (1,769) | 82 ms | 79 ms |
| `HdfStruc.get_structures` | JUL2020_D100RAS.p04 (835) | 36 ms | 34 ms |
| `HdfResultsMesh.get_boundary_conditions_timeseries` | BaldEagleDamBrk.p07 (4) | 16 ms | 18 ms |
| `HdfResultsXsec.get_ref_lines_timeseries` | Homewood.p02 (11) | 13 ms | 19 ms |
| `HdfBndry.get_bc_lines` | AustinOyster.p01 (2) | 12 ms | 13 ms |
| `HdfResultsXsec.get_xsec_timeseries` | OrleansEx.p02 (5) | 8.8 ms | 15 ms |
| `HdfBase.get_unsteady_timestamps` | BaldEagleDamBrk.p07 (865) | 5.4 ms | 6.2 ms |
| `HdfResultsPlan.get_runtime_data` | BaldEagle.p01 (1) | 2.8 ms | 3.0 ms |
| `HdfMesh.get_mesh_area_attributes` | BaldEagleDamBrk.p18 (0) | 1.4 ms | 1.5 ms |
| `HdfResultsPlan.get_volume_accounting` | BaldEagleDamBrk.p07 (1) | 1.1 ms | 1.2 ms |

Native small reads carry a fixed cost per read (proving each dataset's layout
and encoding): at most about 7 ms on repeat calls and 37 ms on the first call
in a process (`get_cross_sections` on the largest cross-section model). Every
case returned identical data in `h5py`, `auto` and `native` mode.

## Tables without geometry

ras-commander's per-cell and per-face result readers return a GeoDataFrame
by default, building a Shapely point or line for every row. With
`include_geometry=False` they return a plain pandas DataFrame with the same
rows, order, values, dtypes and `attrs`, keyed by `mesh_name` and `cell_id` or
`face_id` (point readers return `x` and `y` columns). Use it when you join the
values to geometry you already have, or when you only need the numbers. The
option is available from ras-commander 0.109.0.

```python
from ras_commander import HdfResultsMesh

max_ws = HdfResultsMesh.get_mesh_max_ws(plan_hdf, include_geometry=False)
max_v = HdfResultsMesh.get_mesh_max_face_v(plan_hdf, include_geometry=False)
```

Default call against `include_geometry=False` on the same reader, in the
default `auto` mode: median of 7 repeat calls, and the peak memory of one call.

| Reader | File (rows) | Default | Table | Speed-up | Peak memory |
|---|---|---:|---:|---:|---:|
| `get_mesh_max_face_v` | BaldEagleDamBrk.p07 (177,301 faces) | 1,642 ms | 34 ms | 48× | 37.0 → 14.2 MiB |
| `get_mesh_max_face_v` | Homewood.p02 (161,683 faces) | 1,709 ms | 35 ms | 50× | 34.5 → 13.0 MiB |
| `get_mesh_max_depth` | Homewood.p02 (68,743 cells) | 473 ms | 24 ms | 20× | 10.0 → 4.7 MiB |
| `get_mesh_max_iter` | BaldEagleDamBrk.p07 (87,039 cells) | 104 ms | 17 ms | 6.1× | 17.7 → 5.7 MiB |
| `get_mesh_cell_points` | BaldEagleDamBrk.p07 (89,879 cells) | 60 ms | 12 ms | 5.2× | 16.7 → 6.9 MiB |
| `get_mesh_max_ws` | BaldEagleDamBrk.p07 (87,039 cells) | 114 ms | 25 ms | 4.6× | 18.4 → 7.2 MiB |
| `get_mesh_max_depth` | BaldEagleDamBrk.p07 (87,039 cells) | 770 ms | 252 ms | 3.0× | 13.0 → 6.0 MiB |
| `get_cell_bed_change` (sediment) | Weise.p01 (2,702 cells) | 175 ms | 158 ms | 1.1× | 0.69 → 0.64 MiB |

- **Face readers gain most:** the default builds a Shapely line for every face.
- **Some readers gain less:** `get_mesh_max_depth` on BaldEagleDamBrk.p07 is
  dominated by the reduction over time steps, and the sediment readers by
  their time-series read.
- **Results are identical:** the table matches the default call with its
  geometry column removed, in `h5py`, `auto` and `native` mode, on all 137
  test files.
- **Measured** on the same workstation in one process per case. This is a
  single run, so treat small differences as noise.
- **Default timings are before 0.2.3.** From engine 0.2.3 and ras-commander
  0.109.0 the default output builds each mesh's face lines in one vectorized
  call (`HdfSession.read_mesh_face_lines`); see
  [Face readers from 0.2.3](#face-readers-from-023).

### Face readers from 0.2.3

`HdfResultsMesh.get_mesh_max_face_v` before and after the vectorized face
lines, measured on a quiet Windows workstation in October 2026. **Before**:
ras-commander `main` 9817f9484 with engine 0.2.2. **After**: the published
ras-commander 0.109.0 with engine 0.2.3. Both sides used NumPy 2.3.3,
pandas 2.3.2, Shapely 2.1.2 and h5py 3.14.0. Repeat calls (warm) are the
median of 6 calls in 3 fresh processes per mode, interleaved. Results were
identical before and after, in every mode. This is a separate run from the
table above, so its absolute times are not comparable with it.

| Output | File (faces) | Before h5py | Before `auto` | Before `native` | After h5py | After `auto` | After `native` | `auto` speed-up |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| Default (GeoDataFrame) | BaldEagleDamBrk.p18 (9,581) | 138 ms | 140 ms | 140 ms | 41 ms | 44 ms | 45 ms | 3.2× |
| Default (GeoDataFrame) | BaldEagleDamBrk.p07 (177,301) | 2,085 ms | 2,106 ms | 2,082 ms | 304 ms | 297 ms | 304 ms | 7.1× |
| Default (GeoDataFrame) | Homewood.p02 (161,683) | 1,967 ms | 1,977 ms | 1,922 ms | 273 ms | 267 ms | 259 ms | 7.4× |
| Table | BaldEagleDamBrk.p18 (9,581) | 9.1 ms | 11.6 ms | 10.6 ms | 9.7 ms | 10.6 ms | 11.8 ms | 1.1× |
| Table | BaldEagleDamBrk.p07 (177,301) | 78 ms | 79 ms | 79 ms | 79 ms | 79 ms | 81 ms | 1.0× |
| Table | Homewood.p02 (161,683) | 70 ms | 69 ms | 70 ms | 70 ms | 72 ms | 71 ms | 1.0× |

The default output gains in all three modes, because the gain is in building
the lines, not in reading the HDF. The table output is unchanged.

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

- **Large gains from Rust** come from work the engine does that ras-commander
  used to do in Python: assembling 2D cell polygons from face topology, and
  selecting rows or cells from compressed time-series chunks without
  decompressing more than needed. These gains appear only in `auto`/`native`
  mode.
- **Large gains in every mode** come from vectorizing ras-commander itself:
  face lines built in one Shapely call (0.109.0) and, in 0.110.1, positional
  geometry joins, per-distinct-value time conversion and a NumPy-built native
  fluvial snapshot. h5py mode gains as much as `auto` from these, so a
  native-vs-h5py ratio near 1.0× on such a call does not mean it did not get
  faster.
- **About 1.0× native vs h5py** remains where the HDF read is a small share of
  the call (most of a whole-array summary with geometry is building the
  Shapely objects) or where the call is tiny (see
  [small calls](#small-calls-under-100-ms)).
