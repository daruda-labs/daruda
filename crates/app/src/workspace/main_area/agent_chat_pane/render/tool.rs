//! Tool-call cards (title + status badge + foldable body of diffs and output)
//! and the inline permission cards with their per-choice buttons.

use daruda_acp::{
    ChatItem, PermissionChoice, PermissionItem, PermissionKindView, PermissionResolution,
    ToolCallItem, ToolKindView, ToolOutputBlock, ToolStatusView,
};
use gpui::{AnyElement, App, Hsla, IntoElement, SharedString, div, prelude::*, px};

use super::chrome::pulse_dots;
use super::diff::diff_block;
use super::{DiffEditors, DiffStats, ToggleTarget, foldable_block};
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{Icon, IconName, Sizable as _};
use crate::workspace::main_area::agent_chat_pane::agent_chat_helpers::{
    diff_editor_key, fold_active, renders_raw_input,
};
use crate::workspace::main_area::agent_chat_pane::fold::{FoldKey, FoldState};
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;

/// Tool invocation card — foldable (default collapsed once done, expanded while
/// in progress). The header is the existing title + status-badge row, which
/// already reads as the summary, so no extra inline summary line is added. The
/// body (diffs + plain-text output) shows only when expanded; the card's
/// border / bg chrome wraps the fold assembly either way. The nested diffs are
/// independently foldable.
#[allow(clippy::too_many_arguments)]
pub(super) fn tool_card(
    key: FoldKey,
    expanded: bool,
    tc: &ToolCallItem,
    items: &[ChatItem],
    diff_editors: &DiffEditors,
    diff_stats: &DiffStats,
    fold: &FoldState,
    t: &theme::DarudaTheme,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let (badge_text, badge_fg) = tool_status_badge(tc.status, t, dim, cx);
    // A live tool gets animated trailing dots (Running. / .. / ...) so the
    // in-progress state reads as live, not just a static amber label. `Pending`
    // counts as live too (see `ToolStatusView::is_live`).
    let badge_text = if tc.status.is_live() {
        SharedString::from(format!("{badge_text}{}", pulse_dots(cx)))
    } else {
        badge_text
    };

    // Title + status badge: the header IS the summary. A tool-kind icon leads
    // (so a bash call reads differently from a file read at a glance), then the
    // title fills the row and the badge pins right. For an Execute (terminal)
    // tool the ACP `title` is the shell command, so it renders in the monospace
    // family — the ambient font cascades into the selectable text (zed's
    // command-code-block model), setting the command off from prose labels.
    let fg = theme::dim_toward_gray(theme::agent_chat_fg(cx), dim);
    let is_execute = matches!(tc.kind, ToolKindView::Execute);
    let failed = matches!(tc.status, ToolStatusView::Failed);
    let font_size = px(theme::agent_chat_font_size(cx));
    let header = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::GAP_SM))
                .child(Icon::new(tool_kind_icon(tc.kind)).xsmall().text_color(fg))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(fg)
                        .text_size(font_size)
                        .when(is_execute, |d| d.font_family(theme::FONT_FAMILY_MONOSPACE))
                        .child(
                            crate::ui::selectable_text(
                                SharedString::from(format!("agent-chat-tool-title-{}", tc.id)),
                                tool_title_summary(tc.kind, &tc.title),
                            )
                            .color(fg)
                            .text_size(font_size),
                        ),
                ),
        )
        // Detached shell command (`run_in_background: true`): the tool completes
        // immediately with an ack while the real process keeps running, so a
        // chip marks it as launched-in-background — otherwise the card is
        // indistinguishable from a normal one-shot command.
        .when(tc.is_background(), |d| {
            d.child(
                crate::ui::Badge::new(SharedString::from(s::agent_chat_tool_background()))
                    .bg_color(theme::dim_toward_gray(theme::agent_chat_tint(cx), dim))
                    .border_color(theme::dim_toward_gray(
                        theme::agent_chat_border_tint(cx),
                        dim,
                    ))
                    .text_color(theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim)),
            )
        })
        .child(
            div()
                .flex_none()
                .text_color(badge_fg)
                .text_size(font_size)
                .child(badge_text),
        )
        .into_any_element();

    // Body: an optional raw-input disclosure (generic tools), then nested diffs
    // (each independently foldable), then plain-text output.
    let mut body = div().flex().flex_col().gap(px(theme::AGENT_CHAT_MSG_GAP));
    if renders_raw_input(tc)
        && let Some(raw) = &tc.raw_input
    {
        // Collapsed-by-default disclosure so the detail is on tap without
        // cluttering the card. The pretty-print + selectable-text build only
        // when expanded: `foldable_block` drops the body when collapsed, so
        // building it then is wasted work on the render hot path (GPUI has no
        // partial redraw) for a large `raw_input` blob.
        let raw_key = FoldKey::ToolRawInput(tc.id.clone());
        let raw_expanded = fold.is_expanded(&raw_key, false);
        let raw_header = div()
            .text_color(theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim))
            .text_size(font_size)
            .child(SharedString::from(s::agent_chat_raw_input_label()))
            .into_any_element();
        let raw_json: AnyElement = if raw_expanded {
            let pretty = serde_json::to_string_pretty(raw).unwrap_or_else(|_| raw.to_string());
            div()
                .min_w_0()
                .font_family(theme::FONT_FAMILY_MONOSPACE)
                .text_color(theme::dim_toward_gray(theme::agent_chat_fg_subtle(cx), dim))
                .text_size(font_size)
                .child(
                    crate::ui::selectable_text(
                        SharedString::from(format!("agent-chat-tool-rawin-{}", tc.id)),
                        SharedString::from(pretty),
                    )
                    .text_size(font_size),
                )
                .into_any_element()
        } else {
            gpui::Empty.into_any_element()
        };
        body = body.child(foldable_block(
            SharedString::from(format!("agent-chat-rawin-{}", tc.id)),
            raw_key,
            raw_expanded,
            ToggleTarget::Chevron,
            raw_header,
            None,
            raw_json,
            |row| row,
            dim,
            cx,
        ));
    }
    for (di, diff) in tc.diffs.iter().enumerate() {
        let editor = diff_editors.get(&diff_editor_key(&tc.id, di));
        body = body.child(diff_block(
            &tc.id, di, diff, editor, diff_stats, fold, t, dim, cx,
        ));
    }
    if !tc.output.is_empty() {
        body = body.child(
            div()
                .text_color(theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim))
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(SharedString::from(s::agent_chat_tool_output_label())),
        );
        for (ix, block) in tc.output.iter().enumerate() {
            body = body.child(output_block_view(&tc.id, ix, block, dim, cx));
        }
    }

    // Subagent activity: the Claude adapter flattens a spawned subagent's inner
    // tool calls into this session, linking each to this Task/Agent call only
    // via `parent_tool_id`. `rows::project` skips those children in the main
    // flow; render them nested here (recursively — a child may itself spawn one)
    // so the subagent reads as one unit that folds with its parent card.
    let children: Vec<&ToolCallItem> = items
        .iter()
        .filter_map(|it| match it {
            ChatItem::ToolCall(c) if c.parent_tool_id.as_deref() == Some(tc.id.as_str()) => Some(c),
            _ => None,
        })
        .collect();
    if !children.is_empty() {
        body = body.child(
            div()
                .text_color(theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim))
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(SharedString::from(s::agent_chat_subagent_label())),
        );
        for child in children {
            let child_key = FoldKey::Tool(child.id.clone());
            let child_expanded = fold.is_expanded(&child_key, fold_active(&child_key, items));
            body = body.child(
                tool_card(
                    child_key,
                    child_expanded,
                    child,
                    items,
                    diff_editors,
                    diff_stats,
                    fold,
                    t,
                    dim,
                    cx,
                )
                .into_any_element(),
            );
        }
    }

    // Card chrome (border + bg) wraps the fold assembly; the header IS the
    // summary, so no separate inline summary line.
    div()
        .w_full()
        .px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
        .py(px(theme::AGENT_CHAT_INPUT_INNER_PAD_Y))
        .rounded(px(theme::AGENT_CHAT_INPUT_RADIUS))
        // Background-derived tint (not a fixed surface): a translucent lift
        // over the pane background so the card sits one step above it on any
        // background color / theme / opacity. Border is the same overlay one
        // step stronger, so the edge tracks the background too.
        .bg(theme::dim_toward_gray(theme::agent_chat_tint(cx), dim))
        .border_1()
        // A failed tool call gets an error-tinted dashed border so it reads as
        // failed at a glance, not just via the badge (mirrors zed's dashed
        // failure card). `t.banner_error_text` is already dimmed (t is the
        // dimmed theme); the normal border tint is a global-read, so it is
        // dim-wrapped here.
        .border_color(if failed {
            t.banner_error_text
        } else {
            theme::dim_toward_gray(theme::agent_chat_border_tint(cx), dim)
        })
        .when(failed, |d| d.border_dashed())
        .child(foldable_block(
            SharedString::from(format!("agent-chat-tool-{}", tc.id)),
            key,
            expanded,
            ToggleTarget::Chevron,
            header,
            None,
            body.into_any_element(),
            |row| row,
            dim,
            cx,
        ))
}

