//! Tool-call diff embeds: cache keys, rebuild fingerprints, the diff-model
//! builder and its line stats, and read-only diff editor creation.

use daruda_acp::DiffView;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{AppContext as _, Context, Entity};

use crate::view::AgentChatView;
use crate::window_access::WindowAccess;
use daruda_store::project::PaneId;
use daruda_ui::diff_editor::{DiffColors, DiffEditorModel, build_diff_editor_model};

/// Cache key for a tool call's `di`-th diff editor: one editor per file. Shared
/// with the renderer so the embed lookup matches the insert key.
pub(crate) fn diff_editor_key(tool_call_id: &str, di: usize) -> String {
    format!("{tool_call_id}#{di}")
}

/// Fingerprint of **every input** `build_diff_view_model` consumes for one
/// diff: its content, the `path` the editor's language comes from, and `theme`
/// from [`diff_theme_fingerprint`].
///
/// `reconcile_diff_editors` stores this alongside each built editor and compares
/// it against the current one on every pass, so one rule — "the fingerprint
/// moved" — covers every reason a built editor can be stale: a `ToolCallUpdate`
/// replaced the diff (a streaming write growing from a partial snapshot), the
/// path changed the language, or the theme swapped under it. Covering the theme
/// is what an earlier content-only fingerprint missed, and a diff embed cannot
/// recover from that on its own — see [`diff_theme_fingerprint`].
///
/// Not cryptographic — a same-key collision would only skip a rebuild it should
/// have done, an acceptable cost for a `DefaultHasher` over ordinary diff sizes.
pub(crate) fn diff_build_fingerprint(diff: &DiffView, theme: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    diff.path.hash(&mut hasher);
    diff.old_text.hash(&mut hasher);
    diff.new_text.hash(&mut hasher);
    theme.hash(&mut hasher);
    hasher.finish()
}

