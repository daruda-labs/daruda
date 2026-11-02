use super::zig_target;

#[test]
fn supported_targets_do_not_depend_on_the_build_host() {
    for arch in ["x86_64", "aarch64"] {
        for (os, abi, suffix) in [
            ("macos", "", "macos.12.0"),
            ("linux", "gnu", "linux-gnu"),
            ("linux", "musl", "linux-musl"),
            ("windows", "msvc", "windows-msvc"),
        ] {
            assert_eq!(
                zig_target(arch, os, abi, Some("12.0")).unwrap(),
                format!("{arch}-{suffix}")
            );
        }
    }
}

#[test]
fn unsupported_abis_never_fall_back_to_native() {
    assert!(zig_target("x86_64", "windows", "gnu", None).is_err());
    assert!(zig_target("x86", "windows", "msvc", None).is_err());
    assert!(zig_target("x86_64", "linux", "", None).is_err());
}

#[test]
fn macos_requires_the_rust_deployment_floor() {
    assert!(zig_target("aarch64", "macos", "", None).is_err());
}
