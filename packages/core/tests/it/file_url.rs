//! Public API tests for file url.

use daruda_core::file_url::*;
use daruda_core::path_style::PathStyle;
use std::path::Path;

fn path(link: &str, style: PathStyle) -> Option<String> {
    FileUrl::parse(link, style).map(|url| url.path().to_owned())
}

/// What the Windows CI failure came down to: whether a link is a file
/// URL cannot depend on whether its path fits this machine.
#[test]
fn a_file_url_is_one_on_every_host() {
    assert!(is_file_url("file:///tmp"));
    assert!(is_file_url("file:///C:/Users"));
    assert!(is_file_url("file://server/share/a.rs"));
    assert!(is_file_url("FILE:///tmp"));
    assert!(!is_file_url("https://example.com"));
    assert!(!is_file_url("/tmp"));
    assert!(!is_file_url("C:\\Users"));
}

#[test]
fn posix_decodes_segments_and_keeps_the_host_apart() {
    assert_eq!(
        path("file:///tmp/x", PathStyle::Posix).as_deref(),
        Some("/tmp/x")
    );
    assert_eq!(path("file:///", PathStyle::Posix).as_deref(), Some("/"));
    assert_eq!(
        path("file:///a%20b/c", PathStyle::Posix).as_deref(),
        Some("/a b/c")
    );
    let remote = FileUrl::parse("file://box/home/u", PathStyle::Posix).unwrap();
    assert_eq!(remote.host(), Some("box"));
    assert_eq!(remote.path(), "/home/u");
}

#[test]
fn localhost_names_no_other_machine() {
    let url = FileUrl::parse("file://localhost/tmp", PathStyle::Posix).unwrap();
    assert_eq!(url.host(), None);
    assert_eq!(url.path(), "/tmp");
}

#[test]
fn windows_needs_a_drive_or_a_host() {
    let w = PathStyle::Windows;
    assert_eq!(
        path("file:///C:/Users/x", w).as_deref(),
        Some("C:\\Users\\x")
    );
    assert_eq!(path("file:///c%3A/x", w).as_deref(), Some("c:\\x"));
    assert_eq!(path("file:///C:", w).as_deref(), Some("C:\\"));
    assert_eq!(path("file://localhost/C:/x", w).as_deref(), Some("C:\\x"));
    assert_eq!(path("file:///C:/a%20b", w).as_deref(), Some("C:\\a b"));
    assert_eq!(path("file:///tmp", w), None);
    let unc = FileUrl::parse("file://server/share/a.rs", w).unwrap();
    assert_eq!(unc.host(), Some("server"));
    assert_eq!(unc.path(), "\\\\server\\share\\a.rs");
}

#[test]
fn a_decoded_path_is_absolute_in_its_own_style() {
    for style in [PathStyle::Posix, PathStyle::Windows] {
        for link in ["file:///C:/x", "file:///C:", "file://server/share"] {
            if let Some(path) = path(link, style) {
                assert!(style.is_absolute(&path), "{style:?} {link} -> {path}");
            }
        }
    }
}

#[test]
fn non_file_urls_and_bare_paths_do_not_parse() {
    for style in [PathStyle::Posix, PathStyle::Windows] {
        assert_eq!(FileUrl::parse("https://example.com/a", style), None);
        assert_eq!(FileUrl::parse("/tmp/x", style), None);
        assert_eq!(FileUrl::parse("C:\\x", style), None);
    }
}

#[test]
fn invalid_utf8_has_no_path() {
    assert_eq!(path("file:///a%FF", PathStyle::Posix), None);
    assert_eq!(path("file:///C:/a%FF", PathStyle::Windows), None);
}

#[test]
fn a_host_names_this_machine_by_either_spelling() {
    let url = FileUrl::parse("file://Box/home", PathStyle::Posix).unwrap();
    assert!(url.names_this_machine(Some("box")));
    assert!(url.names_this_machine(Some("box.local")));
    assert!(!url.names_this_machine(Some("other")));
    let qualified = FileUrl::parse("file://mbp.local/home", PathStyle::Posix).unwrap();
    assert!(qualified.names_this_machine(Some("MBP.local")));
    assert!(qualified.names_this_machine(Some("mbp")));
    assert!(!qualified.names_this_machine(Some("mbp.lan")));
    assert!(!url.names_this_machine(None));
    let local = FileUrl::parse("file:///home", PathStyle::Posix).unwrap();
    assert!(local.names_this_machine(None));
}

#[test]
fn to_local_path_refuses_another_machine() {
    assert_eq!(to_local_path("file://box/home", Some("other")), None);
    assert_eq!(to_local_path("https://example.com", None), None);
}

#[test]
fn a_local_path_round_trips_through_its_url() {
    let dir = std::env::temp_dir().join("a b");
    let url = from_local_path(&dir).expect("an absolute path has a URL");
    assert!(url.contains("a%20b"), "{url}");
    assert_eq!(to_local_path(&url, None), Some(dir));
    assert_eq!(from_local_path(Path::new("relative")), None);
}

/// Pins the port to the reference decoder on whichever host runs it —
/// the Windows CI job checks the Windows arm against `url` itself.
#[test]
fn local_style_agrees_with_url_on_this_host() {
    for link in [
        "file:///tmp/x",
        "file:///a%20b",
        "file:///C:/Users/x",
        "file:///c%3A/x",
        "file://localhost/tmp",
    ] {
        let reference = url::Url::parse(link).unwrap().to_file_path().ok();
        assert_eq!(to_local_path(link, None), reference, "{link}");
    }
}
