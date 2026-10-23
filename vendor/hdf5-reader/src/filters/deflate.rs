use crate::error::{Error, Result};
use flate2::read::ZlibDecoder;
use flate2::{Decompress, FlushDecompress, Status};
use std::io::Read;

/// Decompress DEFLATE/zlib-compressed data.
pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    decompress_inner(data, None)
}

/// Decompress DEFLATE/zlib-compressed data, reading at most `max_output_len`
/// bytes from the decoded stream.
pub fn decompress_with_limit(data: &[u8], max_output_len: usize) -> Result<Vec<u8>> {
    decompress_inner(data, Some(max_output_len))
}

fn decompress_inner(data: &[u8], max_output_len: Option<usize>) -> Result<Vec<u8>> {
    if let Some(max_output_len) = max_output_len {
        return decompress_bounded(data, max_output_len);
    }

    let mut decoder = ZlibDecoder::new(data);
    let mut output = Vec::new();
    decoder
        .read_to_end(&mut output)
        .map_err(|e| Error::DecompressionError(format!("DEFLATE decompression failed: {e}")))?;
    Ok(output)
}

/// First output buffer for a bounded decode. The buffer grows as the stream
/// actually produces data, so a declared size alone never allocates.
const INITIAL_OUTPUT: usize = 64 * 1024;

fn decompress_bounded(data: &[u8], max_output_len: usize) -> Result<Vec<u8>> {
    let mut decoder = Decompress::new(true);
    let initial = max_output_len.min(data.len().saturating_mul(4).max(INITIAL_OUTPUT));
    let mut output = Vec::with_capacity(initial);
    loop {
        let consumed = usize::try_from(decoder.total_in()).unwrap_or(usize::MAX);
        let input = data.get(consumed..).unwrap_or_default();
        let produced = output.len();
        let status = decoder
            .decompress_vec(input, &mut output, FlushDecompress::None)
            .map_err(|e| Error::DecompressionError(format!("DEFLATE decompression failed: {e}")))?;

        if matches!(status, Status::StreamEnd) || output.len() >= max_output_len {
            output.truncate(max_output_len);
            return Ok(output);
        }
        let used_input = usize::try_from(decoder.total_in()).unwrap_or(usize::MAX) > consumed;
        if output.len() < output.capacity() {
            // Room left but no stream end: continue only while input is consumed.
            if (used_input || output.len() > produced) && decoder.total_in() < data.len() as u64 {
                continue;
            }
            return Err(Error::DecompressionError(
                "DEFLATE decompression failed: stream did not finish".into(),
            ));
        }
        let grow = output.len().max(INITIAL_OUTPUT);
        output.reserve_exact(grow.min(max_output_len - output.len()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    #[test]
    fn roundtrip() {
        let original = b"hello world, this is a test of zlib compression!";
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original).unwrap();
        let compressed = encoder.finish().unwrap();

        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, original);
    }

    #[test]
    fn decompress_with_limit_stops_at_limit() {
        let original = vec![0x5a; 4096];
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&original).unwrap();
        let compressed = encoder.finish().unwrap();

        let decompressed = decompress_with_limit(&compressed, 17).unwrap();
        assert_eq!(decompressed.len(), 17);
        assert_eq!(decompressed, original[..17]);
    }

    #[test]
    fn bounded_decode_grows_past_the_initial_buffer() {
        let original: Vec<u8> = (0..1_000_000u32).map(|i| (i % 251) as u8).collect();
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&original).unwrap();
        let compressed = encoder.finish().unwrap();

        let decompressed = decompress_with_limit(&compressed, original.len() + 1).unwrap();
        assert_eq!(decompressed, original);
    }

    #[test]
    fn huge_limit_does_not_preallocate() {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&[7u8; 100]).unwrap();
        let compressed = encoder.finish().unwrap();

        // A limit of 1 TiB must not be allocated up front.
        let decompressed = decompress_with_limit(&compressed, 1 << 40).unwrap();
        assert_eq!(decompressed, vec![7u8; 100]);
        assert!(decompressed.capacity() < 1 << 20);
    }
}
