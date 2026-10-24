# Changelog

## 0.2.0 (2026-10-24)

- One Rust crate, `ras-commander-fast-hdf` (Rust import
  `ras_commander_fast_hdf`), at the repository root, ready to publish on
  crates.io. It replaces the engine crate `ras-commander-rust`, the separate
  extension crate in `python/`, the export crate `ras-export` and the vendored
  `hdf5-reader`/`hdf5-core` path crates. The name `ras-commander-rust` is
  retired. Rust users enable `hdf`, `mesh` and `results` as needed (default:
  `catalog`, standard library only); without the `python` feature no PyO3 or
  NumPy is in the dependency tree.
- The PyPI wheel is built from the same crate (`python/pyproject.toml`, features
  `python,extension-module`; release wheels add `exports-all,exports-pmtiles`).
  The Python API and behaviour are unchanged from 0.1.1; extension
  `API_VERSION` stays 11 and engine `API_VERSION` stays 10.
- HDF5 reader rebased from `hdf5-reader` 0.9.1 and its older local patch set
  onto upstream 0.9.3 (roteiro-gis/netcdf-rust `v0.9.3`) plus 14 local patches
  (`vendor/patches/`, described in `vendor/LOCAL_PATCHES.md`).
- `freestiler-core` (walkerke/freestiler `beb64b0`), previously a Git
  dependency, is embedded from `vendor/freestiler-core`. No Git dependencies
  remain.
- Embedded third-party source is compiled as private modules, recorded with
  hashes in `vendor/PROVENANCE.json` and checked by
  `scripts/vendor_sources.py --check`. Upstream's reader tests run from
  `vendor/reader-tests`.
- Layout: engine in `src/`, Python bridge in `src/python/` (was `python/src`),
  export writers in `src/exports/`, all tests in `tests/` with fixtures in
  `tests/fixtures`, `build.rs` at the root, notice scripts in `scripts/`.
- Licence notices: `THIRD-PARTY-NOTICES.md` (crate, all features) and
  `python/licenses/THIRD-PARTY-NOTICES.md` (wheel) come from one script,
  `scripts/generate_third_party_notices.py`. Inside the wheel the notices file
  is now under `.dist-info/licenses/licenses/` (same file name), because the
  sdist root already holds the crate's own notices.
- One name everywhere: the notices list the engine as
  `ras-commander-fast-hdf`, and the wheel's copy of this repository's licence
  is now `ras-commander-fast-hdf-LICENSE.txt` (was
  `ras-commander-rust-LICENSE.txt`). ras-commander 0.108.0 and later check
  these names.
- Path-based typed reads can share an open session. Every module-level
  function that takes an HDF path for a typed read (typed attributes, fixed
  strings, 1D line, geometry, result, summary and structure readers, 2D
  geometry families, stored-family and output-block readers) now also accepts
  an open `HdfSession` in place of the path. The call then reuses that
  session's file mapping, chunk cache and metadata memo instead of opening the
  file and repeating the layout and encoding proofs for each small read. The
  values are the same. A closed session raises `ValueError("HDF session is
  closed")`; a path behaves exactly as before. The module flag
  `SESSION_SOURCES = True` lets callers detect support. This is additive:
  `API_VERSION` stays 11.
- Cross-section geometry reads other than the property table no longer decode
  the whole HTAB value table (`Property Tables/XSEC Value`, stored by HEC-RAS
  with one row per compressed chunk) only to count its leading section rows.
  That count is used by the property-table field alone; it is now computed
  when that field or the full `xs_geometry_inventory` is requested. Results are
  unchanged. On the BaldEagle example the first cross-section read in a session
  drops from about 40-70 ms to under 2 ms.
- Engine (additive within API 10): `Topology::face_lines()` returns every 2D
  face as a polyline (start point, stored perimeter vertices, end point) and
  `read_mesh_perimeter()` returns an area's stored `Perimeter` as a closed ring,
  both exactly as ras-commander's Python `get_mesh_cell_faces` and
  `get_mesh_areas` build them (bit-identical on 20 public HEC example files).
  The extension does not call them yet; its API is unchanged.
- The engine no longer depends on `geo`. Cell-ring validation and winding now
  use the `robust` crate's adaptive-precision `orient2d` (the predicate `geo`
  used) and a port of `geo` 0.29's segment-contact classification. `geo` 0.29
  pinned `i_overlay`/`i_float` versions that conflict with `geo` 0.33, so Rust
  consumers that use `geo` 0.33 (terrio 0.5, RustGIS) could not link the
  engine. Results are unchanged: unit tests compare the predicates with
  `geo` 0.29 on every segment pair of a 4 x 4 grid and 800,000 random and
  near-collinear pairs, and packed polygons and omissions of 134 mesh/policy
  reads across 31 public HEC example HDF files are bit-identical to 0.1.1.
  Engine `API_VERSION` (10) and extension `API_VERSION` (11) are unchanged.
- Engine: HEC-RAS 5.0 Maximum/Minimum Water Surface summaries, which carry
  only `Row 0:  WSEL` = `Feet` and `Row 1:  Time` = `Days` attributes and no
  `Location`, now qualify as cell summaries (new `DomainRule::LegacyRowLabels`)
  instead of `unsupported(identity_layout)`. Values are the literal stored
  arrays, so ras-commander results do not change: before, it read the same
  values through its literal fallback. Any other layout stays refused.

## 0.1.1 (2026-10-24)

- Linux wheels for older and musl-based distributions: `manylinux2014`
  (glibc 2.17 or later) replaces `manylinux_2_28`, and a new `musllinux_1_2`
  wheel covers Alpine. `pip install ras-commander` no longer tries to compile
  the engine on those systems. Engine and extension code are unchanged.

- Documentation: this package is ras-commander's default HDF engine, not an
  optional extension. ras-commander 0.107.1 and later list it as a core
  dependency on Windows x64, Linux x64 and macOS arm64 (CPython), so
  `pip install ras-commander` installs it; on other platforms ras-commander
  uses h5py. `RAS_COMMANDER_FAST_HDF=h5py` still turns it off. The package
  itself is unchanged.

## 0.1.0 (2026-10-23)

First public release. The engine (previously the private
`ras-commander-rust` repository) and the Python extension (previously
`rust/fast-hdf` in ras-commander) now live together in this repository.

- Extension `API_VERSION` 11, engine `API_VERSION` 10. Works with
  ras-commander 0.107.0 and later.
- Native reads for ras-commander's plan results, geometry, terrain and
  land-cover HDF access: numeric, compound and fixed-length string datasets,
  attributes, 2D mesh geometry, summaries, time series and boundary
  conditions, cross sections and 1D results, structures, reference lines and
  points, terrain modifications and stored classification tables.
- Native 2D cell polygon assembly, fluvial/pluvial kernels, PMTiles and vector
  exports (GeoJSON, FlatGeobuf, GeoParquet, GeoPackage), and RAS text-file
  parsing.
- The wheel's `THIRD-PARTY-NOTICES.md` covers every bundled crate. `epsg-utils`
  (used by the GeoPackage writer) declares MIT but publishes no licence text;
  its notice reproduces the MIT text with an explanation.
- Qualified identical to h5py through ras-commander on 137 HEC-RAS 5.x–7.x
  files (3,800 method × file pairs in three modes) and 1,428 notebook calls,
  on Windows and Linux.
- `ENGINE_COMMIT` is now the commit of this repository the wheel was built
  from. Wheels built before the move report the private engine commit
  `0b024adc7857e9fb2d364997ad002e01a939cec1`, whose engine source is identical.
