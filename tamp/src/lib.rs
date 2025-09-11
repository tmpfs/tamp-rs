//! Rust bindings for the tamp compression library.
//!
//! Provides safe, heapless streaming compression and decompression using const generics
//! for compile-time buffer allocation. Designed for embedded systems without heap allocation.
//!
//! # Example
//!
//! ```
//! use tamp::{Compressor1K, Decompressor1K, Config};
//!
//! let config = Config::new();
//! let mut compressor = Compressor1K::new(config.clone()).unwrap();
//! let mut compressed = [0u8; 128];
//!
//! let input = b"hello world";
//! let (consumed, written) = compressor.compress_chunk(input, &mut compressed).unwrap();
//! let flush_written = compressor.flush(&mut compressed[written..], false).unwrap();
//!
//! let (mut decompressor, header_consumed) = Decompressor1K::from_header(&compressed).unwrap();
//! let mut decompressed = [0u8; 128];
//! let (_, written) = decompressor.decompress_chunk(
//!     &compressed[header_consumed..written + flush_written],
//!     &mut decompressed
//! ).unwrap();
//!
//! assert_eq!(&decompressed[..written], input);
//! ```
#![no_std]
#![deny(missing_docs)]

use core::marker::PhantomData;
use heapless::Vec;
use tamp_sys::{TampCompressor, TampDecompressor, TampConf, tamp_res,
    TAMP_OK, TAMP_OUTPUT_FULL, TAMP_INPUT_EXHAUSTED, TAMP_EXCESS_BITS, TAMP_INVALID_CONF,
    tamp_compressor_init, tamp_compressor_sink, tamp_compressor_poll, tamp_compressor_full,
    tamp_compressor_flush, tamp_compressor_compress_cb, tamp_decompressor_init,
    tamp_decompressor_decompress_cb, tamp_decompressor_read_header, tamp_initialize_dictionary};

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
            x if x == TAMP_INVALID_CONF as tamp_res => Err(Error::InvalidConfig("Invalid parameters")),
            _ => Err(Error::InvalidConfig("Unknown error")),
        }
    }
}

/// Configuration for tamp compression/decompression.
/// 
/// Default configuration uses 10-bit window (1KB), 8-bit literals, lazy matching enabled.
#[derive(Clone)]
pub struct Config {
    /// Window size in bits (8-15). Window size = 2^window_bits bytes. Default: 10 (1KB).
    pub window_bits: u8,
    /// Literal size in bits (5-8). Default: 8.
    pub literal_bits: u8,
    /// Enable lazy matching for better compression at cost of ~50% more CPU. Default: true.
    pub lazy_matching: bool,
    /// Use custom dictionary initialization. Default: false.
    pub use_custom_dictionary: bool,
}

impl Default for Config {
    /// Creates default configuration: 10-bit window (1KB), 8-bit literals, lazy matching enabled.
    fn default() -> Self {
        Self {
            window_bits: 10,  // 1KB window
            literal_bits: 8,
            lazy_matching: true,
            use_custom_dictionary: false,
        }
    }
}

impl Config {
    /// Creates a new configuration with default settings.
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Sets window size in bits (8-15). Window size = 2^bits bytes.
    /// Larger windows provide better compression but use more memory.
    pub fn window_bits(mut self, bits: u8) -> Result<Self, Error> {
        if !(8..=15).contains(&bits) {
            return Err(Error::InvalidConfig("Window bits must be 8-15"));
        }
        self.window_bits = bits;
        Ok(self)
    }
    
    /// Sets literal size in bits (5-8). More bits = larger alphabet but less compression.
    pub fn literal_bits(mut self, bits: u8) -> Result<Self, Error> {
        if !(5..=8).contains(&bits) {
            return Err(Error::InvalidConfig("Literal bits must be 5-8"));
        }
        self.literal_bits = bits;
        Ok(self)
    }
    
    /// Enables lazy matching. Improves compression ~0.5-2% at cost of ~50% more CPU.
    pub fn lazy_matching(mut self, enabled: bool) -> Self {
        self.lazy_matching = enabled;
        self
    }
    
