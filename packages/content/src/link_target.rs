//! Where a link inside a pane points — one answer shared by the left click,
//! the context menu and the resource-image preview, so a menu cannot offer
//! an opener the click declines. GPUI-free; the only filesystem access is a
//! `metadata` probe on the resolved path, because the file's *kind* decides
//! who opens it and the kind cannot be read off the link text (`./out` may
//! be a directory).

use std::path::{Path, PathBuf};

use daruda_core::file_url;
use daruda_core::path_style::PathStyle;

/// What a link resolves to once the pane's working directory is known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    /// A URL for the platform opener — browser, mail client, custom scheme.
    Web { url: String },
    /// A path on this machine, its `:line[:col]` suffix already stripped.
    Local {
        path: PathBuf,
        line: Option<usize>,
        kind: LocalKind,
    },
    /// A path on a remote session's machine. Probing it here would answer for
    /// the wrong filesystem, so it is never classified further.
    Remote,
    /// Nothing can open it: an in-document anchor, or a bare name the pane
    /// has no working directory to resolve against.
    Opaque,
}

/// Who should open a local path. Decided by a `metadata` probe plus the
/// extension, never by reading the bytes — the viewer does that itself and
/// shows its binary placeholder when a `Text` guess turns out wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalKind {
    /// Anything not classified below: the pane file viewer shows it.
    Text,
    /// A raster the app can decode — previewed inline, zoomed in the
    /// lightbox, opened externally in the OS image viewer.
    Image,
    /// A format only another application renders: PDF, archive, media, font.
    Binary,
    Directory,
    /// Resolved to a path that is not there (deleted since the agent wrote it).
    Missing,
}

/// Extensions the in-app decoder handles ([`crate::visual::decode_image`]); the same
/// list gates the resource-link preview.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/// Formats the pane file viewer cannot show and the OS has a handler for.
/// Deliberately short: an unknown extension falls through to `Text`, where
/// the viewer's own binary detection is the final word.
const BINARY_EXTENSIONS: &[&str] = &[
    "pdf", "zip", "gz", "tgz", "bz2", "xz", "7z", "rar", "tar", "dmg", "pkg", "exe", "dll", "so",
    "dylib", "jar", "class", "mp3", "m4a", "wav", "flac", "ogg", "mp4", "mov", "avi", "mkv",
    "webm", "woff", "woff2", "ttf", "otf", "ico", "icns", "psd", "sqlite", "db",
];

fn is_image_extension(extension: &str) -> bool {
    let ext = extension.to_ascii_lowercase();
    IMAGE_EXTENSIONS.contains(&ext.as_str())
}

/// MIME types the in-app decoder handles — the declared-type twin of
/// [`IMAGE_EXTENSIONS`].
fn is_supported_image_mime(mime: &str) -> bool {
    matches!(
        mime.split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "image/png"
            | "image/jpeg"
            | "image/jpg"
            | "image/gif"
            | "image/webp"
            | "image/bmp"
            | "image/x-ms-bmp"
    )
}

fn declared_mime(mime: Option<&str>) -> Option<&str> {
    mime.map(str::trim).filter(|mime| !mime.is_empty())
}

/// Whether a resource renders as an image: by its declared MIME when the
/// tool sent one, else by extension. The inline preview and the click both
/// ask this, so a previewed image never opens as text.
pub fn is_image_resource(path: &Path, mime: Option<&str>) -> bool {
    declared_mime(mime).map_or_else(
        || {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(is_image_extension)
        },
        is_supported_image_mime,
    )
}

fn is_binary_extension(extension: &str) -> bool {
    let ext = extension.to_ascii_lowercase();
    BINARY_EXTENSIONS.contains(&ext.as_str())
}

/// Whether a link points outside the filesystem. Shared with the classifier
/// so the menu and the click cannot disagree about what a link is.
pub fn is_external_url(link: &str) -> bool {
    link.contains("://")
        || link.starts_with("mailto:")
        || link.starts_with("tel:")
        || link.starts_with('#')
}

/// Classify `link` as the pane with working directory `cwd` sees it.
pub fn classify(link: &str, cwd: Option<&Path>) -> LinkTarget {
    if link.starts_with('#') {
        return LinkTarget::Opaque;
    }
    match local_path(link, cwd) {
        Some(LocalPath { path, line }) => {
            let kind = kind_of(&path);
            LinkTarget::Local { path, line, kind }
        }
        None if is_external_url(link) => LinkTarget::Web {
            url: link.to_string(),
        },
        None => LinkTarget::Opaque,
    }
}

