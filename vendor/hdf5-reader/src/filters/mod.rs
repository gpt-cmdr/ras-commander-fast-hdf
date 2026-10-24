// ras-commander-fast-hdf: modified copy of roteiro-gis/netcdf-rust v0.9.3, MIT OR Apache-2.0,
// Copyright (c) 2026 Ian Norden. Changes are listed in vendor/hdf5-reader/PROVENANCE.md.
pub mod deflate;
pub mod fletcher32;
#[cfg(feature = "hdf")]
pub mod lz4;
pub mod nbit;
pub mod scaleoffset;
pub mod shuffle;

use std::collections::HashMap;

use crate::hdf5_reader::error::{Error, Result};
use crate::hdf5_reader::messages::filter_pipeline::FilterDescription;

/// Standard HDF5 filter IDs.
pub const FILTER_DEFLATE: u16 = 1;
pub const FILTER_SHUFFLE: u16 = 2;
pub const FILTER_FLETCHER32: u16 = 3;
pub const FILTER_SZIP: u16 = 4;
pub const FILTER_NBIT: u16 = 5;
pub const FILTER_SCALEOFFSET: u16 = 6;
/// HDF5 registered LZ4 filter.
pub const FILTER_LZ4: u16 = 32004;

/// A user-supplied filter function.
///
/// Takes the filter description, input data, and element size, then returns
/// the decoded output.
pub type FilterFn = Box<dyn Fn(&FilterDescription, &[u8], usize) -> Result<Vec<u8>> + Send + Sync>;

enum FilterImplementation {
    Builtin,
    Custom(FilterFn),
}

/// A registry of filter implementations.
///
/// Comes pre-loaded with deflate, shuffle, and fletcher32. Users can register
/// additional filters (e.g., Blosc, LZ4, ZFP) before reading datasets.
pub struct FilterRegistry {
    filters: HashMap<u16, FilterImplementation>,
}

impl FilterRegistry {
    /// Create a new registry with the built-in filters pre-registered.
    pub fn new() -> Self {
        let mut registry = FilterRegistry {
            filters: HashMap::new(),
        };
        registry.register_builtin(FILTER_DEFLATE);
        registry.register_builtin(FILTER_SHUFFLE);
        registry.register_builtin(FILTER_FLETCHER32);
        registry.register_builtin(FILTER_NBIT);
        registry.register_builtin(FILTER_SCALEOFFSET);
        #[cfg(feature = "hdf")]
        registry.register_builtin(FILTER_LZ4);
        registry
    }

    fn register_builtin(&mut self, id: u16) {
        self.filters.insert(id, FilterImplementation::Builtin);
    }

    /// Register a custom filter implementation for the given filter ID.
    ///
    /// Overwrites any previously registered filter with the same ID.
    pub fn register(&mut self, id: u16, f: FilterFn) {
        self.filters.insert(id, FilterImplementation::Custom(f));
    }

    /// Apply a single filter by ID.
    pub fn apply(
        &self,
        filter: &FilterDescription,
        data: &[u8],
        element_size: usize,
    ) -> Result<Vec<u8>> {
        self.apply_with_limit(filter, data, element_size, None)
    }

    /// Apply a single filter by ID, passing a maximum decoded output length to
    /// built-in filters that can enforce it while decoding.
    pub fn apply_with_limit(
        &self,
        filter: &FilterDescription,
        data: &[u8],
        element_size: usize,
        max_output_len: Option<usize>,
    ) -> Result<Vec<u8>> {
        match self.filters.get(&filter.id) {
            Some(FilterImplementation::Builtin) => {
                apply_builtin_filter_with_limit(filter, data, element_size, max_output_len)
            }
            Some(FilterImplementation::Custom(_)) if max_output_len.is_some() => {
                Err(Error::UnsupportedFilter(format!(
                    "custom filter {} has no qualified decoded-size bound",
                    filter.id
                )))
            }
            Some(FilterImplementation::Custom(f)) => f(filter, data, element_size),
            None => Err(Error::UnsupportedFilter(format!("filter id {}", filter.id))),
        }
    }
}