/// Render one tool-output block: rendered markdown (drag-selectable, keyed per
/// block for stable selection state), or a resource link as an open button.
/// The ACP spec says clients SHOULD render tool text as Markdown; code blocks
/// keep their own monospace + syntax highlight.
fn output_block_view(
    tool_id: &str,
    ix: usize,
    block: &ToolOutputBlock,
    dim: f32,
    cx: &App,
) -> AnyElement {
    match block {
        ToolOutputBlock::Text(text) => crate::ui::markdown(
            SharedString::from(format!("agent-chat-tool-out-{tool_id}-{ix}")),
            text.clone(),
        )
        .color(theme::dim_toward_gray(theme::agent_chat_fg(cx), dim))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .into_any_element(),
        ToolOutputBlock::ResourceLink { uri, name } => {
            let uri = uri.clone();
            crate::ui::button(
                SharedString::from(format!("agent-chat-tool-link-{tool_id}-{ix}")),
                SharedString::from(name.clone()),
            )
            .on_click(move |_, _, cx| cx.open_url(&uri))
            .into_any_element()
        }
    }
}

/// Collapse a multiline tool title to its first line + "…" so the card header
/// stays a single line. `Execute` (shell command) titles are exempt — the
/// command *is* the title and has no other home, so truncating would hide lines
/// the user needs; every other kind's title is a description whose second-line
/// detail (when it exists) is recoverable from the body / raw-input disclosure.
/// Mirrors zed's kind-gated title in `ToolCall::from_acp`. A single-line title —
/// or one whose only newline is trailing (no real second line) — is returned
/// unchanged, so the "…" only appears when content is actually hidden.
fn tool_title_summary(kind: ToolKindView, title: &str) -> String {
    if matches!(kind, ToolKindView::Execute) {
        return title.to_string();
    }
    match title.split_once('\n') {
        Some((first, rest)) if !rest.trim().is_empty() => format!("{first}…"),
        _ => title.to_string(),
    }
}

