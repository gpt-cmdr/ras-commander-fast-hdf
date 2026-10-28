# Native HDF sessions

A session keeps one HEC-RAS HDF file open for repeated native reads, so metadata and
decoded chunks are reused instead of being re-read on every call. This page describes the
explicit `HdfSession` object, the small session cache that ras-commander uses behind its
public readers, when sessions are closed, and the cache settings. It applies to bridge
`API_VERSION` 11 built against engine API 10. The engine-side caching design is in
[perf-session.md](perf-session.md).

## `HdfSession`

`ras_commander_fast_hdf.HdfSession(path, threads=None, parallel=True)` keeps one
read-only engine session open. Use it as a context manager or call `close()`. Closing is
idempotent, waits for active native reads and releases file mappings. Reading after close
raises `ValueError`; NumPy buffers returned earlier remain valid.

Methods include numeric and batch reads, mesh summary, series and boundary reads, their
inventories, and series reduction. Typed element selections take contiguous
`numpy.uint64` arrays; element and extrema indices are returned as NumPy arrays. The
module-level functions remain available and still accept list inputs; since API 7 their
element and index outputs are NumPy arrays too.

Numeric reads above 2 GiB of decoded values are refused by the engine (engine API 10).

## Sessions as the source of typed reads

The module-level typed reads that take a path (`typed_group_attributes`,
`typed_dataset_attributes`, `raw_fixed_strings`, the 1D line, geometry, result,
summary and structure readers, `geometry_family`, `mesh_geometry_tables`,
`read_mesh_geometry`, `stored_family_index`, `read_family_dataset`,
`read_output_block` and the other path readers) accept an open `HdfSession` as
their first argument instead of the path:

```python
with native.HdfSession(path) as s:
    times = native.raw_fixed_strings(s, "Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/Time Date Stamp")
    xs = native.read_xs_result(s, "unsteady", "Water Surface", (0, rows, 1))
```

A path opens the file, maps it, parses the superblock and proves the layout and encoding
of every dataset it touches, then closes it again. With a session these steps happen once
per session: the session-local memo (keyed by every argument that affects a verdict,
including `max_bytes`) and the decoded-chunk cache are shared by all reads. The engine
computes the same values either way. A closed session raises `ValueError("HDF session is
closed")` and is never reopened; any argument that is neither a session nor a path keeps
the path conversion's `TypeError`. The module attribute `SESSION_SOURCES` is `True` when
the extension supports this (additive; `API_VERSION` 11).

## Sessions from bytes (Rust)

`ReadOnlyHdfSession::from_bytes(data, name, threads, parallel)` opens an HDF5 file held
in memory instead of a path: a download, an upload, or a file in WebAssembly, where there
is no file system. `name` stands in for the path in identities and error messages and is
never opened. Reads are exactly those of the same file opened by path, with the same
budgets and refusals. The session owns the bytes, so the whole file stays in memory while
it is open. Where the platform cannot start threads (WebAssembly without threads), a
one-thread session runs its work on the calling thread. The Python extension does not
expose it.

## Session cache in ras-commander

`fast_hdf.read_dataset()` and `read_datasets()` reuse at most four sessions in an LRU
cache. Keys contain the resolved path, size and nanosecond modification time. Changing
explicit thread settings replaces that path's cached session.

A session opened inside a public ras-commander call (anything wrapped by `log_call` or
`standardize_input`) is shared by all reads of that call and is closed when the outermost
public call returns, so no HDF stays open after a public method returns (an open file
blocks rename and delete on Windows and on network shares). Sessions opened by calling
the low-level readers directly stay cached.

`RAS_COMMANDER_HDF_SESSION_CACHE` selects the policy:

| Value | Behaviour |
|---|---|
| unset or `1` | As above: per public call. |
| `0` | Open and close per operation. |
| `persist` | Keep sessions across public calls until `close_sessions()` (fastest for repeated calls; holds the files open). |

