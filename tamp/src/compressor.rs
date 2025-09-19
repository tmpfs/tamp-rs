use core::marker::PhantomData;
use heapless::Vec;
use tamp_sys::{
    TAMP_EXCESS_BITS, TAMP_INPUT_EXHAUSTED, TAMP_INVALID_CONF, TAMP_OK, TAMP_OUTPUT_FULL,
    TampCompressor, TampConf, TampDecompressor, tamp_compressor_compress_cb, tamp_compressor_flush,
    tamp_compressor_full, tamp_compressor_init, tamp_compressor_poll, tamp_compressor_sink,
    tamp_decompressor_decompress_cb, tamp_decompressor_init, tamp_decompressor_read_header,
    tamp_initialize_dictionary, tamp_res,
};
use crate::Error;

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
            window_bits: 10, // 1KB window
            literal_bits: 8,
            lazy_matching: false,
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

    pub(crate) fn to_c_config(&self) -> TampConf {
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
            return Err(Error::InvalidConfig(
                "Buffer size N must equal 2^window_bits",
            ));
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
            return Err(Error::InvalidConfig(
                "Custom dictionary enabled but none provided",
            ));
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
                compressor.window.as_mut_ptr(),
            )
        };

        Error::from_tamp_res(result)?;
        Ok(compressor)
    }

    /// Compresses input data into output buffer.
    /// Returns (input_consumed, output_written). May not consume all input if output is full.
    /// Call repeatedly until all input is consumed.
    pub fn compress_chunk(
        &mut self,
        input: &[u8],
        output: &mut [u8],
    ) -> Result<(usize, usize), Error> {
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
                None,                  // No callback
                core::ptr::null_mut(), // No user data
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
            tamp_compressor_sink(&mut self.inner, input.as_ptr(), input.len(), &mut consumed);
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