/// Map a tool kind to a leading header icon, so a tool call's type reads at a
/// glance (terminal vs read vs edit …), mirroring zed's kind-based icon.
fn tool_kind_icon(kind: ToolKindView) -> IconName {
    // The vendored `IconName` set has no pencil/edit glyph, so Edit falls back
    // to `File` (Read already uses `Eye`, so no visual collision).
    match kind {
        ToolKindView::Read => IconName::Eye,
        ToolKindView::Edit => IconName::File,
        ToolKindView::Delete => IconName::Delete,
        ToolKindView::Move => IconName::ArrowRight,
        ToolKindView::Search => IconName::Search,
        ToolKindView::Execute => IconName::SquareTerminal,
        ToolKindView::Think => IconName::Bot,
        ToolKindView::Fetch => IconName::Globe,
        ToolKindView::SwitchMode => IconName::Refresh,
        ToolKindView::Other => IconName::Settings2,
    }
}

/// Map a tool status to its badge label + colour.
fn tool_status_badge(
    status: ToolStatusView,
    t: &theme::DarudaTheme,
    dim: f32,
    cx: &App,
) -> (SharedString, Hsla) {
    match status {
        // `Pending` and `InProgress` both read as "running": the adapter marks
        // every call `Pending` until an SDK progress ping (which many tools
        // never get), and a live `Pending` always means an in-flight call in the
        // active turn (see `ToolStatusView::is_live`). Amber accent so a running
        // tool reads stronger than a settled green ✓ / red ✗; `tool_card`
        // appends animated dots to the label.
        ToolStatusView::Pending | ToolStatusView::InProgress => (
            s::agent_chat_tool_status_running().into(),
            t.status_executing_tool_dark,
        ),
        ToolStatusView::Completed => (
            s::agent_chat_tool_status_done().into(),
            t.file_diff_stat_add,
        ),
        ToolStatusView::Failed => (
            s::agent_chat_tool_status_failed().into(),
            t.banner_error_text,
        ),
        // Stopped before settling — muted like Pending (no error red, no
        // success green): it neither failed nor completed.
        ToolStatusView::Cancelled => (
            s::agent_chat_tool_status_cancelled().into(),
            theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim),
        ),
    }
}

