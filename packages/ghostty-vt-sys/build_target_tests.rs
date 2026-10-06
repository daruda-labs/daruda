use super::zig_target;

#[test]
fn supported_targets_do_not_depend_on_the_build_host() {
    for arch in ["x86_64", "aarch64"] {
        for (os, abi, suffix) in [
            ("macos", "", "macos"),
            ("linux", "gnu", "linux-gnu"),
            ("linux", "musl", "linux-musl"),
            ("windows", "msvc", "windows-msvc"),
        ] {
            assert_eq!(
                zig_target(arch, os, abi).unwrap(),
                format!("{arch}-{suffix}")
            );
        }
    }
}

#[test]
fn unsupported_abis_never_fall_back_to_native() {
    assert!(zig_target("x86_64", "windows", "gnu").is_err());
    assert!(zig_target("x86", "windows", "msvc").is_err());
    assert!(zig_target("x86_64", "linux", "").is_err());
}
