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

    let sysroot = arm_none_eabi_sysroot().trim().to_owned();

    println!("{}", sysroot);
    let bindings = bindgen::Builder::default()
        .clang_arg(format!("--target={}", target))
        // .clang_arg("-I/usr/include")
        // .clang_arg(clang_resource_include())
        .clang_arg("-Itamp/tamp/_c_src")
        .clang_arg(format!("--sysroot={}", sysroot))
        .header("wrapper.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
