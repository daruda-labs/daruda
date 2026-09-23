//! Where a link inside a pane points — one answer shared by the left click,
//! the context menu and the resource-image preview, so a menu cannot offer
//! an opener the click declines. GPUI-free; the only filesystem access is a
//! `metadata` probe on the resolved path, because the file's *kind* decides
//! who opens it and the kind cannot be read off the link text (`./out` may
//! be a directory).

use std::path::{Path, PathBuf};

/// What a link resolves to once the pane's working directory is known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum LinkTarget {
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
pub(in crate::workspace) enum LocalKind {
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

/// Extensions the in-app decoder handles (`visual::decode_image`); the same
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

pub(in crate::workspace) fn is_image_extension(extension: &str) -> bool {
    let ext = extension.to_ascii_lowercase();
    IMAGE_EXTENSIONS.contains(&ext.as_str())
}

fn is_binary_extension(extension: &str) -> bool {
    let ext = extension.to_ascii_lowercase();
    BINARY_EXTENSIONS.contains(&ext.as_str())
}

/// Whether a link points outside the filesystem. Shared with the classifier
/// so the menu and the click cannot disagree about what a link is.
pub(in crate::workspace) fn is_external_url(link: &str) -> bool {
    link.contains("://")
        || link.starts_with("mailto:")
        || link.starts_with("tel:")
        || link.starts_with('#')
}

/// Classify `link` as the pane with working directory `cwd` sees it.
pub(in crate::workspace) fn classify(link: &str, cwd: Option<&Path>) -> LinkTarget {
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
pub(in crate::workspace) fn classify_remote(link: &str) -> LinkTarget {
    if link.starts_with('#') {
        LinkTarget::Opaque
    } else if file_url_path(link).is_some() || !is_external_url(link) {
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
pub(in crate::workspace) fn classify_resource(uri: &str, cwd: Option<&Path>) -> LinkTarget {
    if uri.starts_with('#') {
        return LinkTarget::Opaque;
    }
    let path = if let Some(path) = file_url_path(uri) {
        path
    } else if is_external_url(uri) {
        return LinkTarget::Web {
            url: uri.to_string(),
        };
    } else if Path::new(uri).is_absolute() {
        PathBuf::from(uri)
    } else if let Some(cwd) = cwd {
        cwd.join(uri)
    } else {
        return LinkTarget::Opaque;
    };
    let kind = kind_of(&path);
    LinkTarget::Local {
        path,
        line: None,
        kind,
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
    let path = if let Some(path) = file_url_path(link) {
        path
    } else if is_external_url(link) {
        return None;
    } else {
        let path = PathBuf::from(link);
        if path.is_absolute() {
            path
        } else if link.starts_with("./")
            || link.starts_with("../")
            || link.contains('/')
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

fn file_url_path(link: &str) -> Option<PathBuf> {
    let url = url::Url::parse(link).ok()?;
    if url.scheme() != "file" || url.host_str().is_some_and(|host| host != "localhost") {
        return None;
    }
    url.to_file_path().ok()
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
        let subdir = dir.path().join("crates/app/src");
        std::fs::create_dir_all(&subdir).unwrap();
        let path = subdir.join("diff.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();

        assert_eq!(
            local_path("crates/app/src/diff.rs:75", Some(dir.path())),
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
            classify_resource("shot.png", Some(dir.path())),
            LinkTarget::Local {
                path: dir.path().join("shot.png"),
                line: None,
                kind: LocalKind::Missing,
            }
        );
        std::fs::write(dir.path().join("shot.png"), b"x").unwrap();
        assert!(matches!(
            classify_resource("shot.png", Some(dir.path())),
            LinkTarget::Local {
                kind: LocalKind::Image,
                ..
            }
        ));
        assert_eq!(classify_resource("shot.png", None), LinkTarget::Opaque);
        assert_eq!(
            classify_resource("https://example.com/a.png", None),
            LinkTarget::Web {
                url: "https://example.com/a.png".into()
            }
        );
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