impl Default for FilterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Apply the filter pipeline in reverse (decompression direction) to a chunk.
///
/// HDF5 stores filters in the order they were applied during writing.
/// On read, we apply them in reverse order.
///
/// If `registry` is `None`, the built-in filter set is used.
///
/// `filter_mask` is a bitmask where bit N being set means filter N should be skipped.
pub fn apply_pipeline(
    data: &[u8],
    filters: &[FilterDescription],
    filter_mask: u32,
    element_size: usize,
    registry: Option<&FilterRegistry>,
) -> Result<Vec<u8>> {
    apply_pipeline_with_limit(data, filters, filter_mask, element_size, registry, None)
}

/// Apply the filter pipeline in reverse (decompression direction) to a chunk,
/// passing a maximum decoded output length to built-in filters that support
/// bounded decompression.
pub fn apply_pipeline_with_limit(
    data: &[u8],
    filters: &[FilterDescription],
    filter_mask: u32,
    element_size: usize,
    registry: Option<&FilterRegistry>,
    max_output_len: Option<usize>,
) -> Result<Vec<u8>> {
    // Count active filters so an all-skipped pipeline can avoid the loop.
    let active_count = filters
        .iter()
        .enumerate()
        .rev()
        .filter(|(i, _)| filter_mask & (1 << i) == 0)
        .count();

    if active_count == 0 {
        validate_output_limit("filter pipeline", data.len(), max_output_len)?;
        return Ok(data.to_vec());
    }

    // For a single active filter, avoid the double-buffer loop overhead.
    if active_count == 1 {
        for (i, filter) in filters.iter().enumerate().rev() {
            if filter_mask & (1 << i) != 0 {
                continue;
            }
            let output = if let Some(reg) = registry {
                reg.apply_with_limit(filter, data, element_size, max_output_len)?
            } else {
                apply_builtin_filter_with_limit(filter, data, element_size, max_output_len)?
            };
            validate_output_limit("filter pipeline", output.len(), max_output_len)?;
            return Ok(output);
        }
    }

    // Multi-filter pipeline: the first stage reads from the borrowed input
    // slice (avoiding a copy), subsequent stages consume the previous output.
    // Each filter stage necessarily allocates (output sizes are unpredictable),
    // but we avoid the initial data.to_vec() copy.
    let mut owned: Option<Vec<u8>> = None;

    for (i, filter) in filters.iter().enumerate().rev() {
        if filter_mask & (1 << i) != 0 {
            continue;
        }

        let input: &[u8] = match &owned {
            Some(buf) => buf,
            None => data,
        };

        // A decoded intermediate may still be compressed by an earlier filter.
        // Its bound is the forward encoded-size bound of that prefix, not the
        // final chunk size (e.g. two DEFLATE filters on an eight-byte chunk).
        let stage_limit = match max_output_len {
            Some(mut n) => {
                for (j, earlier) in filters[..i].iter().enumerate() {
                    if filter_mask & (1 << j) != 0 {
                        continue;
                    }
                    if registry.is_some_and(|reg| {
                        matches!(
                            reg.filters.get(&earlier.id),
                            Some(FilterImplementation::Custom(_))
                        )
                    }) {
                        return Err(Error::UnsupportedFilter(format!(
                            "custom filter {} has no qualified encoded-size bound",
                            earlier.id
                        )));
                    }
                    n = encoded_size_bound(earlier.id, n)?;
                }
                Some(n)
            }
            None => None,
        };
        owned = Some(if let Some(reg) = registry {
            reg.apply_with_limit(filter, input, element_size, stage_limit)?
        } else {
            apply_builtin_filter_with_limit(filter, input, element_size, stage_limit)?
        });
    }

    let output = owned.unwrap_or_else(|| data.to_vec());
    validate_output_limit("filter pipeline", output.len(), max_output_len)?;
    Ok(output)
}

/// Forward bounds for active built-in prefixes. Unknown/custom codecs and LZ4 framing
/// are deliberately unqualified here; single LZ4 decode still enforces its declared size.
fn encoded_size_bound(id: u16, n: usize) -> Result<usize> {
    Ok(match id {
        FILTER_DEFLATE => n
            .saturating_add(n >> 12)
            .saturating_add(n >> 14)
            .saturating_add(n >> 25)
            .saturating_add(13),
        FILTER_FLETCHER32 => n.saturating_add(4),
        // HDF5 scale-offset header plus at most one padding byte.
        FILTER_SCALEOFFSET => n.saturating_add(22),
        FILTER_SHUFFLE | FILTER_NBIT => n,
        _ => {
            return Err(Error::UnsupportedFilter(format!(
                "filter {id} has no qualified encoded-size bound in a bounded pipeline"
            )))
        }
    })
}

