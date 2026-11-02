use std::path::PathBuf;
use std::process::Command;

mod build_target;

const ZIG_VERSION: &str = "0.17.0";

fn main() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("ghostty_vt_sys must live under packages/*");

    let ghostty_dir = workspace_root.join("vendor/ghostty");
    println!("cargo:rerun-if-env-changed=ZIG");
    println!("cargo:rerun-if-env-changed=PATH");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    println!(
        "cargo:rerun-if-changed={}",
        ghostty_dir.join("src").display()
    );
    for input in ["build.zig", "build.zig.zon", "pkg"] {
        println!(
            "cargo:rerun-if-changed={}",
            ghostty_dir.join(input).display()
        );
    }
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("include/ghostty_vt.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("zig/build.zig").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("zig/build.zig.zon").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("zig/lib.zig").display()
    );

    if !ghostty_dir.join("src/terminal/main.zig").is_file() {
        panic!(
            "vendor/ghostty is missing; run `git submodule update --init --recursive` and retry"
        );
    }

    let zig = find_zig(workspace_root);
    let version = Command::new(&zig)
        .arg("version")
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "cannot run {}: {e}; install Zig {ZIG_VERSION} or set ZIG",
                zig.display()
            )
        });
    assert!(
        version.status.success(),
        "zig version failed: {}",
        String::from_utf8_lossy(&version.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        ZIG_VERSION,
        "unsupported Zig version"
    );
    if zig.is_absolute() {
        println!("cargo:rerun-if-changed={}", zig.display());
    }

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let prefix = out_dir.join("zig-out");
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").expect("target architecture");
    let os = std::env::var("CARGO_CFG_TARGET_OS").expect("target OS");
    let abi = std::env::var("CARGO_CFG_TARGET_ENV").expect("target ABI");
    let macos_deployment_target = std::env::var("MACOSX_DEPLOYMENT_TARGET").ok();
    let target = build_target::zig_target(&arch, &os, &abi, macos_deployment_target.as_deref())
        .unwrap_or_else(|e| panic!("{e}"));

    let mut cmd = Command::new(&zig);
    cmd.current_dir(manifest_dir.join("zig"))
        .arg("build")
        .arg("-Doptimize=ReleaseFast")
        .arg(format!("-Dtarget={target}"))
        .arg("--cache-dir")
        .arg(out_dir.join("zig-cache"))
        // Zig otherwise fetches dependencies into `zig/zig-pkg` beside
        // build.zig.zon, a source-tree path shared by concurrent builds.
        .arg("--pkg-dir")
        .arg(out_dir.join("zig-pkg"))
        .arg("--prefix")
        .arg(&prefix);

    let status = cmd.status().expect("failed to invoke zig");
    if !status.success() {
        panic!("zig build failed");
    }

    let lib_dir = prefix.join("lib");

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=ghostty_vt");
    // MSVC has no `c` to link — the Rust target pulls the UCRT in itself, and
    // naming a library that does not exist fails the link outright. Read the
    // *target*: a build script is compiled for the host, so `cfg!` here would
    // answer about the wrong machine.
    if os != "windows" {
        println!("cargo:rustc-link-lib=c");
    }
}

fn find_zig(workspace_root: &std::path::Path) -> PathBuf {
    if let Some(path) = std::env::var_os("ZIG") {
        let path = PathBuf::from(path);
        return path.canonicalize().unwrap_or(path);
    }

    let executable = if cfg!(windows) { "zig.exe" } else { "zig" };
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths) {
            let candidate = directory.join(executable);
            if candidate.is_file() {
                return candidate.canonicalize().unwrap_or(candidate);
            }
        }
    }

    workspace_root
        .join(".context/zig")
        .join(ZIG_VERSION)
        .join(executable)
}
