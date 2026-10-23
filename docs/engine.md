# Rust engine: `ras-commander-rust`

`crates/ras-commander-rust` is a read-only Rust library for HEC-RAS projects
and the HDF5 files HEC-RAS writes. It has no Python dependency. The Python
extension in `python/` is one consumer; any Rust program can be another.

The crate is not published on crates.io. Depend on it by Git commit:

```toml
[dependencies]
ras-commander-rust = { git = "https://github.com/gpt-cmdr/ras-commander-fast-hdf", rev = "<commit>", default-features = false, features = ["hdf", "mesh", "results"] }
```

Minimum Rust version: 1.88. The crate is `#![forbid(unsafe_code)]`.

## Cargo features

| Feature | Adds | Dependencies |
|---|---|---|
| `catalog` (default) | Project (`.prj`) and plan (`.p##`) records, linked geometry/flow files, declared simulation windows | Standard library only |
| `hdf` | Read-only HDF sessions; numeric, compound, string and attribute reads; capabilities; 1D geometry and results; structures; read families; time and units; CRS evidence | `hdf5-reader` (vendored), `ndarray`, `rayon`, `flate2` |
| `mesh` | 2D mesh topology and cell polygon assembly (implies `hdf`) | `geo` |
| `results` | Bounded temporal reduction of result arrays (implies `hdf`) | none extra |

## Main entry points

```rust,ignore
use ras_commander_rust::catalog::ProjectCatalog;

let project = ProjectCatalog::open("Model.prj")?;
println!("{}: {} plans", project.title, project.plans.len());
// Missing or malformed plans are kept in the list with a diagnostic.
```

```rust,ignore
use ras_commander_rust::hdf::ReadOnlyHdfSession;
use ras_commander_rust::mesh::GeometryPolicy;

let source = ReadOnlyHdfSession::open("Model.p01.hdf")?;
for mesh_name in source.mesh_names()? {
    let mesh = source.read_topology(&mesh_name)?;
    let packed = mesh.topology.assemble(GeometryPolicy::Strict, 4)?;
    // packed.coordinates, packed.offsets and packed.cell_ids belong together.
}
```

`ReadOnlyHdfSession` memory-maps the file. The file must not change while a
session is open; returned arrays own their buffers and outlive the session.

| Module | Contents |
|---|---|
| `catalog` | `ProjectCatalog`, plan and file-link records. |
| `hdf` | `ReadOnlyHdfSession` and every HDF read: numeric selections and batches, typed attributes, fixed strings, capability reports, mesh summary/series/boundary reads, cross sections (`xs1d`, `xs_geometry`, `xs_results`), 1D summary and structures, stored read families (`families`, `structures`, `reference_points`, `terrain_modifications`), `time`, `units`, `layers`. |
| `mesh` | `Topology`, `GeometryPolicy` (`Strict` or explicit quarantine) and packed polygon assembly. |
| `results` | `ReductionRequest` and temporal reduction over time rows: maximum, row of the maximum, finite count, and optional threshold exceedance count and first exceedance row. |
| `crs` | Raw projection evidence. The engine reports stored WKT and where it came from; it never guesses an EPSG code. |
| `capability` | The shared status types below. |
| `api` | `API_VERSION` (10) and `capabilities()`, the list of entry points. |

The full contract for every surface is in
[reference/engine-api.md](reference/engine-api.md) and the pages it links.

## Status vocabulary

Every capability report uses one shape, `Capability`:

| Status | Meaning | Carries |
|---|---|---|
| `supported` | Proven and readable | nothing |
| `unavailable` | Canonically absent (HEC-RAS did not write it) | reason |
| `unsupported` | Present but not proven, or over a limit | `UnsupportedCode`, reason |
| `corrupt` | Present but damaged or contradictory | reason |
| `delegated` | Checked by another, more specific reader | reason |

`UnsupportedCode` is one of `encoding`, `schema`, `identity_layout`,
`resource_limit` or `not_implemented`. Inspect codes, never parse the reason
text.

Errors (`Error` with `ErrorKind`): `io`, `internal`, `file_not_found`,
`not_found`, `unsupported`, `invalid_request`, `corrupt`,
`structural_topology` and `invalid_geometry`.

## Budgets and limits

Reads take explicit byte limits (`max_bytes`, `metadata_max_bytes`; 64 MiB
by default) and are checked before any allocation. The generic reads
(`read_numeric`, `read_numeric_batch`, `read_topology`) refuse more than 2 GiB
of decoded values per call. Descriptor counts, string payloads and recursion
depth have their own fixed caps. Exceeding any of them is
`unsupported(resource_limit)`, never a partial result.

## Versions of HEC-RAS covered

Qualified on files written by HEC-RAS 5.x, 6.x and 7.0.x. Layouts the corpus
does not cover are refused (notably cross-section tables written before
HEC-RAS 5.0.7), so ras-commander reads them with h5py.

## Licenses

MIT. The vendored `hdf5-reader` and `hdf5-core` are modified copies of
[netcdf-rust](https://github.com/roteiro-gis/netcdf-rust), used under its MIT
option; changes are listed in `vendor/LOCAL_PATCHES.md` and
`vendor/hdf5-reader/PROVENANCE.md`. `THIRD-PARTY-NOTICES.md` lists every crate
linked with all features enabled; regenerate it after any `Cargo.lock` change:

```bash
python scripts/generate_third_party_notices.py --title "ras-commander-rust (all features)"
```