/// Absolute ceiling on one built-in filter's decoded output: one byte past
/// the largest chunk HDF5 permits, so the usual `expected + 1` limit still
/// detects over-long streams at the format maximum.
const MAX_FILTER_OUTPUT: usize = crate::hdf5_reader::HDF5_MAX_CHUNK_BYTES.saturating_add(1);

fn apply_builtin_filter_with_limit(
    filter: &FilterDescription,
    data: &[u8],
    element_size: usize,
    max_output_len: Option<usize>,
) -> Result<Vec<u8>> {
    // Built-in decoders size their output from file content, so every call is
    // bounded, including callers that pass no limit.
    let limit = max_output_len.map_or(MAX_FILTER_OUTPUT, |n| n.min(MAX_FILTER_OUTPUT));
    match filter.id {
        FILTER_DEFLATE => deflate::decompress_with_limit(data, limit),
        FILTER_SHUFFLE => Ok(shuffle::unshuffle(data, element_size)),
        FILTER_FLETCHER32 => fletcher32::verify_and_strip(data),
        FILTER_SZIP => Err(Error::UnsupportedFilter("szip".into())),
        FILTER_NBIT => nbit::decompress_with_limit(data, &filter.client_data, limit),
        FILTER_SCALEOFFSET => scaleoffset::decompress_with_limit(data, &filter.client_data, limit),
        #[cfg(feature = "hdf")]
        FILTER_LZ4 => lz4::decompress_with_limit(data, limit),
        id => Err(Error::UnsupportedFilter(format!("filter id {}", id))),
    }
}

/// A zero-filled filter output buffer, or an error when it cannot be allocated: a corrupted
/// chunk shape or size field must fail the read, not abort the process.
pub(crate) fn zeroed_output(len: usize, context: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.try_reserve_exact(len).map_err(|_| {
        Error::DecompressionError(format!("{context}: cannot allocate {len} output bytes"))
    })?;
    out.resize(len, 0);
    Ok(out)
}

