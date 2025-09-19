//! Rust bindings for the tamp compression library.
//!
//! Provides safe, heapless streaming compression and decompression using const generics
//! for compile-time buffer allocation. Designed for embedded systems without heap allocation.
#![no_std]
#![deny(missing_docs)]

use tamp_sys::{
    TAMP_EXCESS_BITS, TAMP_INPUT_EXHAUSTED, TAMP_INVALID_CONF, TAMP_OK, TAMP_OUTPUT_FULL, tamp_res,
};

#[cfg(feature = "compressor")]
mod compressor;

#[cfg(feature = "compressor")]
pub use compressor::{Config, Compressor};

#[cfg(feature = "decompressor")]
mod decompressor;

#[cfg(feature = "decompressor")]
pub use decompressor::Decompressor;


/// Errors that can occur during compression or decompression.
#[derive(Debug)]
pub enum Error {
    /// Output buffer is full. Call again with a larger buffer or after consuming output.
    OutputFull,
    /// Input data exhausted. Normal condition for streaming decompression.
    InputExhausted,
    /// Invalid configuration parameter.
    InvalidConfig(&'static str),
    /// Symbol has more bits than configured literal size.
    ExcessBits,
    /// Heapless buffer cannot be resized to required size.
    BufferTooSmall,
}

impl Error {
    fn from_tamp_res(res: tamp_res) -> Result<(), Self> {
        match res {
            x if x == TAMP_OK as tamp_res => Ok(()),
            x if x == TAMP_OUTPUT_FULL as tamp_res => Err(Error::OutputFull),
            x if x == TAMP_INPUT_EXHAUSTED as tamp_res => Err(Error::InputExhausted),
            x if x == TAMP_EXCESS_BITS as tamp_res => Err(Error::ExcessBits),
            x if x == TAMP_INVALID_CONF as tamp_res => {
                Err(Error::InvalidConfig("Invalid parameters"))
            }
            _ => Err(Error::InvalidConfig("Unknown error")),
        }
    }
}

/// Compressor with 256-byte window (8-bit window). Minimal memory usage.
pub type Compressor256 = Compressor<256>;
/// Compressor with 512-byte window (9-bit window). Low memory usage.
pub type Compressor512 = Compressor<512>;
/// Compressor with 1KB window (10-bit window). Default and recommended for most uses.
pub type Compressor1K = Compressor<1024>;
/// Compressor with 2KB window (11-bit window). Better compression for larger data.
pub type Compressor2K = Compressor<2048>;
/// Compressor with 4KB window (12-bit window). Best compression but high memory usage.
pub type Compressor4K = Compressor<4096>;

/// Decompressor with 256-byte window (8-bit window). Minimal memory usage.
pub type Decompressor256 = Decompressor<256>;
/// Decompressor with 512-byte window (9-bit window). Low memory usage.
pub type Decompressor512 = Decompressor<512>;
/// Decompressor with 1KB window (10-bit window). Default and recommended for most uses.
pub type Decompressor1K = Decompressor<1024>;
/// Decompressor with 2KB window (11-bit window). Better compression for larger data.
pub type Decompressor2K = Decompressor<2048>;
/// Decompressor with 4KB window (12-bit window). Best compression but high memory usage.
pub type Decompressor4K = Decompressor<4096>;

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::borrow::ToOwned;
    use std::format;

    #[test]
    fn test_corpus() {
        test_compress_decompress_canterbury_corpus::<256>(Config::new().window_bits(8).unwrap());
        test_compress_decompress_canterbury_corpus::<512>(Config::new().window_bits(9).unwrap());
        test_compress_decompress_canterbury_corpus::<1024>(Config::new());
        test_compress_decompress_canterbury_corpus::<2048>(Config::new().window_bits(11).unwrap());
        test_compress_decompress_canterbury_corpus::<4096>(Config::new().window_bits(12).unwrap());
    }

    fn test_compress_decompress_canterbury_corpus<const N: usize>(config: Config) {
        use std::fs::{File, OpenOptions, create_dir_all, read_dir, remove_file};
        use std::io::{BufReader, BufWriter, Read, Write};
        use std::path::Path;
        use std::{println, vec};

        let canterbury_dir = Path::new("fixtures/canterbury-corpus/canterbury");
        let target_dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_owned());
        let target_path = Path::new(&target_dir);
        create_dir_all(target_path).expect("Failed to create target directory");

