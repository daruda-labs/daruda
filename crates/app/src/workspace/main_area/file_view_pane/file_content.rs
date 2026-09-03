//! Pane-area file-viewer content loader.
//!
//! GPUI-free helpers that run on `background_executor`. Read a file
//! from disk (or `git show :path` for staged content) and return a
//! [`PaneFileContent`] ready for the file viewer to render. Pure
//! I/O + parsing + highlighter glue — no GPUI types.
//!
//! [`load_file_content`] is the single public entry point; the
//! `load_raw` / `load_diff` helpers stay private and are selected
//! via [`FileViewMode`].

use super::highlighter::{LanguageHint, highlight_hunks, highlight_raw_rows};
use super::mermaid_theme::MermaidPalette;
use super::visual::RasterImage;
use super::word_diff::apply_word_diff;
use super::{
    FileViewMode, PaneFileContent, build_diff_rows, build_raw_rows, count_diff_stats,
    parse_diff_hunks,
};
use crate::path_ext::PathExt;

/// Result of a background file load.
///
/// Raw (non-markdown) text is destined for the pane's `InputState`
/// editor entity — the steady-state `PaneFileContent::LoadedRaw`
/// carries no data, so the text travels in this transport enum and is
/// fed into the editor exactly once by the load-completion handler.
pub(in crate::workspace) enum LoadOutcome {
    /// Content the viewer stores as-is (diff, markdown, error states),
    /// together with the Markdown bitmaps the resolve pass loaded — one per
    /// slot, in slot order, for the GPUI side to convert. Empty for every
    /// outcome but a successfully parsed Markdown file.
    Plain {
        content: PaneFileContent,
        rasters: Vec<Option<RasterImage>>,
    },
    /// Raw file text for the editor. Stored content becomes `LoadedRaw`.
    Raw { text: String },
}

impl LoadOutcome {
    /// A `Plain` outcome with no Markdown bitmaps behind it.
    fn plain(content: PaneFileContent) -> Self {
        Self::Plain {
            content,
            rasters: Vec::new(),
        }
    }
}

/// Load file content for the pane-area file viewer. Called from a background task.
#[allow(clippy::too_many_arguments)]
pub(in crate::workspace) fn load_file_content(
    wt_path: &std::path::Path,
    repo_root: Option<&std::path::Path>,
    path: &std::path::Path,
    staged: bool,
    mode: FileViewMode,
    file_status: Option<char>,
    syntax_theme: &str,
    mermaid_palette: &MermaidPalette,
) -> LoadOutcome {
    match mode {
        FileViewMode::Raw | FileViewMode::Preview => load_raw(
            wt_path,
            repo_root,
            path,
            staged,
            syntax_theme,
            mermaid_palette,
        ),
        FileViewMode::Changes => LoadOutcome::plain(load_diff(
            repo_root,
            path,
            staged,
            file_status,
            syntax_theme,
            mermaid_palette.dark,
        )),
    }
}

fn load_raw(
    wt_path: &std::path::Path,
    repo_root: Option<&std::path::Path>,
    path: &std::path::Path,
    staged: bool,
    syntax_theme: &str,
    mermaid_palette: &MermaidPalette,
) -> LoadOutcome {
    use crate::ui::theme;

    let bytes: Result<Vec<u8>, String> = if staged {
        if repo_root.is_none() {
            return LoadOutcome::plain(PaneFileContent::Error("No git repository root".to_owned()));
        }
        // git show :path requires a repo-root-relative path.
        // `path` is absolute (set at the left-dock entry point); strip the repo root
        // prefix.  For legacy relative paths (old session state) use as-is.
        let repo_rel: std::path::PathBuf = if path.is_absolute() {
            let r = repo_root.unwrap_or(wt_path);
            match path.strip_prefix(r) {
                Ok(rel) => rel.to_path_buf(),
                Err(_) => {
                    return LoadOutcome::plain(PaneFileContent::Error(format!(
                        "staged path {} is not inside repo root {}",
                        path.display(),
                        r.display()
                    )));
                }
            }
        } else {
            path.to_path_buf()
        };
        crate::lane::git::git_show_staged(wt_path, &repo_rel).map_err(|e| e.to_string())
    } else {
        // `path` is absolute when opened from the left dock; fall back via
        // LanePaths::from_git_status for legacy relative paths from old session state.
        let full: std::borrow::Cow<'_, std::path::Path> = if path.is_absolute() {
            std::borrow::Cow::Borrowed(path)
        } else {
            let wp = crate::lane::paths::LanePaths { wt_path, repo_root };
            std::borrow::Cow::Owned(wp.from_git_status(path))
        };
        if !full.exists() {
            return LoadOutcome::plain(PaneFileContent::Deleted);
        }
        std::fs::read(full.as_ref()).map_err(|e| e.to_string())
    };

    match bytes {
        Err(e) => LoadOutcome::plain(PaneFileContent::Error(e)),
        Ok(b) => {
            if b.contains(&0u8) {
                return LoadOutcome::plain(PaneFileContent::Binary);
            }
            let (text, byte_truncated) = if b.len() > theme::FILE_VIEWER_MAX_BYTES {
                let s = String::from_utf8_lossy(&b[..theme::FILE_VIEWER_MAX_BYTES]).into_owned();
                (s, true)
            } else {
                match String::from_utf8(b) {
                    Err(_) => return LoadOutcome::plain(PaneFileContent::Binary),
                    Ok(s) => (s, false),
                }
            };
            let ext = path.extension_str();

            if ext == "md" || ext == "markdown" {
                let mut blocks = super::markdown_viewer::parse_markdown(
                    &text,
                    syntax_theme,
                    !mermaid_palette.dark,
                );
                let base_dir = path.parent().map(std::path::Path::to_path_buf);
                let rasters = super::markdown_viewer::resolve_all(
                    &mut blocks,
                    &mut |url| {
                        let base_dir = base_dir.as_ref()?;
                        super::visual::load_image_source(url, base_dir)
                            .and_then(|bytes| super::visual::decode_image(&bytes))
                            .ok()
                    },
                    &mut |source| super::visual::render_mermaid_raster(source, mermaid_palette),
                );
                let all_lines: Vec<String> = text.lines().map(str::to_owned).collect();
                let total_count = all_lines.len();
                let mut raw_rows = build_raw_rows(&all_lines);
                highlight_raw_rows(
                    &mut raw_rows,
                    LanguageHint::Extension(ext),
                    syntax_theme,
                    !mermaid_palette.dark,
                );
                return LoadOutcome::Plain {
                    content: PaneFileContent::LoadedMarkdown {
                        blocks,
                        raw_rows,
                        total_count,
                        byte_truncated,
                    },
                    rasters,
                };
            }

            LoadOutcome::Raw { text }
        }
    }
}

