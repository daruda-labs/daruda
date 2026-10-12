use super::*;

fn target(exe: &str, host: Host) -> InstallGate {
    InstallGate::for_host(Path::new(exe), Some(host))
}

#[test]
fn a_bundled_mac_build_is_replaced_as_its_bundle() {
    assert_eq!(
        target(
            "/Applications/daruda.app/Contents/MacOS/daruda",
            Host::MacOs
        ),
        InstallGate::Target(InstallTarget::Bundle(PathBuf::from(
            "/Applications/daruda.app"
        )))
    );
}

#[test]
fn a_mac_dev_build_is_not_an_install() {
    assert_eq!(
        target("/Users/dev/daruda/target/debug/daruda", Host::MacOs),
        InstallGate::Unpackaged
    );
}

/// A portable install has no marker in its path, so the directory holding
/// the executable is the answer. Forward slashes because `Path` splits on
/// the *host's* separator — a backslash is an ordinary character here on
/// macOS, and Windows takes either. Only the branch is under test.
#[test]
fn a_windows_build_is_replaced_as_the_directory_it_sits_in() {
    assert_eq!(
        target(
            "C:/Users/me/daruda-0.3.0-windows-x86_64/daruda.exe",
            Host::Windows
        ),
        InstallGate::Target(InstallTarget::Directory(PathBuf::from(
            "C:/Users/me/daruda-0.3.0-windows-x86_64"
        )))
    );
}

#[test]
fn a_windows_cargo_build_is_not_an_install() {
    for exe in [
        "C:/src/daruda/target/debug/daruda.exe",
        "C:/src/daruda/target/release/daruda.exe",
    ] {
        assert_eq!(target(exe, Host::Windows), InstallGate::Unpackaged, "{exe}");
    }
}

/// The guard is narrow on purpose: a real install may well sit in a
/// directory called `release`, just not one under `target`.
#[test]
fn a_directory_named_release_outside_target_is_still_an_install() {
    assert_eq!(
        target("C:/Users/me/release/daruda.exe", Host::Windows),
        InstallGate::Target(InstallTarget::Directory(PathBuf::from(
            "C:/Users/me/release"
        )))
    );
}

#[test]
fn managed_windows_installations_use_setup_instead_of_zip() {
    let root = tempfile::tempdir().unwrap();
    let exe = root.path().join("daruda.exe");
    std::fs::write(root.path().join("uninstall.exe"), "installer owned").unwrap();
    let gate = InstallGate::for_host(&exe, Some(Host::Windows));
    let target = gate.target().unwrap();
    assert!(target.uses_installer());
    assert_eq!(
        target.asset_suffix(),
        Some(super::super::installer::ASSET_SUFFIX)
    );
}

/// Both the deb's `/usr/bin` and an extracted tar.gz are announced, not
/// replaced: dpkg owns the first, and Linux has no swap path.
#[test]
fn a_linux_install_is_left_to_its_package() {
    for exe in [
        "/usr/bin/daruda",
        "/home/me/daruda-0.3.0-linux-x86_64/daruda",
    ] {
        assert_eq!(
            target(exe, Host::Linux),
            InstallGate::ManualPackage,
            "{exe}"
        );
    }
}

#[test]
fn a_linux_cargo_build_is_not_a_package() {
    assert_eq!(
        target("/home/me/daruda/target/debug/daruda", Host::Linux),
        InstallGate::Unpackaged
    );
}

/// A Linux path inside a directory named like a bundle is still a Linux
/// install; the macOS rule must not leak across hosts.
#[test]
fn each_host_applies_only_its_own_rule() {
    let exe = "/opt/daruda.app/daruda";
    assert!(matches!(target(exe, Host::MacOs), InstallGate::Target(_)));
    assert_eq!(target(exe, Host::Linux), InstallGate::ManualPackage);
}

/// The same path answers differently per host, which is the whole reason
/// the decision is one value rather than a `cfg` at each step.
#[test]
fn the_mac_and_windows_rules_do_not_answer_alike() {
    let exe = "/Applications/daruda.app/Contents/MacOS/daruda";
    assert_ne!(target(exe, Host::MacOs), target(exe, Host::Windows));
}

#[test]
fn a_host_without_packages_is_unpackaged() {
    assert_eq!(
        InstallGate::for_host(Path::new("/usr/local/bin/daruda"), None),
        InstallGate::Unpackaged
    );
}