        // Iterate through all files in Canterbury corpus
        for entry in read_dir(canterbury_dir).expect("Failed to read Canterbury corpus directory") {
            let entry = entry.expect("Failed to read directory entry");
            let path = entry.path();

            // Skip SHA1SUM file and directories
            if !path.is_file() || path.file_name().unwrap() == "SHA1SUM" {
                continue;
            }

            println!(
                "Testing compression {} on: {:?}",
                N,
                path.file_name().unwrap()
            );

            // Open file for reading in chunks
            let mut file =
                File::open(&path).unwrap_or_else(|_| panic!("Failed to open file: {:?}", path));
            // Create compressed output file in target directory
            let compressed_filename = format!(
                "test_compressed_{}_{}.tamp",
                path.file_name().unwrap().to_string_lossy(),
                N
            );
            let compressed_path = target_path.join(compressed_filename);
            let compressed_file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&compressed_path)
                .unwrap_or_else(|_| {
                    panic!("Failed to create compressed file: {:?}", compressed_path)
                });
            let mut compressed_writer = BufWriter::new(compressed_file);
            let mut compressor = Compressor::<N>::new(config.clone()).unwrap();

            // Compress file in 1024-byte chunks
            let mut total_compressed_size = 0;
            let mut chunk_buffer = [0u8; 1024];
            let mut output_buffer = [0u8; 2048];
            let mut input_data = vec![]; // Keep track of original data for verification

            loop {
                let bytes_read = file.read(&mut chunk_buffer).unwrap();
                if bytes_read == 0 {
                    break; // End of file
                }

                let chunk = &chunk_buffer[..bytes_read];
                input_data.extend_from_slice(chunk);

                // Compress this chunk
                let mut chunk_offset = 0;
                while chunk_offset < bytes_read {
                    let (consumed, written) = compressor
                        .compress_chunk(&chunk[chunk_offset..], &mut output_buffer)
                        .unwrap();

                    if written > 0 {
                        compressed_writer
                            .write_all(&output_buffer[..written])
                            .unwrap();
                        total_compressed_size += written;
                    }

                    chunk_offset += consumed;

                    if consumed == 0 {
                        panic!("Output buffer too small for compression");
                    }
                }
            }

            // Flush any remaining data
            let flush_written = compressor.flush(&mut output_buffer, false).unwrap();
            if flush_written > 0 {
                compressed_writer
                    .write_all(&output_buffer[..flush_written])
                    .unwrap();
                total_compressed_size += flush_written;
            }
            compressed_writer.flush().unwrap();
            drop(compressed_writer);

            assert!(
                total_compressed_size > 0,
                "No output written for {:?}",
                path.file_name()
            );

            // Verify compression actually occurred for most files (some very small files might not compress)
            if input_data.len() > 100 {
                assert!(
                    total_compressed_size < input_data.len(),
                    "No compression achieved for {:?}",
                    path.file_name()
                );
            }

            // Decompress - first read header from file to get proper configuration
            let compressed_file = File::open(&compressed_path).unwrap_or_else(|_| {
                panic!("Failed to open compressed file: {:?}", compressed_path)
            });
            let mut compressed_reader = BufReader::new(compressed_file);

            // Read initial chunk to get header
            let mut header_buffer = [0u8; 64];
            compressed_reader.read_exact(&mut header_buffer).unwrap();
            let (mut decompressor, header_consumed) =
                Decompressor::<N>::from_header(&header_buffer).unwrap();

            let mut decompressed = vec![0u8; input_data.len() + 100];
            let mut decompressed_offset = 0;
            let mut compressed_input_buffer = [0u8; 1024];

            // Position reader after header
            let mut compressed_reader = BufReader::new(File::open(&compressed_path).unwrap());
            compressed_reader
                .read_exact(&mut vec![0u8; header_consumed])
                .unwrap();

            // Decompress in chunks from file
            loop {
                let bytes_read = compressed_reader
                    .read(&mut compressed_input_buffer)
                    .unwrap();
                if bytes_read == 0 {
                    break;
                }

                let mut input_offset = 0;
                while input_offset < bytes_read && decompressed_offset < input_data.len() {
                    let (consumed_decomp, written_decomp) = decompressor
                        .decompress_chunk(
                            &compressed_input_buffer[input_offset..bytes_read],
                            &mut decompressed[decompressed_offset..],
                        )
                        .unwrap();

                    if consumed_decomp == 0 && written_decomp == 0 {
                        break;
                    }

                    input_offset += consumed_decomp;
                    decompressed_offset += written_decomp;
                }

                if decompressed_offset >= input_data.len() {
                    break;
                }
            }

            // Verify the data was decompressed correctly
            assert_eq!(
                decompressed_offset,
                input_data.len(),
                "Decompressed size mismatch for {:?}",
                path.file_name()
            );
            assert_eq!(
                &decompressed[..decompressed_offset],
                &input_data[..],
                "Data corruption detected for {:?}",
                path.file_name()
            );

            println!(
                "✓ {:?}: {} -> {} bytes ({:.1}% compression)",
                path.file_name().unwrap(),
                input_data.len(),
                total_compressed_size,
                100.0 * (1.0 - total_compressed_size as f64 / input_data.len() as f64)
            );

            // Clean up temporary compressed file
            let _ = remove_file(&compressed_path);
        }
    }
}