/// Unstaged diff (index ↔ working tree) computed in-app via `imara-diff`.
/// Returns `None` when either side isn't readable UTF-8 text (binary file,
/// missing from the index, or an IO/path error) so the caller can fall
/// back to `git diff`.
fn in_app_unstaged_diff(repo: &std::path::Path, path: &std::path::Path) -> Option<String> {
    let rel = path.strip_prefix(repo).unwrap_or(path);
    let old = crate::lane::git::git_show_staged(repo, rel).ok()?;
    let old = String::from_utf8(old).ok()?;
    let new = std::fs::read_to_string(path).ok()?;
    Some(super::line_diff::unified_diff_text(&old, &new))
}

fn load_diff(
    repo_root: Option<&std::path::Path>,
    path: &std::path::Path,
    staged: bool,
    file_status: Option<char>,
    syntax_theme: &str,
    diagram_dark: bool,
) -> PaneFileContent {
    if repo_root.is_none() {
        return PaneFileContent::Error("No git repository root".to_owned());
    }

    // Untracked files produce no output from `git diff`; use --no-index to
    // show the file content as entirely new (all added lines).
    let is_untracked = file_status == Some('?') && !staged;
    // `path` is absolute when opened from the left dock.  git diff accepts
    // absolute paths when run from the repo root, so we pass it directly.
    // For legacy relative paths from old session state, behaviour is unchanged.
    let repo = repo_root.expect("repo_root is Some: is_git() was checked at function entry");
    let diff_result = if is_untracked {
        crate::lane::git::git_diff_untracked(repo, path)
    } else if !staged {
        // Unstaged diff computed in-app with the Histogram algorithm
        // (index ↔ working tree, matching `git diff` with no options).
        // Any failure (binary, unreadable, missing from index, path issue)
        // falls back to `git diff` so behaviour never regresses.
        match in_app_unstaged_diff(repo, path) {
            Some(text) => Ok(text),
            None => crate::lane::git::git_diff(repo, path, staged),
        }
    } else {
        crate::lane::git::git_diff(repo, path, staged)
    };

    match diff_result {
        Err(e) => PaneFileContent::Error(e.to_string()),
        Ok(text) => {
            if text.contains("Binary files") {
                return PaneFileContent::Binary;
            }
            let mut hunks = parse_diff_hunks(&text);

            // Syntax highlighting (file extension → language detection).
            let ext = path.extension_str();
            highlight_hunks(
                &mut hunks,
                LanguageHint::Extension(ext),
                syntax_theme,
                !diagram_dark,
            );

            // Word-level diff for adjacent Removed/Added pairs.
            apply_word_diff(&mut hunks);

            let (added, removed) = count_diff_stats(&hunks);
            let rows_all = build_diff_rows(&hunks, false);
            let rows_no_ctx = build_diff_rows(&hunks, true);
            PaneFileContent::LoadedDiff {
                rows_all,
                rows_no_ctx,
                added,
                removed,
            }
        }
    }
}