/// Classify `link` for a remote session: a URL still opens here, but any
/// path — `file://` included — names the remote machine's filesystem.
pub fn classify_remote(link: &str) -> LinkTarget {
    if link.starts_with('#') {
        LinkTarget::Opaque
    } else if file_url::is_file_url(link) || !is_external_url(link) {
        LinkTarget::Remote
    } else {
        LinkTarget::Web {
            url: link.to_string(),
        }
    }
}

/// Classify a tool's resource-link URI. Unlike Markdown text, a resource is a
/// file by definition, so a relative URI resolves against `cwd` even when
/// absent (→ `Missing`, which reports) instead of reading as a plain word.
pub fn classify_resource(uri: &str, mime: Option<&str>, cwd: Option<&Path>) -> LinkTarget {
    if uri.starts_with('#') {
        return LinkTarget::Opaque;
    }
    match resource_path(uri, cwd) {
        Some(path) => {
            let kind = resource_kind(&path, mime);
            LinkTarget::Local {
                path,
                line: None,
                kind,
            }
        }
        None if is_external_url(uri) => LinkTarget::Web {
            url: uri.to_string(),
        },
        None => LinkTarget::Opaque,
    }
}

/// The local path a URI reference names, without touching the disk: a
/// `file://` URL for this machine, an absolute path, or a relative one under
/// `base`. `None` for any other URL. Shared by a tool's resource link (click
/// and inline preview) and a Markdown image in the file viewer.
pub fn resource_path(uri: &str, base: Option<&Path>) -> Option<PathBuf> {
    if file_url::is_file_url(uri) {
        return file_url::to_local_path(uri, None);
    }
    if is_external_url(uri) {
        return None;
    }
    let path = Path::new(uri);
    // A UNC path is absolute, so only this branch can name a share.
    if path.is_absolute() {
        on_local_disk(path)
    } else {
        base.map(|base| base.join(path))
    }
}

/// `path`, unless it names a network share or a device. The link is
/// untrusted — a README, a tool's output — and on Windows merely probing
/// `\\host\share` authenticates to that host, so it is refused here, before
/// any `metadata`, `canonicalize` or read can reach it.
fn on_local_disk(path: &Path) -> Option<PathBuf> {
    let spelled = path.to_str()?;
    (!PathStyle::local().is_network_or_device(spelled)).then(|| path.to_path_buf())
}

/// [`kind_of`], with a declared MIME overriding the extension's say on
/// whether a file is an image — the same rule as [`is_image_resource`].
fn resource_kind(path: &Path, mime: Option<&str>) -> LocalKind {
    let kind = kind_of(path);
    if declared_mime(mime).is_none() {
        return kind;
    }
    match kind {
        LocalKind::Directory | LocalKind::Missing => kind,
        _ if is_image_resource(path, mime) => LocalKind::Image,
        // Declared as something other than an image: the OS picks the handler.
        LocalKind::Image => LocalKind::Binary,
        other => other,
    }
}

fn kind_of(path: &Path) -> LocalKind {
    let Ok(metadata) = std::fs::metadata(path) else {
        return LocalKind::Missing;
    };
    if metadata.is_dir() {
        return LocalKind::Directory;
    }
    let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    if is_image_extension(ext) {
        LocalKind::Image
    } else if is_binary_extension(ext) {
        LocalKind::Binary
    } else {
        LocalKind::Text
    }
}

/// A link resolved to a filesystem path, `:line[:col]` suffix stripped.
#[derive(Clone, Debug, PartialEq, Eq)]
struct LocalPath {
    path: PathBuf,
    line: Option<usize>,
}

/// The path a link names, if it names one. Accepts a `file://` URL, an
/// absolute path, and a relative path when `cwd` is known — but a bare name
/// (`README`) only when it exists under `cwd`, since a word is not a link.
/// `None` for URLs of any other scheme and for anchors.
fn local_path(link: &str, cwd: Option<&Path>) -> Option<LocalPath> {
    let path = if let Some(path) = file_url::to_local_path(link, None) {
        path
    } else if is_external_url(link) {
        return None;
    } else {
        let path = PathBuf::from(link);
        if path.is_absolute() {
            on_local_disk(&path)?
        } else if link.contains(|c| PathStyle::local().is_separator(c))
            || cwd
                .map(|cwd| strip_line_suffix(cwd.join(&path)).path.is_file())
                .unwrap_or(false)
        {
            cwd?.join(path)
        } else {
            return None;
        }
    };
    Some(strip_line_suffix(path))
}

