# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

tamp-rs provides Rust bindings to the [tamp compression library](https://github.com/BrianPugh/tamp). This is a Cargo workspace with two main crates:

- **`tamp-sys`** - Low-level FFI bindings to the C library (uses bindgen)
- **`tamp`** - High-level Rust API wrapper (currently a stub)

The project includes the tamp C library as a git submodule at `tamp-sys/tamp/`.

## Development Commands

### Building
```bash
cargo build           # Build all workspace members
cargo build -p tamp   # Build specific crate
```

### Testing
```bash
cargo test            # Run all tests
cargo test -p tamp    # Test specific crate
```

### Linting
```bash
cargo clippy          # Run Clippy linter
cargo fmt            # Format code
```

## Architecture

This is a standard Rust FFI binding pattern:

1. **tamp-sys** contains the build script (`build.rs`) that:
   - Uses bindgen to generate Rust bindings from C headers
   - Links the tamp C library
   - Currently configured for edition "2024" and includes `bindgen` as build dependency

2. **tamp** will provide the high-level Rust API that wraps the sys crate
   - Currently contains only a placeholder `add` function
   - Should depend on `tamp-sys` once implemented

The C library source is located in `tamp-sys/tamp/` and includes a comprehensive CLAUDE.md with detailed information about the upstream tamp project's multi-language implementation strategy.