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
