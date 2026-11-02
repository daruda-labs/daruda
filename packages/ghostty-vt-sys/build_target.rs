//! Pure Cargo-to-Zig target mapping, shared by the build script and tests.

/// `macos_deployment_target` is the floor the Rust objects build for, so the
/// archive links into them without a newer-version mismatch.
pub fn zig_target(
    arch: &str,
    os: &str,
    abi: &str,
    macos_deployment_target: Option<&str>,
) -> Result<String, String> {
    let arch = match arch {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => return Err(format!("unsupported Ghostty target architecture: {other}")),
    };
    let platform = match (os, abi) {
        // Zig would otherwise target the build host's newer macOS version.
        ("macos", "") => match macos_deployment_target {
            Some(version) => format!("macos.{version}"),
            None => return Err("MACOSX_DEPLOYMENT_TARGET is not set".to_owned()),
        },
        ("linux", "gnu") => "linux-gnu".to_owned(),
        ("linux", "musl") => "linux-musl".to_owned(),
        ("windows", "msvc") => "windows-msvc".to_owned(),
        _ => return Err(format!("unsupported Ghostty target platform: {os}/{abi}")),
    };
    Ok(format!("{arch}-{platform}"))
}

#[cfg(test)]
#[path = "build_target_tests.rs"]
mod tests;
