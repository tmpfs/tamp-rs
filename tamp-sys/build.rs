use std::env;
use std::path::PathBuf;
use std::process::Command;

fn clang_resource_include() -> String {
    let out = Command::new("clang")
        .arg("-print-resource-dir")
        .output()
        .expect("failed to invoke clang -print-resource-dir");
    let dir = String::from_utf8(out.stdout).unwrap();
    format!("-I{}/include", dir.trim())
}

fn arm_none_eabi_sysroot() -> String {
    String::from_utf8(
        Command::new("arm-none-eabi-gcc")
            .arg("-print-sysroot")
            .output()
            .expect("failed to run arm-none-eabi-gcc -print-sysroot")
            .stdout,
    )
    .unwrap()
}

fn main() {
    let target = std::env::var("TARGET").unwrap();
    
    let mut builder = bindgen::Builder::default()
        .clang_arg(format!("--target={}", target))
        .clang_arg("-Itamp/tamp/_c_src")
        .header("wrapper.h")
        .use_core()
        .ctypes_prefix("::core::ffi")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));

    if target.starts_with("thumbv") || target.starts_with("xtensa") {
        // ARM embedded targets (e.g., thumbv7em-none-eabihf)
        let sysroot = arm_none_eabi_sysroot().trim().to_owned();
        builder = builder.clang_arg(format!("--sysroot={}", sysroot));
    } else {
        // Desktop targets (Windows, Linux, macOS)
        builder = builder.clang_arg(clang_resource_include());
    }

    let bindings = builder
        .generate()
        .expect("Unable to generate bindings");

    // Build the C library with size optimizations
    let mut build = cc::Build::new();
    let mut files = vec!["tamp/tamp/_c_src/tamp/common.c"];
    
    // Only compile what's needed based on features
    #[cfg(feature = "compressor")]
    files.push("tamp/tamp/_c_src/tamp/compressor.c");
    
    #[cfg(feature = "decompressor")]
    files.push("tamp/tamp/_c_src/tamp/decompressor.c");
    
    build.files(&files)
        .flag("-Wno-type-limits")
        .include("tamp/tamp/_c_src");
    
    // Add size optimization flags for embedded targets
    if target.starts_with("thumbv") {
        build
            .flag("-Os")           // Optimize for size
            .flag("-ffunction-sections")  // Place functions in separate sections
            .flag("-fdata-sections")      // Place data in separate sections
            // .flag("-flto")               // Link-time optimization
            .flag("-DTAMP_LAZY_MATCHING=0"); // Disable lazy matching to save code size
    }
    
    build.compile("tamp");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