fn validate_output_limit(context: &str, len: usize, max_output_len: Option<usize>) -> Result<()> {
    if let Some(max_output_len) = max_output_len {
        if len > max_output_len {
            return Err(Error::DecompressionError(format!(
                "{context} decoded to {len} bytes, limit {max_output_len}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    #[test]
    fn repeated_deflate_keeps_intermediate_zlib_wrapper() {
        // Exact compressed chunk from a HEC-RAS 6.x plan file (DSS Profile Time):
        // the dataset applies DEFLATE twice to an eight-byte chunk.
        let compressed = [
            0x78, 0x01, 0xab, 0x60, 0x4c, 0x4e, 0x60, 0x68, 0x66, 0x63, 0x60, 0xe8, 0x60, 0x68,
            0x04, 0x00, 0x12, 0xc7, 0x02, 0xcf,
        ];
        let filters = vec![
            FilterDescription {
                id: FILTER_DEFLATE,
                name: None,
                client_data: vec![1]
            };
            2
        ];
        let expected = (-0.0f64).to_le_bytes();
        for registry in [None, Some(FilterRegistry::new())] {
            assert_eq!(
                apply_pipeline_with_limit(&compressed, &filters, 0, 8, registry.as_ref(), Some(8))
                    .unwrap(),
                expected
            );
            // Masked first stage leaves one real DEFLATE stream and unchanged limit.
            let inner = deflate::decompress(&compressed).unwrap();
            assert_eq!(
                apply_pipeline_with_limit(&inner, &filters, 2, 8, registry.as_ref(), Some(8))
                    .unwrap(),
                expected
            );
        }
    }
    #[test]
    fn expanding_prefix_bounds_and_explicit_refusals() {
        assert_eq!(encoded_size_bound(FILTER_SCALEOFFSET, 256).unwrap(), 278);
        assert_eq!(
            encoded_size_bound(FILTER_SCALEOFFSET, usize::MAX).unwrap(),
            usize::MAX
        );
        let filter = |id| FilterDescription {
            id,
            name: None,
            client_data: vec![],
        };
        let mut registry = FilterRegistry::new();
        registry.register(
            50000,
            Box::new(|_, _, _| panic!("unbounded custom filter must not run")),
        );
        for filters in [
            vec![filter(50000)],
            vec![filter(50000), filter(FILTER_DEFLATE)],
            vec![filter(FILTER_LZ4), filter(FILTER_DEFLATE)],
        ] {
            assert!(matches!(
                apply_pipeline_with_limit(&[], &filters, 0, 4, Some(&registry), Some(256)),
                Err(Error::UnsupportedFilter(_))
            ));
        }
        // Even an override of a known builtin has no declared expansion bound.
        registry.register(
            FILTER_SHUFFLE,
            Box::new(|_, _, _| panic!("override must not run")),
        );
        assert!(matches!(
            apply_pipeline_with_limit(
                &[],
                &[filter(FILTER_SHUFFLE), filter(FILTER_DEFLATE)],
                0,
                4,
                Some(&registry),
                Some(256)
            ),
            Err(Error::UnsupportedFilter(_))
        ));
        // A masked unknown prefix imposes no bound and is never called.
        assert_eq!(
            apply_pipeline_with_limit(
                &[],
                &[filter(50000), filter(FILTER_SHUFFLE)],
                1,
                4,
                None,
                Some(0)
            )
            .unwrap(),
            Vec::<u8>::new()
        );
    }
    #[test]
    fn scaleoffset_then_deflate_matches_full_h5py_values() {
        let file = crate::hdf5_reader::Hdf5File::from_bytes(include_bytes!(
            "../../tests/fixtures/scaleoffset_deflate.h5"
        ))
        .unwrap();
        let values = file.dataset("values").unwrap().read_array::<i32>().unwrap();
        let expected: Vec<i32> = (0..64u32)
            .map(|i| i.wrapping_mul(0x9e3779b9) as i32)
            .collect();
        assert_eq!(values.as_slice().unwrap(), expected);
    }
    #[test]
    fn filter_registry_default() {
        let registry = FilterRegistry::new();
        // Built-in filters should be registered
        assert!(registry.filters.contains_key(&FILTER_DEFLATE));
        assert!(registry.filters.contains_key(&FILTER_SHUFFLE));
        assert!(registry.filters.contains_key(&FILTER_FLETCHER32));
        assert!(registry.filters.contains_key(&FILTER_NBIT));
        assert!(registry.filters.contains_key(&FILTER_SCALEOFFSET));
    }

    #[test]
    fn filter_registry_custom() {
        let mut registry = FilterRegistry::new();
        // Register a no-op custom filter
        registry.register(32000, Box::new(|_, data, _| Ok(data.to_vec())));
        let filter = FilterDescription {
            id: 32000,
            name: None,
            client_data: Vec::new(),
        };
        let result = registry.apply(&filter, &[1, 2, 3], 1).unwrap();
        assert_eq!(result, vec![1, 2, 3]);
    }

    #[test]
    fn filter_registry_unknown() {
        let registry = FilterRegistry::new();
        let filter = FilterDescription {
            id: 9999,
            name: None,
            client_data: Vec::new(),
        };
        let err = registry.apply(&filter, &[1, 2, 3], 1).unwrap_err();
        assert!(matches!(err, Error::UnsupportedFilter(_)));
    }

    #[test]
    fn apply_pipeline_with_limit_caps_registry_deflate_output() {
        let original = vec![0u8; 4096];
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&original).unwrap();
        let compressed = encoder.finish().unwrap();
        let filter = FilterDescription {
            id: FILTER_DEFLATE,
            name: None,
            client_data: vec![6],
        };
        let registry = FilterRegistry::new();

        let decoded =
            apply_pipeline_with_limit(&compressed, &[filter], 0, 1, Some(&registry), Some(65))
                .unwrap();

        assert_eq!(decoded.len(), 65);
        assert_eq!(decoded, original[..65]);
    }

    #[test]
    fn apply_pipeline_with_limit_rejects_oversized_final_output() {
        let filter = FilterDescription {
            id: FILTER_SHUFFLE,
            name: None,
            client_data: Vec::new(),
        };

        let err =
            apply_pipeline_with_limit(&[1, 2, 3, 4], &[filter], 0, 1, None, Some(3)).unwrap_err();

        assert!(err.to_string().contains("limit 3"));
    }
}
