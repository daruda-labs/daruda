use std::path::PathBuf;
use std::process::Command;

mod build_target;

const ZIG_VERSION: &str = "0.14.1";

fn main() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("ghostty_vt_sys must live under crates/*");

    let ghostty_dir = workspace_root.join("vendor/ghostty");
    println!("cargo:rerun-if-env-changed=ZIG");
    println!("cargo:rerun-if-env-changed=PATH");
    println!(
        "cargo:rerun-if-changed={}",
        ghostty_dir.join("src").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        ghostty_dir.join("build.zig.zon").display()
    );
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
    let target = build_target::zig_target(&arch, &os, &abi).unwrap_or_else(|e| panic!("{e}"));

    let mut cmd = Command::new(&zig);
    cmd.current_dir(manifest_dir.join("zig"))
        .arg("build")
        .arg("-Doptimize=ReleaseFast")
        .arg(format!("-Dtarget={target}"))
        .arg(format!(
            "-Dghostty-source={}",
            ghostty_dir.join("src").display()
        ))
        .arg("--cache-dir")
        .arg(out_dir.join("zig-cache"))
        .arg("--prefix")
        .arg(&prefix);

    // Force Zig to use its bundled libSystem.tbd instead of the Xcode SDK on
    // macOS. Zig 0.14.1's LLD cannot parse macOS 26 (Xcode 26.4) SDK stubs:
    // the SDK declares `arm64e-macos` targets while LLD only matches
    // `aarch64-macos`, surfacing as undefined `_abort` / `_malloc` symbols.
    // ghostty_vt is a pure VT parser with no direct macOS API calls, so the
    // bundled libSystem suffices. Remove once ghostty supports Zig 0.16+
    // (tracking: https://codeberg.org/ziglang/zig/issues/31658,
    // https://github.com/ghostty-org/ghostty/issues/12228).
    // The Unicode generators link on the host even when cross-compiling.
    if cfg!(target_os = "macos") {
        cmd.env("DEVELOPER_DIR", "/dev/null");
    }

    let status = cmd.status().expect("failed to invoke zig");
    if !status.success() {
        panic!("zig build failed");
    }

    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib").display()
    );
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