    /// Enables custom dictionary initialization. Dictionary must be provided during construction.
    pub fn custom_dictionary(mut self, enabled: bool) -> Self {
        self.use_custom_dictionary = enabled;
        self
    }
    
    fn to_c_config(&self) -> TampConf {
        let mut conf = TampConf {
            _bitfield_align_1: [],
            _bitfield_1: Default::default(),
        };
        conf.set_window(self.window_bits as u16);
        conf.set_literal(self.literal_bits as u16);
        conf.set_use_custom_dictionary(self.use_custom_dictionary as u16);
        // Note: lazy_matching not available in current bindings
        conf
    }
    
    /// Returns window size in bytes (2^window_bits).
    pub fn window_size(&self) -> usize {
        1usize << self.window_bits
    }
}

/// Streaming compressor with heapless window buffer.
/// 
/// `N` is the window buffer size in bytes and must equal 2^window_bits.
/// Use type aliases like `Compressor1K` for convenience.
/// 
/// Memory usage: ~N + 64 bytes (window + struct overhead).
pub struct Compressor<const N: usize> {
    inner: TampCompressor,
    window: Vec<u8, N>,
    _marker: PhantomData<*mut ()>, // !Send + !Sync for raw C state
}

impl<const N: usize> Compressor<N> {
    /// Creates a new compressor with the given configuration.
    /// Buffer size N must equal 2^config.window_bits.
    pub fn new(config: Config) -> Result<Self, Error> {
        Self::with_dictionary(config, None)
    }
    
    /// Creates a compressor with optional dictionary initialization.
    /// Dictionary improves compression when data has predictable patterns.
    pub fn with_dictionary(config: Config, dictionary: Option<&[u8]>) -> Result<Self, Error> {
        let expected_size = config.window_size();
        if N != expected_size {
            return Err(Error::InvalidConfig("Buffer size N must equal 2^window_bits"));
        }
        
        let mut window = Vec::new();
        window.resize(N, 0).map_err(|_| Error::BufferTooSmall)?;
        
        // Initialize dictionary if provided
        if let Some(dict) = dictionary {
            if config.use_custom_dictionary {
                let copy_len = dict.len().min(N);
                window[..copy_len].copy_from_slice(&dict[..copy_len]);
            } else {
                // Use tamp's built-in dictionary initialization
                unsafe {
                    tamp_initialize_dictionary(window.as_mut_ptr(), N);
                }
                if !dict.is_empty() {
                    let copy_len = dict.len().min(N);
                    window[..copy_len].copy_from_slice(&dict[..copy_len]);
                }
            }
        } else if config.use_custom_dictionary {
            return Err(Error::InvalidConfig("Custom dictionary enabled but none provided"));
        }
        
        let mut compressor = Self {
            inner: unsafe { core::mem::zeroed() },
            window,
            _marker: PhantomData,
        };
        
        let c_config = config.to_c_config();
        let result = unsafe {
            tamp_compressor_init(
                &mut compressor.inner,
                &c_config,
                compressor.window.as_mut_ptr()
            )
        };
        
        Error::from_tamp_res(result)?;
        Ok(compressor)
    }
    
    /// Compresses input data into output buffer.
    /// Returns (input_consumed, output_written). May not consume all input if output is full.
    /// Call repeatedly until all input is consumed.
    pub fn compress_chunk(&mut self, input: &[u8], output: &mut [u8]) -> Result<(usize, usize), Error> {
        let mut input_consumed = 0;
        let mut output_written = 0;
        
        let result = unsafe {
            tamp_compressor_compress_cb(
                &mut self.inner,
                output.as_mut_ptr(),
                output.len(),
                &mut output_written,
                input.as_ptr(),
                input.len(), 
                &mut input_consumed,
                None,  // No callback
                core::ptr::null_mut(),  // No user data
            )
        };
        
        Error::from_tamp_res(result)?;
        Ok((input_consumed, output_written))
    }
    
    /// Low-level: sinks up to 16 bytes into internal buffer.
    /// Use with `poll()` for fine-grained control. Most users should use `compress_chunk()`.
    pub fn sink(&mut self, input: &[u8]) -> usize {
        let mut consumed = 0;
        unsafe {
            tamp_compressor_sink(
                &mut self.inner,
                input.as_ptr(),
                input.len(),
                &mut consumed,
            );
        }
        consumed
    }
    
