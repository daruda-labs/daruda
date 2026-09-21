use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=fixture.rs");
    let target = env::var("TARGET").expect("Cargo target");
    let name = if target.contains("windows") {
        "test-process.exe"
    } else {
        "test-process"
    };
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output")).join(name);
    let status = Command::new(env::var_os("RUSTC").expect("Cargo compiler"))
        .args(["--edition=2024", "--target", &target, "fixture.rs", "-o"])
        .arg(&output)
        .status()
        .expect("compile process fixture");
    assert!(status.success(), "process fixture compilation failed");
    println!("cargo:rustc-env=TEST_PROCESS_EXE={}", output.display());
}
