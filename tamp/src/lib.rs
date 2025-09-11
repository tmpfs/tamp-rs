#![no_std]

use core::marker::PhantomData;
use heapless::Vec;
use tamp_sys::{TampCompressor, TampDecompressor, TampConf, tamp_res,
    TAMP_OK, TAMP_OUTPUT_FULL, TAMP_INPUT_EXHAUSTED, TAMP_EXCESS_BITS, TAMP_INVALID_CONF,
    tamp_compressor_init, tamp_compressor_sink, tamp_compressor_poll, tamp_compressor_full,
    tamp_compressor_flush, tamp_compressor_compress_cb, tamp_decompressor_init,
    tamp_decompressor_decompress_cb, tamp_decompressor_read_header, tamp_initialize_dictionary};

#[derive(Debug)]
pub enum Error {
    OutputFull,
    InputExhausted,
    InvalidConfig(&'static str),
    ExcessBits,
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

#[derive(Clone)]
pub struct Config {
    pub window_bits: u8,    // 8-15, default 10 (1KB window)
    pub literal_bits: u8,   // 5-8, default 8
    pub lazy_matching: bool, // default true
    pub use_custom_dictionary: bool,
}

impl Default for Config {
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
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn window_bits(mut self, bits: u8) -> Result<Self, Error> {
        if !(8..=15).contains(&bits) {
            return Err(Error::InvalidConfig("Window bits must be 8-15"));
        }
        self.window_bits = bits;
        Ok(self)
    }
    
    pub fn literal_bits(mut self, bits: u8) -> Result<Self, Error> {
        if !(5..=8).contains(&bits) {
            return Err(Error::InvalidConfig("Literal bits must be 5-8"));
        }
        self.literal_bits = bits;
        Ok(self)
    }
    
    pub fn lazy_matching(mut self, enabled: bool) -> Self {
        self.lazy_matching = enabled;
        self
    }
    
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
    
    pub fn window_size(&self) -> usize {
        1usize << self.window_bits
    }
}

/// Streaming compressor with heapless buffers
/// N is the window buffer size in bytes - must be 2^window_bits
pub struct Compressor<const N: usize> {
    inner: TampCompressor,
    window: Vec<u8, N>,
    _marker: PhantomData<*mut ()>, // !Send + !Sync for raw C state
}

impl<const N: usize> Compressor<N> {
    pub fn new(config: Config) -> Result<Self, Error> {
        Self::with_dictionary(config, None)
    }
    
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
    
    /// Compress a chunk of input data into output buffer
    /// Returns (input_consumed, output_written)
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
    
    /// Sink data into internal buffer (up to 16 bytes)
    /// Returns number of bytes consumed from input
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
    
    /// Poll internal buffer for compressed output
    /// Returns bytes written to output
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
    
    /// Check if internal input buffer is full
    pub fn is_full(&self) -> bool {
        unsafe { tamp_compressor_full(&self.inner as *const _ as *mut _) }
    }
    
    /// Flush remaining data and finalize compression
    /// write_token: true to continue using compressor, false for final flush
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

/// Streaming decompressor with heapless buffers  
/// N is the window buffer size in bytes - must be 2^window_bits
pub struct Decompressor<const N: usize> {
    inner: TampDecompressor,
    window: Vec<u8, N>,
    _marker: PhantomData<*mut ()>,
}

impl<const N: usize> Decompressor<N> {
    pub fn new(config: Config) -> Result<Self, Error> {
        Self::with_dictionary(config, None)
    }
    
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
    
    /// Create decompressor by reading header from compressed stream
    /// Returns (decompressor, bytes_consumed_from_input)
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
    
    /// Decompress input data into output buffer
    /// Returns (input_consumed, output_written)
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

// Type aliases for common configurations
pub type Compressor256 = Compressor<256>;      // 8-bit window (256 bytes)
pub type Compressor512 = Compressor<512>;      // 9-bit window (512 bytes)  
pub type Compressor1K = Compressor<1024>;      // 10-bit window (1KB) - default
pub type Compressor2K = Compressor<2048>;      // 11-bit window (2KB)
pub type Compressor4K = Compressor<4096>;      // 12-bit window (4KB)

pub type Decompressor256 = Decompressor<256>;
pub type Decompressor512 = Decompressor<512>;
pub type Decompressor1K = Decompressor<1024>;
pub type Decompressor2K = Decompressor<2048>;
pub type Decompressor4K = Decompressor<4096>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compression_basic() {
        let config = Config::new();
        let mut compressor = Compressor1K::new(config).unwrap();
        
        let input = b"hello world hello world hello world";
        let mut output = [0u8; 128];
        
        let (consumed, written) = compressor.compress_chunk(input, &mut output).unwrap();
        
        assert!(consumed > 0);
        assert!(written > 0);
        assert!(written < input.len()); // Should compress
    }
}
