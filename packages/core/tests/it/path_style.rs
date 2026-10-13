//! Public API tests for path style.

use daruda_core::path_style::*;

#[test]
fn local_matches_the_build_target() {
    assert_eq!(PathStyle::local() == PathStyle::Windows, cfg!(windows));
}

#[test]
fn windows_takes_both_separators_and_posix_only_one() {
    assert!(PathStyle::Windows.is_separator('\\'));
    assert!(PathStyle::Windows.is_separator('/'));
    assert!(PathStyle::Posix.is_separator('/'));
    assert!(!PathStyle::Posix.is_separator('\\'));
}

#[test]
fn a_share_or_device_is_not_a_local_disk() {
    let w = PathStyle::Windows;
    assert!(w.is_network_or_device("\\\\evil\\share\\a.png"));
    assert!(w.is_network_or_device("//evil/share/a.png"));
    assert!(w.is_network_or_device("\\\\?\\UNC\\evil\\share"));
    assert!(w.is_network_or_device("\\\\.\\pipe\\x"));
    assert!(!w.is_network_or_device("\\\\?\\C:\\Users"));
    assert!(!w.is_network_or_device("C:\\Users"));
    assert!(!w.is_network_or_device("\\Users"));
    assert!(!w.is_network_or_device("relative\\a.png"));
    assert!(!PathStyle::Posix.is_network_or_device("//evil/share/a.png"));
}

#[test]
fn absolute_follows_the_style_not_the_host() {
    assert!(PathStyle::Posix.is_absolute("/tmp"));
    assert!(!PathStyle::Posix.is_absolute("C:\\Users"));
    assert!(!PathStyle::Posix.is_absolute("src/main.rs"));

    assert!(PathStyle::Windows.is_absolute("C:\\Users"));
    assert!(PathStyle::Windows.is_absolute("c:/Users"));
    assert!(PathStyle::Windows.is_absolute("\\\\server\\share"));
    assert!(PathStyle::Windows.is_absolute("/tmp"));
    assert!(!PathStyle::Windows.is_absolute("C:Users"));
    assert!(!PathStyle::Windows.is_absolute("src\\main.rs"));
}
