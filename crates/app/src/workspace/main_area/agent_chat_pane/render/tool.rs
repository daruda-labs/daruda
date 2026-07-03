//! Tool-call cards (title + status badge + foldable body of diffs and output)
//! and the inline permission cards with their per-choice buttons.

use daruda_acp::{
    PermissionChoice, PermissionItem, PermissionKindView, PermissionResolution, ToolCallItem,
    ToolOutputBlock, ToolStatusView,
};
use gpui::{AnyElement, App, Hsla, IntoElement, SharedString, div, prelude::*, px};

use super::chrome::pulse_dots;
use super::diff::diff_block;
use super::{DiffEditors, DiffStats, foldable_block};
use crate::surface::strings as s;
use crate::ui::theme;
use crate::workspace::main_area::agent_chat_pane::agent_chat_ops::diff_editor_key;
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
    diff_editors: &DiffEditors,
    diff_stats: &DiffStats,
    fold: &FoldState,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let (badge_text, badge_fg) = tool_status_badge(tc.status, t, cx);
    // A running tool gets animated trailing dots (Running. / .. / ...) so the
    // in-progress state reads as live, not just a static amber label.
    let badge_text = if matches!(tc.status, ToolStatusView::InProgress) {
        SharedString::from(format!("{badge_text}{}", pulse_dots(cx)))
    } else {
        badge_text
    };

    // Title + status badge: the header IS the summary, so the title fills the
    // row and the badge pins to the right.
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
                .text_color(theme::agent_chat_fg(cx))
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(
                    crate::ui::selectable_text(
                        SharedString::from(format!("agent-chat-tool-title-{}", tc.id)),
                        tc.title.clone(),
                    )
                    .color(theme::agent_chat_fg(cx))
                    .text_size(px(theme::agent_chat_font_size(cx))),
                ),
        )
        .child(
            div()
                .flex_none()
                .text_color(badge_fg)
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(badge_text),
        )
        .into_any_element();

    // Body: nested diffs (each independently foldable) then plain-text output.
    let mut body = div().flex().flex_col().gap(px(theme::AGENT_CHAT_MSG_GAP));
    for (di, diff) in tc.diffs.iter().enumerate() {
        let editor = diff_editors.get(&diff_editor_key(&tc.id, di));
        body = body.child(diff_block(
            &tc.id, di, diff, editor, diff_stats, fold, t, cx,
        ));
    }
    if !tc.output.is_empty() {
        body = body.child(
            div()
                .text_color(theme::agent_chat_fg_muted(cx))
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(SharedString::from(s::agent_chat_tool_output_label())),
        );
        for (ix, block) in tc.output.iter().enumerate() {
            body = body.child(output_block_view(&tc.id, ix, block, cx));
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
        .bg(theme::agent_chat_tint(cx))
        .border_1()
        .border_color(theme::agent_chat_border_tint(cx))
        .child(foldable_block(
            SharedString::from(format!("agent-chat-tool-{}", tc.id)),
            key,
            expanded,
            header,
            None,
            body.into_any_element(),
            |row| row,
            cx,
        ))
}

/// Render one tool-output block: rendered markdown (drag-selectable, keyed per
/// block for stable selection state), or a resource link as an open button.
/// The ACP spec says clients SHOULD render tool text as Markdown; code blocks
/// keep their own monospace + syntax highlight.
fn output_block_view(tool_id: &str, ix: usize, block: &ToolOutputBlock, cx: &App) -> AnyElement {
    match block {
        ToolOutputBlock::Text(text) => crate::ui::markdown(
            SharedString::from(format!("agent-chat-tool-out-{tool_id}-{ix}")),
            text.clone(),
        )
        .color(theme::agent_chat_fg(cx))
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

/// Map a tool status to its badge label + colour.
fn tool_status_badge(
    status: ToolStatusView,
    t: &theme::DarudaTheme,
    cx: &App,
) -> (SharedString, Hsla) {
    match status {
        ToolStatusView::Pending => (
            s::agent_chat_tool_status_pending().into(),
            theme::agent_chat_fg_muted(cx),
        ),
        // Amber accent so a running tool reads stronger than a settled
        // green ✓ / red ✗; `tool_card` appends animated dots to the label.
        ToolStatusView::InProgress => (
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
            theme::agent_chat_fg_muted(cx),
        ),
    }
}

/// Inline permission card — title + one button per choice. Once resolved,
/// the buttons are gone and the chosen option is shown instead.
pub(super) fn permission_card(
    ix: usize,
    card: &PermissionItem,
    t: &theme::DarudaTheme,
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
                .text_color(theme::agent_chat_fg(cx))
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(
                    crate::ui::selectable_text(("agent-chat-perm-title", ix), title)
                        .color(theme::agent_chat_fg(cx))
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
                    .text_color(theme::agent_chat_fg_muted(cx))
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
                    .text_color(theme::agent_chat_fg_muted(cx))
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
