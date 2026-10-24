//! Upstream test harness for the embedded HDF5 reader; see `../Cargo.toml`.
//!
//! The library is named `hdf5_reader` and re-exports the embedded reader at its
//! root, so upstream tests written against the `hdf5-reader` crate compile
//! unchanged. Its in-source unit tests run here as well as in the main crate.
#![allow(dead_code, unused_imports)]

#[path = "../../hdf5-core/src/lib.rs"]
pub mod hdf5_core;

#[path = "../../hdf5-reader/src/lib.rs"]
pub mod hdf5_reader;

pub use self::hdf5_reader::*;
