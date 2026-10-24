# Documentation

Developer documentation for `ras-commander-fast-hdf`: the native HEC-RAS HDF
engine and its Python extension, which together are ras-commander's default
HDF engine. ras-commander 0.107.1 and later install it automatically on
Windows x64, Linux x64 and macOS arm64; `pip install ras-commander` is enough.

If you only want to *use* the engine from ras-commander (installation, the
`auto | h5py | native` modes, which methods run natively, performance and
limitations), read the user guide on the ras-commander site:
[Native Rust HDF engine](https://rascommander.info/ras/native-engine/).

## Guides

| Page | Read it to |
|---|---|
| [Architecture](architecture.md) | Understand the three layers (engine, extension, ras-commander adapter), how a read flows, and why refusals are safe. |
| [Python extension](python-extension.md) | See what `ras_commander_fast_hdf` exposes: versions, sessions, read functions, kernels, errors. |
| [Rust engine](engine.md) | Use the `ras-commander-fast-hdf` crate from Rust: Cargo features, modules, status vocabulary, budgets. |
| [Performance](performance.md) | See how much faster each ras-commander method is with the native engine, and how that was measured. |
| [Building](building.md) | Build the wheel or the crate from source on Windows, Linux or macOS. |
| [Testing and qualification](testing.md) | Run the Rust tests, and qualify a wheel against h5py with ras-commander's equivalence suites. |
| [Releasing](releasing.md) | Version, build, qualify and publish a release to PyPI and crates.io. |

## Reference

Detailed contracts, kept as they were written while each surface was built.
They are exact but terse; start with the guides above.

| Page | Covers |
|---|---|
| [Engine API contract](reference/engine-api.md) | Every engine surface, its statuses, limits and the history of API generations. |
| [Read families](reference/read-families.md) | Stored table families (structures, reference lines/points, terrain modifications, land cover, ...) and their qualification. |
| [Mesh results](reference/mesh-results.md) | 2D mesh summary, time-series and boundary-condition reads. |
| [1D geometry and results](reference/one-d.md) | Cross sections, 1D summary output, 1D structures and pumps. |
| [Conformance](reference/conformance.md) | How the engine was checked against real HEC-RAS files. |
| [Extension contract](reference/extension-contract.md) | The full `ras_commander_fast_hdf` binding contract, by API generation. |
| [Sessions](reference/sessions.md) | `HdfSession` lifetime, caching and ras-commander's session policy. |
| [Session performance design](reference/perf-session.md) | Why sessions are reused and how their caches are bounded. |
| [Validation](reference/validation.md) | Measured results of the original qualification of the extension. |
| [Provenance](reference/provenance.md) | Where the source came from. |
