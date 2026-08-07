//! Per-file diff blocks nested inside a tool card: the foldable diff header,
//! the expanded body (embedded read-only editor or inline old/new fallback),
//! the collapsed `+N −M` badge, and the inline fallback lines.

use daruda_acp::DiffView;
use gpui::{
    AnyElement, AnyWindowHandle, App, Entity, Hsla, IntoElement, SharedString, Window, div,
    prelude::*, px,
};

use super::DiffStats;
use super::embed::bounded_editor_embed;
use super::fold_header::{FoldHeader, FoldRow};
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{ButtonVariants as _, Icon, Sizable as _, button_bare, copy_button};
use crate::window_registry::WindowRegistry;
use crate::workspace::main_area::agent_chat_pane::agent_chat_helpers::{DiffStat, diff_editor_key};
use crate::workspace::main_area::agent_chat_pane::fold::{FoldKey, FoldState};
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;
use crate::workspace::main_area::pane_tree::PaneId;

const ICON_CONTENT_COPY: &str = "icons/ui/content-copy.svg";
const ICON_CHECK: &str = "icons/ui/check.svg";
const ICON_OPEN_IN_NEW: &str = "icons/ui/open-in-new.svg";

/// Diff for a tool-call file modification — foldable (default collapsed),
/// nested inside the tool card body. The header is the chevron + the file path
/// (single-line ellipsized), on the hunk-header bg chrome. Collapsed, `+N −M`
/// from `diff_stats` (green added / red removed) stands in for the folded diff as
/// a right-anchored badge; a diff with no stat entry (a no-change diff) shows
/// nothing. Expanded body: when a
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
    dim: f32,
    pane_id: PaneId,
    window_handle: AnyWindowHandle,
    window: &mut Window,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let diff_key = diff_editor_key(tool_id, di);
    let key = FoldKey::Diff(diff_key.clone());
    // Diff policy is DefaultExpanded → derivation ignores `active` either way.
    let expanded = fold.is_expanded(&key, false);

    let path_string = diff.path.display().to_string();
    // The path is the block's identity, shown in both fold states, so it takes
    // the stretch slot; the truncation idiom lives in `fold_header`. It reads as
    // a link through cursor + hover underline. `GIT_RENAMED` is the existing
    // muted blue token closest to the desired link tone without borrowing from
    // the syntax palette. Clicking it opens the file in the pane-area file
    // viewer — dispatched through `Workspace::open_diff_in_file_view`, reached
    // via `WindowRegistry` since this self-owned view has no direct `Workspace`
    // handle (the same lookup its own pane context-menu builder uses,
    // `render/mod.rs`).
    let path_for_click = diff.path.clone();
    let path_for_external_open = diff.path.clone();
    let path_link = div()
        .id(SharedString::from(format!(
            "agent-chat-diff-path-{diff_key}"
        )))
        .cursor_pointer()
        .text_color(theme::dim_toward_gray(theme::GIT_RENAMED, dim))
        .hover(|s| s.underline())
        .font_family(theme::FONT_FAMILY_MONOSPACE)
        .text_size(px(theme::agent_chat_font_size(cx)))
        .tooltip(crate::ui::tooltip::text(s::diff_open_in_file_view()))
        .child(SharedString::from(path_string.clone()))
        .on_click(move |_, window, cx| {
            // Keep the click from bubbling to an ancestor click handler —
            // same defensive stop as `code_copy_button`'s, even though
            // `.toggle_on_chevron()` below means this row carries no
            // ambient handler today.
            cx.stop_propagation();
            let Some(ws) =
                WindowRegistry::workspace_for_window(window_handle, cx).and_then(|ws| ws.upgrade())
            else {
                return;
            };
            let path = path_for_click.clone();
            ws.update(cx, |ws, cx| {
                ws.open_diff_in_file_view(pane_id, path, window, cx)
            });
        });
    // `with_interactive_title`, not `with_title`: the path carries a click
    // handler, so it has to shrink-wrap its glyphs instead of spanning the
    // stretch slot (`fold_header` owns that geometry and explains why).
    let mut header = FoldHeader::with_interactive_title(path_link);
    // Open-externally action — launches the user's preferred editor (Settings
    // → External Editor) or the OS default handler. Same dispatch shape as
    // the path click, one Workspace method over. The tooltip names the editor
    // the click will actually launch, read from the same `Workspace` field the
    // action itself resolves so label and behaviour cannot disagree; with no
    // preferred editor the OS default handler runs and has no name to show.
    let external_editor = WindowRegistry::workspace_for_window(window_handle, cx)
        .and_then(|ws| ws.upgrade())
        .map(|ws| ws.read(cx).preferred_editor.clone());
    let open_externally_tooltip = match external_editor
        .as_deref()
        .and_then(daruda_config::external_editor_preset)
    {
        Some(preset) => s::diff_open_in_editor(preset.display_name),
        None => s::diff_open_externally(),
    };
    header = header.trailing(
        button_bare(SharedString::from(format!(
            "agent-chat-diff-open-externally-{diff_key}"
        )))
        // `button_bare`'s default `Secondary` variant fills with the UI theme's
        // raised surface, which reads as a black chip sitting on this header's
        // terminal-derived tint. `.ghost().xsmall()` is the icon-button chrome
        // the pane's own activity bar already uses (`chrome.rs`).
        .ghost()
        .xsmall()
        .icon(Icon::empty().path(ICON_OPEN_IN_NEW))
        .tooltip(open_externally_tooltip)
        .on_click(move |_, _window, cx| {
            cx.stop_propagation();
            let Some(ws) =
                WindowRegistry::workspace_for_window(window_handle, cx).and_then(|ws| ws.upgrade())
            else {
                return;
            };
            let path = path_for_external_open.clone();
            ws.update(cx, |ws, cx| ws.open_diff_externally(pane_id, path, cx));
        })
        .into_any_element(),
    );
    // Copy-path action — always-visible trailing icon (this pane's diffs are
    // few, unlike the hover-gated markdown code-block copy button).
    header = header.trailing(
        copy_button(
            SharedString::from(format!("agent-chat-diff-copy-path-{diff_key}")),
            SharedString::from(path_string),
            Icon::empty().path(ICON_CONTENT_COPY),
            SharedString::from(s::diff_copy_path()),
            Icon::empty().path(ICON_CHECK),
            SharedString::from(s::diff_path_copied()),
            window,
            cx,
        )
        // Same chrome as the open-externally button above, for the same reason.
        .ghost()
        .xsmall()
        .into_any_element(),
    );
    // `+N −M` is right-anchored, so it is a trailing badge — not the stretch
    // slot's collapsed summary it used to be routed through. Trailing content is
    // fold-state-independent, so the stat stays readable while the diff is open.
    if let Some(stat) = diff_stats.get(&diff_key) {
        header = header.trailing(diff_stat_summary(stat, t, cx));
    }

    // Background-derived tint (not the fixed UI `BG_RAISED` surface) so the
    // header matches the same terminal-preset background the rest of the
    // tool card chrome tracks (`tool.rs` / `plan.rs` card bg). Not part of
    // `t` (the pre-dimmed theme snapshot), so it needs its own dim wrap.
    // Computed up front (rather than inside the `chrome` closure below) because
    // that closure runs during `FoldRow::render`'s own `cx` borrow, and
    // `agent_chat_tint` needs its own immutable `cx` read.
    let header_bg = theme::dim_toward_gray(theme::agent_chat_tint(cx), dim);

    // The hunk-bg + padding chrome lives on the header row; the rounded /
    // overflow-hidden container wraps the whole foldable block. The body's own
    // backgrounds paint over the container, so only the header carries hunk-bg.
    //
    // `flex_none` is load-bearing: `overflow_hidden` zeroes this flex item's
    // automatic minimum size, and the chat list lays rows out at min-content
    // height (gpui `list.rs` `available_item_space`) — without it, the row's
    // measured height undercounts the diff and this container absorbs the
    // whole deficit, clipping the diff body / folded header.
    let block = FoldRow::block(
        SharedString::from(format!("agent-chat-diff-{diff_key}")),
        key,
        expanded,
        header,
        |cx| diff_body(diff, editor, t, &diff_key, dim, cx).into_any_element(),
    )
    .toggle_on_chevron()
    .chrome(move |row| {
        row.px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
            .py(px(theme::GAP_XS))
            .bg(header_bg)
    })
    .render(dim, cx);
    div()
        .w_full()
        .flex_none()
        .rounded(px(theme::RADIUS_XS))
        .overflow_hidden()
        .debug_selector(|| format!("agent-chat-diff-container-{diff_key}"))
        .child(block)
        .into_any_element()
}

