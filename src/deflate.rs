use std::io::{Read, Write};
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;

/// Decompresses raw DEFLATE bytes (no zlib or gzip header).
pub fn decompress(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let mut decoder = DeflateDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

/// Compresses bytes using raw DEFLATE (level 6, dynamic strategy).
pub fn compress(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::new(6));
    encoder.write_all(data)?;
    encoder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_deflate_roundtrip() {
        let sample = b"VisualSource raw DEFLATE test with repeated repetition repetition data!";
        let compressed = compress(sample).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(&decompressed[..], sample);
    }
}
