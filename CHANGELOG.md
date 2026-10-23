# Changelog

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
