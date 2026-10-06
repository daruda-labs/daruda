//! `file:` URLs, decoded for a named [`PathStyle`] instead of the build OS.
//!
//! Two questions that `url::Url::to_file_path` answers as one:
//! - *Is it a file URL?* — [`is_file_url`], scheme only. True on every host,
//!   so a remote session can refuse the link without asking whether the path
//!   would fit this machine (`file:///tmp` has no Windows form).
//! - *Which path, on which machine?* — [`FileUrl::parse`] decodes for a given
//!   style and keeps the host apart; [`to_local_path`] is the answer for this
//!   machine. A decode failure never means "not a file".
//!
//! Pure: no filesystem access. The Windows rules are asserted from any host.

use std::path::{Path, PathBuf};

use crate::path_style::PathStyle;

const FILE_SCHEME: &str = "file";
const LOCALHOST: &str = "localhost";

/// Whether `link` is a `file:` URL, however its path is spelled.
pub fn is_file_url(link: &str) -> bool {
    url::Url::parse(link).is_ok_and(|url| url.scheme() == FILE_SCHEME)
}

/// `link` as a path on this machine: a `file:` URL with no host,
/// `localhost`, or `this_host`, decoded for [`PathStyle::local`].
pub fn to_local_path(link: &str, this_host: Option<&str>) -> Option<PathBuf> {
    let url = FileUrl::parse(link, PathStyle::local())?;
    url.names_this_machine(this_host)
        .then(|| PathBuf::from(url.path))
}

/// The `file:` URL for a path on this machine, percent-encoded — what the
/// platform opener needs, since it refuses a raw path holding a space.
/// `None` for a relative path.
pub fn from_local_path(path: &Path) -> Option<String> {
    url::Url::from_file_path(path).ok().map(String::from)
}

/// A decoded `file:` URL: the machine it names and the path written in the
/// style it was decoded for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileUrl {
    host: Option<String>,
    path: String,
}

impl FileUrl {
    /// `None` when `link` is not a `file:` URL, or its path has no form in
    /// `style` — a Windows path needs a drive or a host, and both styles
    /// need the decoded bytes to be UTF-8.
    pub fn parse(link: &str, style: PathStyle) -> Option<Self> {
        let url = url::Url::parse(link).ok()?;
        if url.scheme() != FILE_SCHEME {
            return None;
        }
        let host = url
            .host_str()
            .filter(|host| !host.is_empty() && !host.eq_ignore_ascii_case(LOCALHOST))
            .map(str::to_owned);
        let segments = url.path_segments()?;
        let path = match style {
            PathStyle::Posix => posix_path(segments)?,
            PathStyle::Windows => windows_path(host.as_deref(), segments)?,
        };
        Some(Self { host, path })
    }

    /// The machine the URL names; `None` when it names the local one by
    /// spelling (no host, or `localhost`).
    pub fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// The path in the decoded style. Under Windows a named host is part of
    /// it, as a UNC path (`\\server\share\…`).
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Whether the URL names this machine, known as `this_host`. A short and
    /// a qualified name match (`box` / `box.local`) — shells report either.
    pub fn names_this_machine(&self, this_host: Option<&str>) -> bool {
        match (self.host.as_deref(), this_host) {
            (None, _) => true,
            (Some(host), Some(this_host)) => same_machine(host, this_host),
            (Some(_), None) => false,
        }
    }
}

/// Equal ignoring case, or one is the other's unqualified form (`box` for
/// `box.local`). Two qualified names must agree whole: `mbp.local` and
/// `mbp.lan` are two machines that happen to share a first label.
fn same_machine(a: &str, b: &str) -> bool {
    let unqualified_of = |short: &str, full: &str| {
        !short.contains('.')
            && full
                .split_once('.')
                .is_some_and(|(label, _)| label.eq_ignore_ascii_case(short))
    };
    a.eq_ignore_ascii_case(b) || unqualified_of(a, b) || unqualified_of(b, a)
}

fn posix_path<'a>(segments: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut bytes = Vec::new();
    for segment in segments {
        bytes.push(b'/');
        bytes.extend(percent_encoding::percent_decode_str(segment));
    }
    String::from_utf8(bytes).ok()
}

fn windows_path<'a>(
    host: Option<&str>,
    mut segments: impl Iterator<Item = &'a str>,
) -> Option<String> {
    let sep = PathStyle::Windows.primary_separator();
    let mut path = String::new();
    if let Some(host) = host {
        path.extend([sep, sep]);
        path.push_str(host);
    } else {
        path.push_str(&windows_drive(segments.next()?)?);
    }
    let mut wrote_segment = false;
    for segment in segments {
        path.push(sep);
        path.push_str(
            &percent_encoding::percent_decode_str(segment)
                .decode_utf8()
                .ok()?,
        );
        wrote_segment = true;
    }
    // `C:` alone is drive-relative; the URL named the drive's root.
    if host.is_none() && !wrote_segment {
        path.push(sep);
    }
    Some(path)
}

/// `C:` or its percent-encoded `C%3A`.
fn windows_drive(segment: &str) -> Option<String> {
    let decoded = percent_encoding::percent_decode_str(segment)
        .decode_utf8()
        .ok()?;
    match decoded.as_bytes() {
        [drive, b':'] if drive.is_ascii_alphabetic() => Some(decoded.into_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(link: &str, style: PathStyle) -> Option<String> {
        FileUrl::parse(link, style).map(|url| url.path)
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
}
