//! Native, read-only engine for HEC-RAS project and plan files and the HDF5
//! files HEC-RAS writes, in pure Rust.
//!
//! A read either returns exactly what h5py would give (values, dtype, shape,
//! strings, attributes, order) or a typed refusal ([`ErrorKind`],
//! [`Capability`]); it never guesses, repairs or coerces. Owned outputs outlive
//! the session that read them. Nothing here writes HEC-RAS files or runs
//! HEC-RAS.
//!
//! # Features
//!
//! | Feature | Adds |
//! |---|---|
//! | `catalog` (default) | [`catalog`]: project and plan text inventory; standard library only. |
//! | `hdf` | `hdf`: read-only HDF5 sessions, numeric/compound/string datasets, attributes, identities, units, time. |
//! | `mesh` | `mesh`: 2D mesh geometry and topology (implies `hdf`). |
//! | `results` | `results`: 1D/2D results, summaries and time series (implies `hdf`). |
//! | `python` | The `ras_commander_fast_hdf` Python extension (PyO3) used by ras-commander. Not needed from Rust. |
//! | `extension-module`, `exports-*` | Python wheel builds (maturin). |
//!
//! ```toml
//! [dependencies]
//! ras-commander-fast-hdf = { version = "0.2", features = ["hdf", "mesh", "results"] }
//! ```
//!
//! # Third-party code
//!
//! HDF5 decoding uses a modified copy of the `hdf5-reader` and `hdf5-core`
//! crates from [roteiro-gis/netcdf-rust](https://github.com/roteiro-gis/netcdf-rust)
//! v0.9.3 (MIT OR Apache-2.0, Copyright (c) 2026 Ian Norden), compiled as
//! private modules. The Python extension's vector tiles use a copy of
//! `freestiler-core` from [walkerke/freestiler](https://github.com/walkerke/freestiler)
//! (MIT, Copyright (c) 2026 Kyle Walker; its supercluster port is ISC). Their
//! licence texts, change notices and provenance are in the package's `vendor/`
//! directory and in `THIRD-PARTY-NOTICES.md`.
//!
//! # Safety
//!
//! The engine modules are `#![forbid(unsafe_code)]`. Only the embedded HDF5
//! reader uses `unsafe`: read-only memory mapping of the input file and
//! bounds-checked copies of decoded chunks into preallocated output buffers.
#![deny(unsafe_code)]

#[forbid(unsafe_code)]
pub mod api;
#[forbid(unsafe_code)]
pub mod error;
pub use error::{Error, ErrorKind, Result};
#[cfg(feature = "catalog")]
#[forbid(unsafe_code)]
pub mod catalog;
#[cfg(feature = "hdf")]
#[forbid(unsafe_code)]
pub mod hdf;
#[cfg(feature = "mesh")]
#[forbid(unsafe_code)]
pub mod mesh;
#[cfg(feature = "mesh")]
#[forbid(unsafe_code)]
mod predicates;
#[cfg(feature = "results")]
#[forbid(unsafe_code)]
pub mod results;
#[cfg(all(test, feature = "hdf"))]
thread_local! { pub(crate) static POOL_CREATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(feature = "hdf")]
pub(crate) fn pool(threads: usize) -> Result<std::sync::Arc<rayon::ThreadPool>> {
    if threads == 0 || threads > 1024 {
        return Err(Error::new(
            ErrorKind::InvalidRequest,
            "threads must be between 1 and 1024",
        ));
    }
    static POOLS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::BTreeMap<usize, std::sync::Arc<rayon::ThreadPool>>>,
    > = std::sync::OnceLock::new();
    let mut pools = POOLS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| Error::new(ErrorKind::Internal, "pool registry poisoned"))?;
    if let Some(pool) = pools.get(&threads) {
        return Ok(pool.clone());
    }
    #[cfg(test)]
    POOL_CREATIONS.with(|n| n.set(n.get() + 1));
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
    let pool = std::sync::Arc::new(pool);
    pools.insert(threads, pool.clone());
    Ok(pool)
}
#[forbid(unsafe_code)]
pub mod crs;
#[cfg(feature = "hdf")]
#[forbid(unsafe_code)]
pub mod schema;

#[forbid(unsafe_code)]
pub mod capability;
pub use api::{capabilities, EngineCapabilities, Entrypoint, API_VERSION};
pub use capability::{Capability, Reported, UnsupportedCode};

// --- Embedded third-party source (vendor/; see vendor/PROVENANCE.json) -------
//
// Compiled from the vendored files exactly as recorded there. They are private
// modules: none of their items are part of this crate's public API. rustfmt and
// most lints are not applied to upstream code.

/// hdf5-core 0.9.3 (roteiro-gis/netcdf-rust, MIT OR Apache-2.0).
#[cfg(feature = "hdf")]
#[path = "../vendor/hdf5-core/src/lib.rs"]
#[rustfmt::skip]
#[allow(unused, clippy::all)]
mod hdf5_core;

/// hdf5-reader 0.9.3 with 14 local patches (roteiro-gis/netcdf-rust, MIT OR
/// Apache-2.0). The only module that uses `unsafe`: read-only memory mapping
/// and chunk copies into preallocated buffers, each with a `SAFETY` comment
/// (upstream's `undocumented_unsafe_blocks` and `unsafe_op_in_unsafe_fn` lints
/// stay denied).
#[cfg(feature = "hdf")]
#[path = "../vendor/hdf5-reader/src/lib.rs"]
#[rustfmt::skip]
#[allow(unsafe_code, unused, clippy::all)]
#[deny(unsafe_op_in_unsafe_fn, clippy::undocumented_unsafe_blocks)]
mod hdf5_reader;

/// freestiler-core 0.1.0 at walkerke/freestiler beb64b0 (MIT; supercluster ISC).
#[cfg(feature = "python")]
#[path = "../vendor/freestiler-core/src/lib.rs"]
#[rustfmt::skip]
#[forbid(unsafe_code)]
#[allow(unused, clippy::all)]
mod freestiler_core;

// --- Python extension (feature "python") --------------------------------------

/// Arrow IPC -> GeoJSON/FlatGeobuf/GeoParquet/GeoPackage/PMTiles writers.
#[cfg(feature = "exports-arrow")]
#[forbid(unsafe_code)]
mod exports;

/// The `ras_commander_fast_hdf` Python module: PyO3 bindings over the engine.
///
/// The bindings match the engine's `#[non_exhaustive]` enums with wildcard arms,
/// as an external consumer must. Inside one crate those arms are unreachable
/// today; they keep the bindings total if the engine adds a variant.
#[cfg(feature = "python")]
#[allow(unreachable_patterns)]
mod python;
