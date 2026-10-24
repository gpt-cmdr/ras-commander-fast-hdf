# freestiler-core: provenance and change notice

`vendor/freestiler-core/src` is the Rust source of the `freestiler-core` crate (version
0.1.0) from [walkerke/freestiler](https://github.com/walkerke/freestiler), commit
`beb64b0d4414da895ec67d02b2f91db6ab6acc37` (2026-10-04, "Merge pull request #28 from
walkerke/feat/in-memory-categorical-clustering"), path `src/rust/freestiler-core`.
`freestiler-core` is not published on crates.io (`publish = false` upstream), so
ras-commander-fast-hdf compiles it as the internal module `crate::freestiler_core`
instead of depending on it through Git.

## Licences

The crate is `MIT AND ISC`:

- freestiler's own code: MIT, Copyright (c) 2026 Kyle Walker. Full text: `LICENSE.md`.
  `LICENSE` is the CRAN-format year/holder stub that accompanies it upstream.
- `src/supercluster/`: a port of Supercluster 8.0.1 (Copyright (c) 2021 Mapbox) and
  KDBush 4.1.0 (Copyright (c) 2026 Vladimir Agafonkin), ISC. Full text:
  `src/supercluster/UPSTREAM-LICENSES` and `COPYRIGHTS` (upstream `inst/COPYRIGHTS`).
- `LICENSE.note` is upstream's explanation of the split.

Both licences permit copying and modifying the source provided the copyright and
permission notices are kept; all of the files above are copied byte for byte.

## What was changed

Changes by CLB Engineering Corporation for ras-commander-fast-hdf, all mechanical and
reproducible with `python scripts/vendor_sources.py` (rules recorded in
`vendor/PROVENANCE.json`):

1. Every `.rs` file starts with a two-line notice naming the upstream and this file.
2. Crate-relative paths `crate::` are re-rooted at the module: `crate::freestiler_core::`.
3. Upstream's optional Cargo features `duckdb`, `geoparquet`, `fsst` and `fastpfor` are
   not vendored. Their `cfg(feature = "...")` conditions are replaced by the always-false
   `cfg(any())` (with a comment naming the feature), so the code they guard is compiled
   out exactly as in an upstream build without those features. ras-commander-fast-hdf
   never enabled them.
4. `src/file_input.rs` and `src/streaming.rs` are omitted: upstream compiles them only
   with the `duckdb` or `geoparquet` features.
5. The `test_mlt_core_decodes_*` unit tests in `src/mlt.rs` decode tiles with
   upstream's dev-dependency `mlt-core`, which builds C++ code through CMake. They are
   compiled out with the same always-false `cfg(any())`; the other freestiler unit tests
   run as part of this crate's tests.

No function, constant or algorithm was changed. `tests/fixtures/supercluster-8.0.1.json.gz`
is upstream's oracle for the supercluster unit tests, copied unchanged; it stays in the
repository and is not part of the published crate.

## Dependencies

Upstream's manifest pins several versions (`rayon = "=1.10.0"`, `rayon-core`, `spade`,
`indexmap`, `jobserver`) only to keep R's CRAN toolchain (Rust 1.77) building. The
ras-commander-fast-hdf manifest does not repeat those pins, so it does not constrain
downstream crates; `Cargo.lock` keeps the versions the wheel was qualified with. The
output-format crates keep upstream's exact pins: `pmtiles2 = "=0.3.1"` and
`integer-encoding = "=4.0.2"`.
