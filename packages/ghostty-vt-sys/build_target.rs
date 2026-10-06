//! Pure Cargo-to-Zig target mapping, shared by the build script and tests.

pub fn zig_target(arch: &str, os: &str, abi: &str) -> Result<String, String> {
    let arch = match arch {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => return Err(format!("unsupported Ghostty target architecture: {other}")),
    };
    let platform = match (os, abi) {
        ("macos", "") => "macos",
        ("linux", "gnu") => "linux-gnu",
        ("linux", "musl") => "linux-musl",
        ("windows", "msvc") => "windows-msvc",
        _ => return Err(format!("unsupported Ghostty target platform: {os}/{abi}")),
    };
    Ok(format!("{arch}-{platform}"))
}

#[cfg(test)]
#[path = "build_target_tests.rs"]
mod tests;
