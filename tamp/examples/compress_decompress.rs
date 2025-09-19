use tamp::{Compressor1K, Decompressor1K, Config};

fn main() {
    let config = Config::new();
    let mut compressor = Compressor1K::new(config.clone()).unwrap();
    let mut compressed = [0u8; 128];

    let input = b"Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed volutpat odio eget dolor aliquam, eu consequat magna viverra. Proin at pretium nulla, sed sagittis lorem. Suspendisse potenti. Fusce tempor ex non mauris scelerisque, vestibulum porta metus pretium. Nunc pharetra dapibus elit, sed blandit nisl sodales ut. Cras est massa, porttitor in mattis et, convallis vitae orci. Donec ac blandit justo. Donec porttitor dui nec congue condimentum. Vivamus aliquet est diam, sed bibendum turpis commodo nec. Nulla ut euismod dui. Vestibulum feugiat risus dui, in lacinia nulla euismod id. Duis sed maximus quam, in malesuada nulla. Praesent malesuada elementum erat eleifend ornare. Nulla eget facilisis lacus.";
    let (_consumed, written) = compressor.compress_chunk(input, &mut compressed).unwrap();
    let flush_written = compressor.flush(&mut compressed[written..], false).unwrap();
    let (mut decompressor, header_consumed) = Decompressor1K::from_header(&compressed).unwrap();
    let mut decompressed = [0u8; 128];
    let (_, written) = decompressor.decompress_chunk(
        &compressed[header_consumed..written + flush_written],
        &mut decompressed
    ).unwrap();

    assert_eq!(&decompressed[..written], input);

    println!("original size: {}", input.len());
    println!("compressed size: {}", written);

}