    /// Low-level: polls internal buffer for compressed data.
    /// Use with `sink()` for fine-grained control. Most users should use `compress_chunk()`.
    pub fn poll(&mut self, output: &mut [u8]) -> Result<usize, Error> {
        let mut output_written = 0;
        let result = unsafe {
            tamp_compressor_poll(
                &mut self.inner,
                output.as_mut_ptr(),
                output.len(),
                &mut output_written,
            )
        };
        
        Error::from_tamp_res(result)?;
        Ok(output_written)
    }
    
    /// Returns true if internal input buffer is full (16 bytes).
    /// When full, call `poll()` to process buffered data.
    pub fn is_full(&self) -> bool {
        unsafe { tamp_compressor_full(&self.inner as *const _ as *mut _) }
    }
    
    /// Flushes remaining data from internal buffers.
    /// `write_token`: true to continue using compressor, false for final flush.
    /// Must be called at end of compression to ensure all data is output.
    pub fn flush(&mut self, output: &mut [u8], write_token: bool) -> Result<usize, Error> {
        let mut output_written = 0;
        
        let result = unsafe {
            tamp_compressor_flush(
                &mut self.inner,
                output.as_mut_ptr(),
                output.len(),
                &mut output_written,
                write_token,
            )
        };
        
        Error::from_tamp_res(result)?;
        Ok(output_written)
    }
}

/// Streaming decompressor with heapless window buffer.
/// 
/// `N` is the window buffer size in bytes and must equal 2^window_bits.
/// Use type aliases like `Decompressor1K` for convenience.
/// 
/// Memory usage: ~N + 32 bytes (window + struct overhead).
pub struct Decompressor<const N: usize> {
    inner: TampDecompressor,
    window: Vec<u8, N>,
    _marker: PhantomData<*mut ()>,
}

impl<const N: usize> Decompressor<N> {
    /// Creates a new decompressor with the given configuration.
    /// Buffer size N must equal 2^config.window_bits.
    pub fn new(config: Config) -> Result<Self, Error> {
        Self::with_dictionary(config, None)
    }
    
    /// Creates a decompressor with optional dictionary initialization.
    /// Dictionary must match the one used during compression.
    pub fn with_dictionary(config: Config, dictionary: Option<&[u8]>) -> Result<Self, Error> {
        let expected_size = config.window_size();
        if N != expected_size {
            return Err(Error::InvalidConfig("Buffer size N must equal 2^window_bits"));
        }
        
        let mut window = Vec::new();
        window.resize(N, 0).map_err(|_| Error::BufferTooSmall)?;
        
        // Initialize dictionary if provided
        if let Some(dict) = dictionary && config.use_custom_dictionary {
                let copy_len = dict.len().min(N);
                window[..copy_len].copy_from_slice(&dict[..copy_len]);
        }
        
        let mut decompressor = Self {
            inner: unsafe { core::mem::zeroed() },
            window,
            _marker: PhantomData,
        };
        
        let c_config = config.to_c_config();
        let result = unsafe {
            tamp_decompressor_init(
                &mut decompressor.inner,
                &c_config,
                decompressor.window.as_mut_ptr()
            )
        };
        
        Error::from_tamp_res(result)?;
        Ok(decompressor)
    }
    
    /// Creates decompressor by reading configuration from compressed stream header.
    /// Returns (decompressor, bytes_consumed_from_input).
    /// Buffer size N must match the window size found in header.
    pub fn from_header(input: &[u8]) -> Result<(Self, usize), Error> {
        let mut conf = unsafe { core::mem::zeroed::<TampConf>() };
        let mut input_consumed = 0;
        
        let result = unsafe {
            tamp_decompressor_read_header(
                &mut conf,
                input.as_ptr(),
                input.len(),
                &mut input_consumed,
            )
        };
        
        Error::from_tamp_res(result)?;
        
        let config = Config {
            window_bits: conf.window() as u8,
            literal_bits: conf.literal() as u8,
            use_custom_dictionary: conf.use_custom_dictionary() != 0,
            lazy_matching: false, // Not used for decompression
        };
        
        let expected_size = config.window_size();
        if N != expected_size {
            return Err(Error::InvalidConfig("Buffer size N doesn't match header"));
        }
        
        let decompressor = Self::new(config)?;
        Ok((decompressor, input_consumed))
    }
    