/// Fingerprint of the theme inputs every diff in one pass shares — the syntax
/// theme id, its light/dark variant, and the snapshotted palette.
///
/// A diff embed's colours are baked into `set_highlight_override` at build time,
/// and that override is what the editor reads *instead of* resolving
/// `cx.theme().highlight_theme` on every paint (`gpui_component`'s
/// `input/element.rs`) — deliberately, since an interleaved +/- buffer is not
/// valid source for any grammar. The consequence is that a built diff embed
/// cannot follow a theme swap the way an output embed does; only a rebuild moves
/// it. Folding this into [`diff_build_fingerprint`] is what makes the swap look
/// like any other staleness to the reconciler.
pub(crate) fn diff_theme_fingerprint(
    syntax_theme: &str,
    is_light: bool,
    colors: &DiffColors,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    syntax_theme.hash(&mut hasher);
    is_light.hash(&mut hasher);
    // `Hsla` is `f32`-based and so not `Hash`; hash the bit patterns. Equal
    // colours always share a bit pattern here — these are copied straight out of
    // the theme, never arithmetic results that could differ by a NaN or a signed
    // zero.
    for c in colors.channels() {
        c.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

/// Added / removed line counts for one tool-call diff, used by the fold summary
/// (`+N −M`) shown when the diff editor is collapsed. Counted from the *same*
/// hunks that build the diff editor (see [`build_diff_view_model`]), so the
/// numbers match what the editor renders exactly. Cached alongside the editor
/// in `AgentChatView.diff_stats`, keyed by [`diff_editor_key`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DiffStat {
    pub(crate) added: usize,
    pub(crate) removed: usize,
}

/// Language id for an editor's syntax tree, from the diff's file extension.
/// Resolves through the shared registry-backed resolver, so it never names a
/// language the highlighter cannot load.
pub(crate) fn diff_editor_language(diff: &DiffView) -> gpui::SharedString {
    daruda_ui::highlighter::language_for_extension(extension_str(&diff.path))
}

/// Convert a tool-call [`DiffView`] into the editor inputs the shared
/// diff-through-editor renderer consumes, plus the [`DiffStat`] for the same
/// diff. Pure / GPUI-free: builds the unified diff from `old_text`/`new_text`,
/// syntax-highlights and word-diffs the hunks exactly as the File viewer's
/// `load_diff` does, then folds them into a [`DiffEditorModel`].
///
/// The stat is counted from those *same* hunks (via [`diff_stat_from_hunks`]),
/// so it matches the rendered editor line-for-line.
///
/// Returns `None` when the two sides are identical (no hunks → nothing to
/// render), so the caller leaves the inline fallback in place and records no
/// stat entry (absent ≡ `0/0`).
///
/// The gutter shows line numbers only for a full-file diff (`old_text` is
/// `None` — a file *creation* / whole-file write, where the numbers are the
/// real file lines). An `Edit` sends only the replaced snippet as
/// `old_text`/`new_text`, so its diff would be numbered from 1 regardless of
/// where in the file the edit lands (the ACP `Diff` carries no line offset) —
/// those numbers would mislead, so they are hidden.
pub(crate) fn build_diff_view_model(
    diff: &DiffView,
    syntax_theme: &str,
    is_light: bool,
    colors: &DiffColors,
) -> Option<(DiffEditorModel, DiffStat)> {
    use daruda_content::diff::line_diff::unified_diff_text;
    use daruda_content::diff::word_diff::apply_word_diff;
    use daruda_content::diff::{build_diff_rows, parse_diff_hunks};
    use daruda_ui::row_highlight::{LanguageHint, highlight_hunks};

    let old = diff.old_text.as_deref().unwrap_or("");
    let text = unified_diff_text(old, &diff.new_text);
    let mut hunks = parse_diff_hunks(&text);
    if hunks.is_empty() {
        return None;
    }
    // Count add/remove from the parsed hunks before they are highlighted /
    // word-diffed (those passes only annotate, never reclassify lines), so the
    // stat is from the exact same diff that builds the editor below.
    let stat = diff_stat_from_hunks(&hunks);
    let ext = extension_str(&diff.path);
    highlight_hunks(&mut hunks, LanguageHint::Extension(ext));
    apply_word_diff(&mut hunks);
    let no_newline = crate::strings::agent_chat::diff_no_newline();
    let rows = build_diff_rows(&hunks, false, &no_newline);
    let show_line_numbers = diff.old_text.is_none();
    let syntax = daruda_ui::theme::syntax_theme_of(
        daruda_ui::theme::SyntaxPalette::from_config_name(syntax_theme),
        is_light,
    );
    Some((
        build_diff_editor_model(&rows, colors, &syntax, show_line_numbers),
        stat,
    ))
}

/// Tally a [`DiffStat`] from parsed diff hunks. Pure / GPUI-free wrapper over
/// the File viewer's `count_diff_stats`, which counts `DiffLine::Added` vs
/// `DiffLine::Removed` across the hunks — the same line classification the
/// editor rows are built from.
pub(super) fn diff_stat_from_hunks(hunks: &[daruda_content::diff::DiffHunk]) -> DiffStat {
    let (added, removed) = daruda_content::diff::count_diff_stats(hunks);
    DiffStat { added, removed }
}

/// Create + configure a read-only diff editor entity against the live window
/// `access` resolves. Mirrors the File viewer's editor construction
/// (`multi_line` + `code_editor`) and the diff-config it applies
/// (`set_disabled(true)` for read-only + decorations + injected highlight
/// spans), but wrapped where the viewer scrolls sideways — an embed has no
/// width of its own to spend. Returns `None` if the owning window is gone.
///
/// The by-handle half of [`WindowAccess`] uses the view's stored handle rather
/// than `WindowRegistry::handle_for_workspace(cx.entity_id())` because after the
/// pane became its own entity `cx.entity_id()` is the view, not the Workspace,
/// so the registry would no longer resolve the window.
pub(crate) fn create_diff_editor(
    cx: &mut Context<AgentChatView>,
    access: &mut WindowAccess<'_>,
    pane_id: PaneId,
    language: &str,
    model: DiffEditorModel,
) -> Option<Entity<daruda_ui::InputState>> {
    let language = language.to_owned();
    match access.with(cx, move |window, cx_w| {
        cx_w.new(|cx_state| {
            // One synthetic-buffer line per decoration (no trailing newline in
            // `model.text`), so this is the editor's display-row count until its
            // first paint wraps it to a width. Seeding it makes `display_rows()`
            // size the (parent-height-less) editor to its content on that first
            // render instead of a collapsed single line.
            let rows = model.decorations.len().max(1);
            // `language` selects CodeEditor mode (gutter, indent guides), not
            // the colours: `set_highlight_override` below short-circuits the
            // tree-sitter path entirely, because a synthetic +/- diff buffer
            // is not valid source for any grammar.
            let mut state = daruda_ui::InputState::new(window, cx_state)
                .multi_line(true)
                .soft_wrap(true)
                .code_editor(language)
                .rows(rows);
            state.set_value(model.text, window, cx_state);
            state.set_disabled(true, cx_state);
            state.set_line_decorations(model.decorations, cx_state);
            state.set_highlight_override(Some(model.highlights), cx_state);
            state
        })
    }) {
        Ok(editor) => Some(editor),
        Err(e) => {
            // Window gone mid-stream — drop this editor; the inline fallback
            // renders. Logged so it isn't a silent no-op.
            daruda_store::observability::log_writer::LogWriter::log(
                ErrorReport::new("Failed to build agent-chat diff editor")
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .with_context("error", format!("{e}"))
                    .dedup(format!("agent_chat.diff_editor.window_gone.{pane_id}"))
                    .build(),
            );
            None
        }
    }
}

/// A file extension as `&str`, or `""` when absent or not UTF-8.
fn extension_str(path: &std::path::Path) -> &str {
    path.extension().and_then(|ext| ext.to_str()).unwrap_or("")
}