/// Inline permission card — title + one button per choice. Once resolved,
/// the buttons are gone and the chosen option is shown instead.
pub(super) fn permission_card(
    ix: usize,
    card: &PermissionItem,
    t: &theme::DarudaTheme,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let title: SharedString = card
        .tool_title
        .clone()
        .unwrap_or_else(s::agent_chat_permission_title)
        .into();

    let mut root = div()
        .flex()
        .flex_col()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .w_full()
        .px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
        .py(px(theme::AGENT_CHAT_INPUT_INNER_PAD_Y))
        .rounded(px(theme::AGENT_CHAT_INPUT_RADIUS))
        .bg(t.banner_warning_bg)
        .child(
            div()
                .text_color(t.banner_warning_text)
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(SharedString::from(s::agent_chat_permission_title())),
        )
        .child(
            div()
                .text_color(theme::dim_toward_gray(theme::agent_chat_fg(cx), dim))
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(
                    crate::ui::selectable_text(("agent-chat-perm-title", ix), title)
                        .color(theme::dim_toward_gray(theme::agent_chat_fg(cx), dim))
                        .text_size(px(theme::agent_chat_font_size(cx))),
                ),
        );

    match &card.resolved {
        Some(PermissionResolution::Chosen(option_id)) => {
            // Resolved: surface the chosen option's name (fall back to its
            // id) instead of the buttons.
            let chosen = card
                .options
                .iter()
                .find(|o| &o.option_id == option_id)
                .map(|o| o.name.clone())
                .unwrap_or_else(|| option_id.clone());
            root = root.child(
                div()
                    .text_color(theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim))
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .child(SharedString::from(format!(
                        "{} {}",
                        s::agent_chat_permission_resolved_prefix(),
                        chosen
                    ))),
            );
        }
        Some(PermissionResolution::Cancelled) => {
            // The turn was cancelled before the user decided — drop the
            // buttons and surface that the request was cancelled.
            root = root.child(
                div()
                    .text_color(theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim))
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .child(SharedString::from(s::agent_chat_permission_cancelled())),
            );
        }
        None => {
            let mut row = div().flex().flex_row().flex_wrap().gap(px(theme::GAP_SM));
            for (choice_ix, choice) in card.options.iter().enumerate() {
                row = row.child(permission_button(ix, choice_ix, choice, cx));
            }
            root = root.child(row);
        }
    }

    root
}

/// One permission choice button. Allow kinds use the accent (primary)
/// treatment; reject kinds use the danger treatment.
fn permission_button(
    ix: usize,
    choice_ix: usize,
    choice: &PermissionChoice,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    // Distinct id per (item, choice) without the old `ix * 16 + choice_ix`
    // arithmetic, which collided once a card carried more than 16 choices.
    let id = SharedString::from(format!("agent-chat-perm-{ix}-{choice_ix}"));
    let label: SharedString = choice.name.clone().into();
    let kind = choice.kind;
    let option_id = choice.option_id.clone();

    let button = match kind {
        PermissionKindView::AllowOnce | PermissionKindView::AllowAlways => {
            crate::ui::button_primary(id, label)
        }
        PermissionKindView::RejectOnce | PermissionKindView::RejectAlways => {
            crate::ui::button_danger(id, label)
        }
    };
    button.on_click(cx.listener(move |this, _, _window, cx| {
        this.respond_permission(option_id.clone(), kind, cx);
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiline_non_execute_title_summarizes_to_first_line() {
        assert_eq!(
            tool_title_summary(ToolKindView::Read, "Read src/main.rs\nlines 1-40"),
            "Read src/main.rs…"
        );
    }

    #[test]
    fn single_line_title_is_unchanged() {
        assert_eq!(
            tool_title_summary(ToolKindView::Search, "Search TODO"),
            "Search TODO"
        );
    }

    #[test]
    fn trailing_only_newline_does_not_add_ellipsis() {
        // A trailing newline (or a blank/whitespace second line) hides no real
        // content, so the title is returned as-is — no misleading "…".
        assert_eq!(
            tool_title_summary(ToolKindView::Read, "Read foo\n"),
            "Read foo\n"
        );
        assert_eq!(
            tool_title_summary(ToolKindView::Read, "Read foo\n   "),
            "Read foo\n   "
        );
    }

    #[test]
    fn execute_title_keeps_every_line() {
        // The shell command is the title and has no other home — never truncate.
        let cmd = "cd /repo &&\n  cargo build\n  cargo test";
        assert_eq!(tool_title_summary(ToolKindView::Execute, cmd), cmd);
    }
}
