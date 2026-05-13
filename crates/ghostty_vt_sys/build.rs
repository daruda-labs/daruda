use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("ghostty_vt_sys must live under crates/*");

    let ghostty_dir = workspace_root.join("vendor/ghostty");
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

    if !ghostty_dir.exists() {
        panic!(
            "vendor/ghostty is missing; run `git submodule update --init --recursive` and retry"
        );
    }

    let zig = find_zig(workspace_root);
    let zig_version = Command::new(&zig).arg("version").output().ok();
    if zig_version.is_none() {
        panic!(
            "`zig` is required; run `./scripts/bootstrap-zig.sh` \
to install Zig 0.14.1 into .context/zig/zig"
        );
    }

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let prefix = out_dir.join("zig-out");

    let mut cmd = Command::new(&zig);
    cmd.current_dir(manifest_dir.join("zig"))
        .arg("build")
        .arg("-Doptimize=ReleaseFast")
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
    println!("cargo:rustc-link-lib=c");
}

fn find_zig(workspace_root: &std::path::Path) -> PathBuf {
    if let Some(path) = std::env::var_os("ZIG") {
        return PathBuf::from(path);
    }

    if Command::new("zig").arg("version").output().is_ok() {
        return PathBuf::from("zig");
    }

    workspace_root.join(".context/zig/zig")
}
