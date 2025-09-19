use core::marker::PhantomData;
use heapless::Vec;
use tamp_sys::{
    TAMP_EXCESS_BITS, TAMP_INPUT_EXHAUSTED, TAMP_INVALID_CONF, TAMP_OK, TAMP_OUTPUT_FULL,
    TampCompressor, TampConf, TampDecompressor, tamp_compressor_compress_cb, tamp_compressor_flush,
    tamp_compressor_full, tamp_compressor_init, tamp_compressor_poll, tamp_compressor_sink,
    tamp_decompressor_decompress_cb, tamp_decompressor_init, tamp_decompressor_read_header,
    tamp_initialize_dictionary, tamp_res,
};
use crate::{Error, Config};

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
            return Err(Error::InvalidConfig(
                "Buffer size N must equal 2^window_bits",
            ));
        }

        let mut window = Vec::new();
        window.resize(N, 0).map_err(|_| Error::BufferTooSmall)?;

        // Initialize dictionary if provided
        if let Some(dict) = dictionary
            && config.use_custom_dictionary
        {
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
                decompressor.window.as_mut_ptr(),
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
    pub fn decompress_chunk(
        &mut self,
        input: &[u8],
        output: &mut [u8],
    ) -> Result<(usize, usize), Error> {
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
                None,                  // No callback
                core::ptr::null_mut(), // No user data
            )
        };

        // For decompressor, INPUT_EXHAUSTED and OUTPUT_FULL are normal conditions
        match result {
            x if x == TAMP_OK as tamp_res
                || x == TAMP_OUTPUT_FULL as tamp_res
                || x == TAMP_INPUT_EXHAUSTED as tamp_res =>
            {
                Ok((input_consumed, output_written))
            }
            _ => Error::from_tamp_res(result).map(|_| (input_consumed, output_written)),
        }
    }
}
