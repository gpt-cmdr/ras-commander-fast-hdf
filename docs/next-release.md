# Next HDF release: task list

Targets: **ras-commander-fast-hdf 0.2.5 (or 0.3.0 if semver requires)** and **ras-commander 0.112.0**.
Written 2026-10-29. Status: `todo` / `in progress` / `done` / `blocked`.

Rules for every task:
- **Exact or refuse.** A read either matches h5py exactly or is refused with a typed error.
- **Prove it on real HEC-RAS files** before claiming coverage.
- **Gate before merge.** Engine changes run the release gate and the equivalence suites in h5py, auto and native, compared case by case.
- **No releases without the user's approval.**

## A. Robustness (first)

| # | Task | Repo | Owner | Status |
|---|---|---|---|---|
| A1 | Require `ras-commander-fast-hdf>=0.2.4` (it carries the `get_ref_lines_timeseries` / `get_xsec_timeseries` regression fixes) | ras-commander | robustness agent | todo |
| A2 | Invalid-cell policy for `map_horizontal_profiles`, `map_whole_simulation`, `compare_with_rasmapper`; native `generate_fluvial_pluvial_polygons` on meshes with invalid cells (identical to the reference or narrowly refused) | ras-commander | robustness agent | todo |
| A3 | Terrain-CRS override for `map_sloping_profiles` (parity with the horizontal path) | ras-commander | robustness agent | todo |
| A4 | Bound root object-header parsing and add a reader read budget (`max_read_bytes`, zero-width datatype refusal) in upstream `hdf5-reader`: prepare the PR; open it only with the user's approval | netcdf-rust (upstream) | engine agent | todo |
| A5 | Global in-flight chunk-decode budget across workers and concurrent calls | fast-hdf | engine agent | todo |
| A6 | Release wheels: panic-injection hooks behind a `test-hooks` Cargo feature (off in release); quiet panic hook in the extension | fast-hdf | engine agent | todo |
| A7 | Fix `test_fluvial_min_area_native_parity` under pandas 3.1 (strict column-type comparison) | ras-commander | robustness agent | todo |

## B. Coverage (steerable agent, with the user)

| # | Task | Status |
|---|---|---|
| B0 | Inventory sources: ras-commander's `RasExamples` (HEC example projects, all versions), the FEMA eBFE/BLE and TWDB/TDIS RBFS readers, and any other public model sources in ras-commander. List which sources provide which families and versions. | todo |
| B1 | Clear the `h5py_until_qualified` markers (~20 call sites in 16 modules), one family at a time: find or build a real HEC-RAS 7.0.1 file → native identical to h5py → remove the marker. No file exists yet for encroachments, floodway, pump summaries or reference summaries; build small 7.0.1 projects if needed. | todo |
| B2 | Storage-area results (currently refused by the engine) | todo |
| B3 | Reference line and point result joins in the engine (roadmap: in progress) | todo |
| B4 | Pipe-network run summary; 1D summary families beyond cross sections and storage areas; reference-point interpolation layouts | todo |
| B5 | Multi-part polygons with holes, and 2D areas stored as outlines only: keep refusing; record whether any corpus model has them | todo |
| C1 | Legacy cross sections: review ras-commander #692 (5.0.3 hybrid XS schemas) and #676; use 5.0.x files to unblock the engine's "5.x separated cross-section layout" | todo |
| C2 | Add HEC-RAS 6.0–6.5 and 7.0.2 files to the corpus `legacy` profile; run the equivalence suites | todo |
| C3 | Non-ASCII names (low priority; needs a file) | todo |

New public files go into `gpt-cmdr/ras-hdf-test-corpus` with SHA-256 and provenance. Public sources only.

## D. Performance (after A–C)

| # | Task | Status |
|---|---|---|
| D1 | First-read cost of the stored-family index (5–25 ms per file) | todo |
| D2 | Cache family descriptors per session | todo |
| D3 | Evaluate a faster DEFLATE decoder (quiet-machine benchmark, byte-identical) | todo |

## E. Housekeeping before release

| # | Task | Status |
|---|---|---|
| E1 | Engine PRs: merge #33 (`AGENTS.md` reader rule); rebase/finish or close #31 (terrain-mask bitfields) and #41 (layer catalog) | todo |
| E2 | Quiet benchmark for the release; update both performance pages with measured numbers only | todo |
| E3 | ras2cng: move off ras-commander 0.100 in `uv.lock`; use `include_geometry=False` where it only needs values | todo |
| E4 | Release: engine first (PyPI + crates.io), then ras-commander, after the user's approval | todo |

## Later

S3/HTTP range reads (FEMA FFRD, rashdf replacement). This needs A4 and A5 first.
