# tamp-rs

Rust bindings to the [tamp][] compression library.

[tamp]: https://github.com/BrianPugh/tamp

## Features

Default features enables both compression and decompression.

* `compressor` Enable compression.
* `decompressor` Enable decompression.

## Test

From the workspace root:

```
cargo test -p tamp -- --nocapture

```

## License

MIT or Apache-2.0
