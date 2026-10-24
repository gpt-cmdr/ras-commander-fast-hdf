# Changelog

## Unreleased

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
- The stored-family index (the first stored-family read of a file in a
  session) runs its independent work on the session's thread pool: the walk of
  `Output Blocks` warms the group listings level by level in parallel, and the
  per-dataset descriptors of every read family and non-Base output block are
  computed in parallel, then handled in path order exactly as before (the
  first error in order still wins; the walk's depth and entry limits refuse
  exactly as before). One-thread sessions, which every path-based call uses,
  stay sequential. Results are unchanged.

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
