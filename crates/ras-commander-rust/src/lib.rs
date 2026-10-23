#![forbid(unsafe_code)]
//! Native RAS Commander read-only domain engine. Owned outputs outlive sessions.
//! No Python bindings, execution, tiling, or high-level analysis is included.
pub mod api;
pub mod error;
pub use error::{Error, ErrorKind, Result};
#[cfg(feature = "catalog")]
pub mod catalog;
#[cfg(feature = "hdf")]
pub mod hdf;
#[cfg(feature = "mesh")]
pub mod mesh;
#[cfg(feature = "results")]
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
pub mod crs;
#[cfg(feature = "hdf")]
pub mod schema;

pub mod capability;
pub use api::{capabilities, EngineCapabilities, Entrypoint, API_VERSION};
pub use capability::{Capability, Reported, UnsupportedCode};