fn parse_numeric_suffix(suffix: &str) -> Option<Option<usize>> {
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(suffix.parse::<usize>().ok().filter(|n| *n > 0))
}

/// Peel up to two numeric `:n` suffixes (`path:line:col`) — but a file whose
/// name literally contains the colon wins, so the probe runs before each cut.
fn strip_line_suffix(path: PathBuf) -> LocalPath {
    if path.is_file() {
        return LocalPath { path, line: None };
    }

    let Some(mut s) = path.to_str().map(str::to_owned) else {
        return LocalPath { path, line: None };
    };
    let mut line = None;
    for _ in 0..2 {
        let Some((prefix, suffix)) = s.rsplit_once(':') else {
            break;
        };
        let Some(parsed) = parse_numeric_suffix(suffix) else {
            break;
        };
        if let Some(n) = parsed {
            line = Some(n);
        }
        s = prefix.to_string();
        let stripped = PathBuf::from(&s);
        if stripped.is_file() {
            return LocalPath {
                path: stripped,
                line,
            };
        }
    }
    LocalPath {
        path: PathBuf::from(s),
        line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_absolute_line_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diff.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();

        let link = format!("{}:75", path.display());
        assert_eq!(
            local_path(&link, None),
            Some(LocalPath {
                path,
                line: Some(75)
            })
        );
    }

    #[test]
    fn strips_line_and_column_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diff.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();

        let link = format!("{}:75:9", path.display());
        assert_eq!(
            local_path(&link, None),
            Some(LocalPath {
                path,
                line: Some(75)
            })
        );
    }

    #[test]
    fn keeps_colon_filename_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diff.rs:75");
        std::fs::write(&path, "literal colon filename\n").unwrap();

        assert_eq!(
            local_path(path.to_str().unwrap(), None),
            Some(LocalPath { path, line: None })
        );
    }

    #[test]
    fn resolves_relative_path_against_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let subdir = dir.path().join("packages/app/src");
        std::fs::create_dir_all(&subdir).unwrap();
        let path = subdir.join("diff.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();

        assert_eq!(
            local_path("packages/app/src/diff.rs:75", Some(dir.path())),
            Some(LocalPath {
                path,
                line: Some(75)
            })
        );
    }

    #[test]
    fn decodes_file_url() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("with space.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let encoded = url::Url::from_file_path(&path).unwrap();

        assert_eq!(
            local_path(&format!("{encoded}:75"), None),
            Some(LocalPath {
                path,
                line: Some(75)
            })
        );
    }

    #[test]
    fn declines_external_urls_and_anchors() {
        assert_eq!(local_path("https://example.com/a.rs:75", None), None);
        assert_eq!(local_path("mailto:a@example.com", None), None);
        assert_eq!(local_path("#local-heading", None), None);
    }

    #[test]
    fn a_bare_name_is_a_path_only_when_it_exists_under_cwd() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README"), "hi").unwrap();
        assert!(local_path("README", Some(dir.path())).is_some());
        assert_eq!(local_path("CHANGELOG", Some(dir.path())), None);
        assert_eq!(local_path("README", None), None);
    }

    /// The Codex `view_image` shape: a bare absolute path, no scheme. The
    /// platform opener refuses it, so it must classify as a local image.
    #[test]
    fn a_bare_absolute_image_path_is_a_local_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.PNG");
        std::fs::write(&path, b"not really a png").unwrap();

        assert_eq!(
            classify(path.to_str().unwrap(), None),
            LinkTarget::Local {
                path,
                line: None,
                kind: LocalKind::Image,
            }
        );
    }

    #[test]
    fn kinds_follow_the_probe_then_the_extension() {
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join("notes.txt");
        let unknown = dir.path().join("blob.xyz");
        let pdf = dir.path().join("paper.pdf");
        let sub = dir.path().join("out");
        for f in [&text, &unknown, &pdf] {
            std::fs::write(f, "x").unwrap();
        }
        std::fs::create_dir(&sub).unwrap();

        let kind = |p: &Path| match classify(p.to_str().unwrap(), None) {
            LinkTarget::Local { kind, .. } => kind,
            other => panic!("expected a local target, got {other:?}"),
        };
        assert_eq!(kind(&text), LocalKind::Text);
        assert_eq!(kind(&unknown), LocalKind::Text);
        assert_eq!(kind(&pdf), LocalKind::Binary);
        assert_eq!(kind(&sub), LocalKind::Directory);
        assert_eq!(kind(&dir.path().join("gone.rs")), LocalKind::Missing);
    }

    /// A remote session's paths are never probed here — not even `file://`,
    /// which would otherwise open the local file of the same name.
    #[test]
    fn a_remote_session_keeps_urls_and_refuses_every_path() {
        assert_eq!(classify_remote("/tmp"), LinkTarget::Remote);
        assert_eq!(classify_remote("file:///tmp"), LinkTarget::Remote);
        // No local path form on any host, the way `file:///tmp` has none on
        // Windows — so this pins the rule on the platform the tests run on.
        assert_eq!(
            classify_remote("file://server/share/a.rs"),
            LinkTarget::Remote
        );
        assert_eq!(classify_remote("src/main.rs:12"), LinkTarget::Remote);
        assert_eq!(classify_remote("#heading"), LinkTarget::Opaque);
        assert_eq!(
            classify_remote("https://example.com"),
            LinkTarget::Web {
                url: "https://example.com".into()
            }
        );
    }

    /// A resource that is gone reports as missing rather than reading as a
    /// word — the silent no-op the button started out with.
    #[test]
    fn a_relative_resource_resolves_against_cwd_even_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            classify_resource("shot.png", None, Some(dir.path())),
            LinkTarget::Local {
                path: dir.path().join("shot.png"),
                line: None,
                kind: LocalKind::Missing,
            }
        );
        std::fs::write(dir.path().join("shot.png"), b"x").unwrap();
        assert!(matches!(
            classify_resource("shot.png", None, Some(dir.path())),
            LinkTarget::Local {
                kind: LocalKind::Image,
                ..
            }
        ));
        assert_eq!(
            classify_resource("shot.png", None, None),
            LinkTarget::Opaque
        );
        assert_eq!(
            classify_resource("https://example.com/a.png", None, None),
            LinkTarget::Web {
                url: "https://example.com/a.png".into()
            }
        );
    }

    /// The preview trusts a declared MIME over the extension, so the click
    /// has to as well — or a previewed image opens as text.
    #[test]
    fn a_declared_mime_decides_whether_a_resource_is_an_image() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("artifact"), b"x").unwrap();
        std::fs::write(dir.path().join("picture.png"), b"x").unwrap();
        let kind =
            |uri: &str, mime: Option<&str>| match classify_resource(uri, mime, Some(dir.path())) {
                LinkTarget::Local { path, kind, .. } => {
                    // A missing file previews nothing, so only a real one can disagree.
                    if kind != LocalKind::Missing {
                        assert_eq!(is_image_resource(&path, mime), kind == LocalKind::Image);
                    }
                    kind
                }
                other => panic!("expected a local target, got {other:?}"),
            };
        assert_eq!(kind("artifact", None), LocalKind::Text);
        assert_eq!(kind("artifact", Some("image/png")), LocalKind::Image);
        assert_eq!(kind("picture.png", None), LocalKind::Image);
        assert_eq!(kind("picture.png", Some("text/plain")), LocalKind::Binary);
        assert_eq!(kind("gone.png", Some("image/png")), LocalKind::Missing);
    }

    /// A share is refused before the disk is asked about it: on Windows the
    /// probe itself would authenticate to the attacker's host.
    #[cfg(windows)]
    #[test]
    fn a_network_share_is_never_probed() {
        for link in ["\\\\evil\\share\\a.png", "//evil/share/a.png"] {
            assert_eq!(resource_path(link, None), None, "{link}");
            assert_eq!(classify(link, None), LinkTarget::Opaque, "{link}");
            assert_eq!(
                classify_resource(link, None, None),
                LinkTarget::Opaque,
                "{link}"
            );
        }
    }

    #[test]
    fn a_resource_path_is_resolved_without_the_disk() {
        let cwd = Path::new("/work");
        assert_eq!(resource_path("a.png", Some(cwd)), Some(cwd.join("a.png")));
        assert_eq!(resource_path("a.png", None), None);
        assert_eq!(resource_path("mcp://server/a.png", Some(cwd)), None);
        assert_eq!(resource_path("https://example.com/a.png", Some(cwd)), None);
        assert_eq!(resource_path("file://server/share/a.png", Some(cwd)), None);
    }

    #[test]
    fn urls_are_web_and_anchors_are_opaque() {
        assert_eq!(
            classify("https://example.com/a.rs:75", None),
            LinkTarget::Web {
                url: "https://example.com/a.rs:75".into()
            }
        );
        assert_eq!(
            classify("mailto:a@example.com", None),
            LinkTarget::Web {
                url: "mailto:a@example.com".into()
            }
        );
        assert_eq!(classify("#local-heading", None), LinkTarget::Opaque);
        assert_eq!(classify("just-a-word", None), LinkTarget::Opaque);
    }
}