`fast_hdf.close_sessions(path=None)` closes one path or all cached sessions. On Windows,
`RasCmdr.compute_plan()` closes this cache before preparing execution; parallel and
test-mode paths delegate there. Explicit native sessions are owned by the caller and must
be closed before execution or source mutation. Do not read the same files concurrently
with HEC-RAS or another writer. Stat-based invalidation cannot detect a same-size edit
whose modification time is deliberately restored.

The four-file bound limits handles, not total memory. Engine metadata and chunk caches
keep their own budgets and documented limitations. Cached reads are serialized with cache
eviction; native session methods serialize against close. Public pandas/xarray
construction and automatic unsupported-layout fallback are unchanged. A read-only h5py
handle held by the caller must also be closed before replacing a file on Windows.

## Cache settings (API 8 and later)

`HdfSession` takes keyword-only `cache_bytes` (default 64 MiB) and `cache_slots` (default
521), and provides `cache_stats()`. Defaults derive from the engine and reader
(`hdf5-reader`) constants. Full reads above the byte limit bypass retention. Zero bytes disables both
decoded-chunk retention and the optional 32 MiB hot-array copy, so zero means no data
retention. Decoder scratch memory is additional to the configured retained cache and
result-byte budgets.

Full-range slices reuse the reader hot-array cache (at most 32 MiB per eligible dataset);
large reads still bypass it, and closing the session drops all metadata and cache
ownership. Sparse reads enter the supplied thread pool only when their bucket needs
decoding. Returned NumPy buffers are always owned, and the error and budget contracts are
unchanged.

## Mesh face lines

`read_mesh_face_lines(area, max_bytes=2 GiB)` returns every face of mesh `area` as packed
polylines in the GeoArrow linestring layout: a dictionary of `coordinates` (float64,
`(N, 2)`), `offsets` (int64, one more than the face count) and `face_ids` (int64, `0..faces`
in stored order). Face `i` is `coordinates[offsets[i]:offsets[i + 1]]`: its start point, its
`Faces Perimeter Values` rows in stored order, then its end point. That is exactly the line
ras-commander's Python `HdfMesh.get_mesh_cell_faces` builds, so
`shapely.from_ragged_array(GeometryType.LINESTRING, coordinates, (offsets,))` gives the
same geometry (WKB) in one call.

Only the four face tables are read; cell tables are not, so a mesh whose cells cannot be
assembled still has its face lines. Layouts that the Python per-face loop reads differently
are refused rather than approximated: a table that is not `(rows, 2)` raises
`UnsupportedError`; unequal endpoint and perimeter tables, a negative or out-of-range index or
perimeter span, a negative count and a nonfinite coordinate raise `TopologyError`. A missing
table raises `KeyError`. The packed output (16 bytes per coordinate and per face) counts
against `max_bytes`; above it the call raises `UnsupportedError` with code `resource_limit`.
ras-commander falls back to its Python builder on any of these in `auto` mode.

## Mesh summary and boundary batches

`read_mesh_summary_values` returns the same numeric array as the summary read without
allocating element identities or a result dictionary. Existing summary methods keep their
return contract.

`read_mesh_boundary_batch(area, rows, names=None)` returns a name-keyed dictionary of
selection results for all boundary-condition lines or a named subset; its output budget
is shared across the batch. `fast_hdf.read_datasets` groups canonical per-area
boundary-condition paths into these calls; legacy non-area paths keep the generic numeric
batch.

In `auto` mode, a typed qualification refusal in a boundary-condition batch is retried
through the generic batch reader, preserving h5py-identical values and ordering. `native`
mode stays strict, and host I/O, closed-session and internal failures are never retried.
Requested-only boundary reads and named subsets ignore corrupt unrequested siblings;
inventories and all-lines batches report them. Non-inline extensible-array indexes are
refused explicitly instead of silently returning zeros.