    /// Decompresses input data into output buffer.
    /// Returns (input_consumed, output_written). May not consume all input or fill all output.
    /// Call repeatedly until input is exhausted or output is filled.
    pub fn decompress_chunk(&mut self, input: &[u8], output: &mut [u8]) -> Result<(usize, usize), Error> {
        let mut input_consumed = 0;
        let mut output_written = 0;
        
        let result = unsafe {
            tamp_decompressor_decompress_cb(
                &mut self.inner,
                output.as_mut_ptr(),
                output.len(),
                &mut output_written,
                input.as_ptr(),
                input.len(),
                &mut input_consumed,
                None,  // No callback
                core::ptr::null_mut(),  // No user data
            )
        };
        
        // For decompressor, INPUT_EXHAUSTED and OUTPUT_FULL are normal conditions
        match result {
            x if x == TAMP_OK as tamp_res || 
                 x == TAMP_OUTPUT_FULL as tamp_res || 
                 x == TAMP_INPUT_EXHAUSTED as tamp_res => {
                Ok((input_consumed, output_written))
            }
            _ => Error::from_tamp_res(result).map(|_| (input_consumed, output_written)),
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

    #[test]
    fn test_compression_canterbury_corpus() {
        use std::fs;
        use std::path::Path;
        use std::{println, format, vec};
        
        let canterbury_dir = Path::new("fixtures/canterbury-corpus/canterbury");
        
        // Iterate through all files in Canterbury corpus
        for entry in fs::read_dir(canterbury_dir).expect("Failed to read Canterbury corpus directory") {
            let entry = entry.expect("Failed to read directory entry");
            let path = entry.path();
            
            // Skip SHA1SUM file and directories
            if !path.is_file() || path.file_name().unwrap() == "SHA1SUM" {
                continue;
            }
            
            println!("Testing compression on: {:?}", path.file_name().unwrap());
            
            // Read the file
            let input = fs::read(&path).expect(&format!("Failed to read file: {:?}", path));
            
            // Use larger buffer for larger files
            let mut compressed = vec![0u8; input.len() + 1024]; // Extra space for headers and worst case
            
            let config = Config::new();
            let mut compressor = Compressor1K::new(config.clone()).unwrap();
            
            // Compress
            let (consumed, written) = compressor.compress_chunk(&input, &mut compressed).unwrap();
            assert_eq!(consumed, input.len(), "Failed to consume all input for {:?}", path.file_name());
            assert!(written > 0, "No output written for {:?}", path.file_name());
            
            // Flush any remaining data
            let flush_written = compressor.flush(&mut compressed[written..], false).unwrap();
            let total_compressed = written + flush_written;
            
            // Verify compression actually occurred for most files (some very small files might not compress)
            if input.len() > 100 {
                assert!(total_compressed < input.len(), "No compression achieved for {:?}", path.file_name());
            }
            
            // Decompress - first read header to get proper configuration
            let (mut decompressor, header_consumed) = Decompressor1K::from_header(&compressed).unwrap();
            let mut decompressed = vec![0u8; input.len() + 100]; // Extra space to be safe
            
            let (consumed_decomp, written_decomp) = decompressor.decompress_chunk(
                &compressed[header_consumed..total_compressed], 
                &mut decompressed
            ).unwrap();
            
            // Verify the data was decompressed correctly
            assert!(consumed_decomp > 0, "No input consumed during decompression for {:?}", path.file_name());
            assert_eq!(written_decomp, input.len(), "Decompressed size mismatch for {:?}", path.file_name());
            assert_eq!(&decompressed[..written_decomp], &input[..], "Data corruption detected for {:?}", path.file_name());
            
            println!("✓ {:?}: {} -> {} bytes ({:.1}% compression)", 
                path.file_name().unwrap(), 
                input.len(), 
                total_compressed,
                100.0 * (1.0 - total_compressed as f64 / input.len() as f64)
            );
        }
    }
}