/// The expanded body of a diff block: the embedded read-only editor when one
/// was built, an explicit "no changes" line when both sides are identical, or
/// the inline old/new colored monospace fallback otherwise.
fn diff_body(
    diff: &DiffView,
    editor: Option<&Entity<crate::ui::InputState>>,
    t: &theme::DarudaTheme,
    diff_key: &str,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let mut block = div().flex().flex_col().w_full();

    if let Some(editor) = editor {
        // A whole-file `Write` diff is as tall as the file, so it carries the
        // same unbounded-paint cost a tool-output embed does and takes the same
        // bound. The `diff-` prefix keeps this embed's element ids apart from
        // the output embed of the same tool call, whose key has the same shape.
        return block.child(bounded_editor_embed(
            &format!("diff-{diff_key}"),
            editor,
            None,
            t,
            dim,
            cx,
        ));
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
                // Matches the editor-embed background above (both empty-diff
                // and populated-diff cases paint the same terminal-derived
                // surface); text follows the terminal foreground so it stays
                // in the same matched fg/bg pair the terminal theme itself
                // guarantees contrast for, instead of a UI-muted colour that
                // could mismatch an opaque terminal-derived background.
                .bg(theme::dim_toward_gray(theme::agent_chat_bg(cx), dim))
                .text_color(theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim))
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
