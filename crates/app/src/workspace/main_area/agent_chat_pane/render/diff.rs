//! Per-file diff blocks nested inside a tool card: the foldable diff header,
//! the expanded body (embedded read-only editor or inline old/new fallback),
//! the collapsed `+N −M` summary, and the inline fallback lines.

use daruda_acp::DiffView;
use gpui::{AnyElement, App, Entity, Hsla, IntoElement, SharedString, div, prelude::*, px};

use super::{DiffStats, foldable_block};
use crate::surface::strings as s;
use crate::ui::theme;
use crate::workspace::main_area::agent_chat_pane::agent_chat_ops::{DiffStat, diff_editor_key};
use crate::workspace::main_area::agent_chat_pane::fold::{FoldKey, FoldState};
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;

/// Diff for a tool-call file modification — foldable (default collapsed),
/// nested inside the tool card body. The header is the chevron + the file path
/// (single-line ellipsized), on the hunk-header bg chrome. Collapsed, the
/// summary shows `+N −M` from `diff_stats` (green added / red removed); a diff
/// with no stat entry (a no-change diff) shows nothing. Expanded body: when a
/// read-only diff editor has been built for this file (in the ops layer), embed
/// it so the treatment matches the File viewer exactly — gutter + syntax +
/// word-diff backgrounds. Falls back to inline old/new colored monospace lines
/// when the editor is absent (the two sides are identical, or the window was
/// gone at build time).
#[allow(clippy::too_many_arguments)]
pub(super) fn diff_block(
    tool_id: &str,
    di: usize,
    diff: &DiffView,
    editor: Option<&Entity<crate::ui::InputState>>,
    diff_stats: &DiffStats,
    fold: &FoldState,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let diff_key = diff_editor_key(tool_id, di);
    let key = FoldKey::Diff(diff_key.clone());
    // Diff policy is DefaultCollapsed → derivation ignores `active`.
    let expanded = fold.is_expanded(&key, false);

    let header = div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_color(t.file_diff_hunk_text)
        .font_family(theme::FONT_FAMILY_MONOSPACE)
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(SharedString::from(diff.path.display().to_string()))
        .into_any_element();

    // Collapsed summary: `+N −M`. Absent entry ≡ no changes → show nothing.
    let summary = diff_stats
        .get(&diff_key)
        .map(|stat| diff_stat_summary(stat, t, cx));

    let body = diff_body(diff, editor, t, cx).into_any_element();

    // The hunk-bg + padding chrome lives on the header row; the rounded /
    // overflow-hidden container wraps the whole foldable block. The body's own
    // backgrounds paint over the container, so only the header carries hunk-bg.
    div()
        .w_full()
        .rounded(px(theme::RADIUS_XS))
        .overflow_hidden()
        .child(foldable_block(
            SharedString::from(format!("agent-chat-diff-{diff_key}")),
            key,
            expanded,
            header,
            summary,
            body,
            |row| {
                row.px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
                    .py(px(theme::GAP_XS))
                    .bg(t.file_diff_hunk_bg)
            },
            cx,
        ))
}

/// The expanded body of a diff block: the embedded read-only editor when one
/// was built, an explicit "no changes" line when both sides are identical, or
/// the inline old/new colored monospace fallback otherwise.
fn diff_body(
    diff: &DiffView,
    editor: Option<&Entity<crate::ui::InputState>>,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let mut block = div().flex().flex_col().w_full();

    if let Some(editor) = editor {
        return block.child(
            div()
                .w_full()
                .bg(t.file_viewer_bg)
                .child(crate::ui::file_viewer_editor(editor, cx)),
        );
    }

    // No editor and the two sides are identical: the diff carries no changes,
    // so `build_diff_view_model` returned `None` and no editor was built.
    // Surface that explicitly rather than letting the inline fallback paint the
    // whole file red-then-green (which would read as a full delete + re-add).
    if diff.old_text.as_deref() == Some(diff.new_text.as_str()) {
        return block.child(
            div()
                .px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
                .py(px(theme::GAP_XS))
                .bg(t.file_viewer_bg)
                // Editor-surface island (mirrors the File viewer), so its text
                // follows the UI/editor muted color — NOT the terminal
                // foreground, which would render dark-on-dark here on a light
                // terminal theme since `file_viewer_bg` is an opaque fixed
                // editor surface (unlike the translucent pane tints).
                .text_color(t.text_muted)
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(SharedString::from(s::file_viewer_empty_diff())),
        );
    }

    if let Some(old) = &diff.old_text {
        for line in old.lines() {
            block = block.child(diff_line(
                line,
                t.file_diff_del_bg,
                t.file_diff_del_text,
                '-',
                cx,
            ));
        }
    }
    for line in diff.new_text.lines() {
        block = block.child(diff_line(
            line,
            t.file_diff_add_bg,
            t.file_diff_add_text,
            '+',
            cx,
        ));
    }
    block
}

/// The collapsed diff summary `+N −M`: added count in `file_diff_add_text`
/// (green), removed count in `file_diff_del_text` (red). Built from the
/// [`DiffStat`] the ops layer caches alongside the editor (absent ≡ `0/0`, in
/// which case the caller shows no summary at all).
fn diff_stat_summary(stat: &DiffStat, t: &theme::DarudaTheme, cx: &App) -> AnyElement {
    div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .font_family(theme::FONT_FAMILY_MONOSPACE)
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(
            div()
                .text_color(t.file_diff_add_text)
                .child(SharedString::from(format!("+{}", stat.added))),
        )
        .child(
            div()
                .text_color(t.file_diff_del_text)
                .child(SharedString::from(format!("−{}", stat.removed))),
        )
        .into_any_element()
}

fn diff_line(line: &str, bg: Hsla, fg: Hsla, marker: char, cx: &App) -> impl IntoElement + use<> {
    div()
        .w_full()
        .px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
        .bg(bg)
        .text_color(fg)
        .font_family(theme::FONT_FAMILY_MONOSPACE)
        .whitespace_normal()
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(SharedString::from(format!("{marker} {line}")))
}
