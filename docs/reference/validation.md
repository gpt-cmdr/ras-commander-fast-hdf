# Qualification evidence

This page records the measured evidence behind the Python extension: exact comparisons of
native reads against h5py on real HEC-RAS files, PMTiles validity checks, and early timing
measurements. The numbers describe specific builds and machines; they are evidence, not a
guarantee for every file or platform. Current test commands are in
[../testing.md](../testing.md).

## Initial wheel qualification

The initial Windows release wheel passed 1,142 exact numeric dataset comparisons across
nine real HDF files, and polygon comparisons covering 141,070 cells in six geometry files.
Comparisons checked dtype, shape, bytes, original cell IDs, topology, CRS, bounds, area
and retained perimeter coordinates.

All 304 decoded PMTiles passed geometry validity, source association, properties,
coverage and source identity at every tested zoom. Cases include BaldEagle g01 (87,039
cells, zooms 10-15) and the two-mesh g11 geometry (4,425 cells, zooms 6-12). Integer-grid
repair corrected 64 invalid features found in the original export. The integrated suite
passed 57 tests with five explicit fixture skips; an additional unsupported-LZF-filter
regression passed. The fixture gaps include small plans without computed 2D results and an
absent Depth dataset.

The qualified wheel SHA-256 was
`ad28b85fa1b739b9896f40cb01b31d9c3e68b804c5914f74a911cbe0e3dae63e`. This identifies the
measured prototype, before the final source-ring validity guard and current-API
integration; later checks identify their wheel separately. It is not a cross-platform
qualification.

## Measured prototype performance

Three fresh-process samples per case, alternating backend order, warm filesystem cache
without a cache flush; Windows, four logical CPUs, Python 3.13.13, NumPy 2.3.3, h5py
3.14.0, Shapely 2.1.2 and GeoPandas 1.1.1. Input files were on a local disk. Whole-plan
reads retained all numeric arrays on both backends. Peak process-tree RSS was sampled
every 20 ms and includes interpreter overhead; short-lived allocation peaks can be missed.

The computed BaldEagle p07 plan contains 383 numeric datasets, about 1.09 GB decoded, in a
482,604,388-byte HDF file. Polygon measurements use the g03 geometry with 24,718 cells.
These are measurements of an early integration prototype, not of the current lazy-view
and diagnostic adapter.

| Operation | Python median (s) | Rust, 1 thread (s) | Rust, 4 threads (s) |
|---|---:|---:|---:|
| Whole-plan numeric arrays | 3.676 | 2.740 | 1.009 |
| Largest dataset | 1.554 | 0.947 | 0.291 |
| Every fourth row and column | 3.979 | 1.272 | 0.507 |
| Polygon GeoDataFrame | 1.548 | 0.066 | 0.050 |

Whole-plan peak RSS increased from 1,276 MiB to 1,467 MiB at four Rust threads. The
selected-read case decreased from 720 MiB to 340 MiB. Faster extraction therefore does not
imply uniformly lower memory usage.

End-to-end PMTiles samples also completed, but a matched-thread comparison was not, so no
PMTiles speed multiplier is claimed. Both routes disable configured feature dropping and
simplification; the native route also repairs quantized rings. No HEC-RAS solver was run.

## Current-API integration

The ras-commander integration preserves the lazy result view, selection, bounded batching,
ghost-cell and polygon diagnostic contracts. Its suites passed 112 tests with one fixture
skip, including all six real polygon cases, a multilayer PMTiles export, and rejection of
self-crossing and zero-area rings. A separate test proved that selected and bounded
result-view reads invoke the native binding. The wheel SHA-256 for that run was
`358f0835b080cbbfa89144b25538c225f4a19911a092923d52ce7cc9c7441ca6`.

Fourteen Rust unit tests passed at that point. A separate three-sample polygon comparison
with that wheel produced medians of 2.196 s for Python and 0.093 s for four-thread Rust
(about 24x). The Python samples ranged from 1.552 to 3.865 s and Rust from 0.089 to
0.106 s; this variability limits precision. These timings include the current diagnostic
contract and final source-ring validity checks.

## Limits of this evidence

Not covered by this evidence: browser/WebAssembly instantiation, and a full comparison
against an independent GIS reference reader (a stricter cross-reader comparison passed 23
of 31 layers across three fixtures; the remaining field, CRS, Z, depth-source and
reference-reader differences are explicit failures). Cross-platform wheels are built and
tested by CI (see [../releasing.md](../releasing.md)). This evidence does not establish
hydraulic engineering validity.
