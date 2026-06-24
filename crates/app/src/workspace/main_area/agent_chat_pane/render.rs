//! Pure view of an `&AgentChatContent` — the scrolling conversation and
//! inline permission cards. The prompt input lives in the shared bottom
//! dock (`send_terminal_input` routes to the focused AgentChat pane's
//! session), so this view carries no input field of its own.
//!
//! MVU view purity: every event closure is a one-line dispatch into a
//! `Workspace` op (`respond_agent_permission`). No state transition lives
//! here.
//!
//! Rendering notes:
//! - Assistant / user / thinking text: settled messages render as markdown
//!   via `render_md_blocks_plain` over the `md_blocks` cache that
//!   `reconcile_markdown` populates in the ops layer; the still-streaming
//!   tail renders as wrapped plain text until it settles and is parsed.
//! - Tool-call diffs embed the read-only diff-editor entities that
//!   `reconcile_diff_editors` builds from the diff ops (the `diff_editors`
//!   cache); when an editor can't be built (window gone) or the diff is
//!   identical, the card falls back to inline old/new colored monospace
//!   lines using the file-viewer diff palette.

use daruda_acp::{
    ChatItem, DiffView, PermissionChoice, PermissionItem, PermissionKindView, ToolCallItem,
    ToolStatusView,
};
use gpui::{AnyElement, Entity, Hsla, IntoElement, SharedString, div, prelude::*, px, relative};

/// Read-only diff editor entities keyed by `"{tool_call_id}#{diff_index}"`
/// (built in the ops layer; this view only embeds them).
type DiffEditors = std::collections::HashMap<String, Entity<gpui_component::input::InputState>>;

use crate::surface::strings as s;
use crate::ui::theme;
use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_pane::agent_chat_ops::diff_editor_key;
use crate::workspace::main_area::file_view_pane::markdown_viewer::MdBlock;
use crate::workspace::main_area::file_view_pane::render::render_md_blocks_plain;
use crate::workspace::main_area::pane::{AgentChatContent, AgentSessionStatus};
use crate::workspace::main_area::pane_tree::PaneId;

/// Build the element tree for an Agent chat pane.
pub(in crate::workspace) fn render(
    pane_id: PaneId,
    content: &AgentChatContent,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    // Clone the palette to an owned value so the render body can use `cx`
    // mutably (binding listeners) while reading theme colours — `current`
    // returns a borrow tied to `cx`.
    let t = theme::current(cx).clone();

    let status_banner = status_banner(&content.status, &t);

    let body: AnyElement = if content.items.is_empty() {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(theme::AGENT_CHAT_MSG_FONT_SIZE))
            .text_color(t.text_muted)
            .child(SharedString::from(s::agent_chat_empty()))
            .into_any_element()
    } else {
        let mut list = div()
            .id(("agent-chat-list", pane_id as usize))
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(theme::AGENT_CHAT_LIST_GAP))
            .px(px(theme::AGENT_CHAT_PAD_X))
            .py(px(theme::AGENT_CHAT_PAD_Y));
        for (ix, item) in content.items.iter().enumerate() {
            let blocks = content.md_blocks.get(&ix).map(Vec::as_slice);
            list = list.child(render_item(
                pane_id,
                ix,
                item,
                blocks,
                &content.diff_editors,
                &t,
                cx,
            ));
        }
        list.into_any_element()
    };

    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(t.file_viewer_bg)
        .children(status_banner)
        .child(body)
}

/// The thin top banner — shown while connecting or on error; hidden once
/// the session is live (the conversation itself signals readiness).
fn status_banner(
    status: &AgentSessionStatus,
    t: &theme::DarudaTheme,
) -> Option<impl IntoElement + use<>> {
    let (text, bg, fg): (SharedString, Hsla, Hsla) = match status {
        AgentSessionStatus::Connecting => (
            s::agent_chat_connecting().into(),
            t.banner_info_bg,
            t.banner_info_text,
        ),
        AgentSessionStatus::Connected => return None,
        AgentSessionStatus::Error(message) => (
            format!("{} {}", s::agent_chat_error_prefix(), message).into(),
            t.banner_error_bg,
            t.banner_error_text,
        ),
    };
    Some(
        div()
            .flex_none()
            .w_full()
            .px(px(theme::AGENT_CHAT_PAD_X))
            .py(px(theme::AGENT_CHAT_PAD_Y))
            .bg(bg)
            .text_color(fg)
            .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
            .child(text),
    )
}

/// One conversation row. `md_blocks` is the parsed Markdown for this item's
/// text when it has settled (filled by `reconcile_markdown` in the ops layer);
/// `None` means the text is still streaming and renders as plain wrapped text.
#[allow(clippy::too_many_arguments)]
fn render_item(
    pane_id: PaneId,
    ix: usize,
    item: &ChatItem,
    md_blocks: Option<&[MdBlock]>,
    diff_editors: &DiffEditors,
    t: &theme::DarudaTheme,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    match item {
        ChatItem::UserText(text) => user_bubble(text, md_blocks, t).into_any_element(),
        ChatItem::AssistantText { text, streaming } => {
            assistant_block(text, *streaming, md_blocks, t).into_any_element()
        }
        ChatItem::Thinking { text, streaming } => {
            thinking_block(text, *streaming, md_blocks, t).into_any_element()
        }
        ChatItem::ToolCall(tc) => tool_card(tc, diff_editors, t, cx).into_any_element(),
        ChatItem::Permission(card) => permission_card(pane_id, ix, card, t, cx).into_any_element(),
        ChatItem::Error(message) => error_block(message, t).into_any_element(),
    }
}

/// User prompt — right-aligned accent-tinted bubble. Renders Markdown once the
/// text has settled (it always has, for `UserText`); falls back to plain text
/// if the cache is somehow absent.
fn user_bubble(
    text: &str,
    md_blocks: Option<&[MdBlock]>,
    t: &theme::DarudaTheme,
) -> impl IntoElement + use<> {
    let inner = div()
        .max_w(relative(0.85))
        .px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
        .py(px(theme::AGENT_CHAT_INPUT_INNER_PAD_Y))
        .rounded(px(theme::AGENT_CHAT_INPUT_RADIUS))
        .bg(t.lane_card_active_bg)
        .text_color(t.text_primary)
        .text_size(px(theme::AGENT_CHAT_MSG_FONT_SIZE));
    let inner = match md_blocks {
        Some(blocks) => inner.child(render_md_blocks_plain(blocks, t)),
        None => inner.child(SharedString::from(text.to_string())),
    };
    div().flex().flex_row().justify_end().child(inner)
}

/// Assistant response — left-aligned block. A still-streaming block renders as
/// plain text with a trailing caret glyph; once it settles, `md_blocks` is
/// present and the body renders as Markdown.
fn assistant_block(
    text: &str,
    streaming: bool,
    md_blocks: Option<&[MdBlock]>,
    t: &theme::DarudaTheme,
) -> impl IntoElement + use<> {
    let body_el = match md_blocks {
        Some(blocks) => div()
            .text_color(t.text_primary)
            .text_size(px(theme::AGENT_CHAT_MSG_FONT_SIZE))
            .child(render_md_blocks_plain(blocks, t))
            .into_any_element(),
        None => {
            let body = if streaming {
                format!("{text}{}", s::AGENT_CHAT_STREAM_CARET)
            } else {
                text.to_string()
            };
            div()
                .text_color(t.text_primary)
                .text_size(px(theme::AGENT_CHAT_MSG_FONT_SIZE))
                .child(SharedString::from(body))
                .into_any_element()
        }
    };
    div()
        .flex()
        .flex_col()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .child(
            div()
                .text_color(t.text_muted)
                .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                .child(SharedString::from(s::agent_chat_label_agent())),
        )
        .child(body_el)
}

/// Agent reasoning — dimmed, italicised block under a "Thinking" label.
/// Streaming text stays plain (with a caret); a settled block renders as
/// Markdown, still dimmed and italicised.
fn thinking_block(
    text: &str,
    streaming: bool,
    md_blocks: Option<&[MdBlock]>,
    t: &theme::DarudaTheme,
) -> impl IntoElement + use<> {
    let body_el = match md_blocks {
        Some(blocks) => div()
            .italic()
            .text_color(t.text_subtle)
            .text_size(px(theme::AGENT_CHAT_MSG_FONT_SIZE))
            .child(render_md_blocks_plain(blocks, t))
            .into_any_element(),
        None => {
            let body = if streaming {
                format!("{text}{}", s::AGENT_CHAT_STREAM_CARET)
            } else {
                text.to_string()
            };
            div()
                .italic()
                .text_color(t.text_subtle)
                .text_size(px(theme::AGENT_CHAT_MSG_FONT_SIZE))
                .child(SharedString::from(body))
                .into_any_element()
        }
    };
    div()
        .flex()
        .flex_col()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .child(
            div()
                .text_color(t.text_subtle)
                .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                .child(SharedString::from(s::agent_chat_thinking_label())),
        )
        .child(body_el)
}

/// Surfaced error item — error-tinted block.
fn error_block(message: &str, t: &theme::DarudaTheme) -> impl IntoElement + use<> {
    div()
        .w_full()
        .px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
        .py(px(theme::AGENT_CHAT_INPUT_INNER_PAD_Y))
        .rounded(px(theme::AGENT_CHAT_INPUT_RADIUS))
        .bg(t.banner_error_bg)
        .text_color(t.banner_error_text)
        .text_size(px(theme::AGENT_CHAT_MSG_FONT_SIZE))
        .child(SharedString::from(message.to_string()))
}

/// Tool invocation card — title + status badge, optional diffs, optional
/// plain-text output.
fn tool_card(
    tc: &ToolCallItem,
    diff_editors: &DiffEditors,
    t: &theme::DarudaTheme,
    cx: &mut Context<Workspace>,
) -> impl IntoElement + use<> {
    let (badge_text, badge_fg) = tool_status_badge(tc.status, t);

    let mut card = div()
        .flex()
        .flex_col()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .w_full()
        .px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
        .py(px(theme::AGENT_CHAT_INPUT_INNER_PAD_Y))
        .rounded(px(theme::AGENT_CHAT_INPUT_RADIUS))
        .bg(t.lane_card_bg)
        .border_1()
        .border_color(t.border)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(theme::AGENT_CHAT_MSG_GAP))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(t.text_primary)
                        .text_size(px(theme::AGENT_CHAT_MSG_FONT_SIZE))
                        .child(SharedString::from(tc.title.clone())),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(badge_fg)
                        .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                        .child(badge_text),
                ),
        );

    for (di, diff) in tc.diffs.iter().enumerate() {
        let editor = diff_editors.get(&diff_editor_key(&tc.id, di));
        card = card.child(diff_block(diff, editor, t, cx));
    }

    if !tc.output.is_empty() {
        card = card.child(
            div()
                .text_color(t.text_muted)
                .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                .child(SharedString::from(s::agent_chat_tool_output_label())),
        );
        for block in &tc.output {
            card = card.child(
                div()
                    .font_family(theme::FONT_FAMILY_MONOSPACE)
                    .whitespace_normal()
                    .text_color(t.text_body)
                    .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                    .child(SharedString::from(block.clone())),
            );
        }
    }

    card
}

/// Diff for a tool-call file modification. When a read-only diff editor has
/// been built for this file (in the ops layer), embed it so the treatment
/// matches the File viewer exactly — gutter + syntax + word-diff backgrounds.
/// Falls back to inline old/new colored monospace lines when the editor is
/// absent (e.g. the two sides are identical, or the window was gone at build
/// time). The path header is shown either way.
fn diff_block(
    diff: &DiffView,
    editor: Option<&Entity<gpui_component::input::InputState>>,
    t: &theme::DarudaTheme,
    cx: &mut Context<Workspace>,
) -> impl IntoElement + use<> {
    let mut block = div()
        .flex()
        .flex_col()
        .w_full()
        .rounded(px(theme::RADIUS_XS))
        .overflow_hidden()
        .child(
            div()
                .px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
                .py(px(theme::GAP_XS))
                .bg(t.file_diff_hunk_bg)
                .text_color(t.file_diff_hunk_text)
                .font_family(theme::FONT_FAMILY_MONOSPACE)
                .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                .child(SharedString::from(diff.path.display().to_string())),
        );

    if let Some(editor) = editor {
        return block.child(
            div()
                .w_full()
                .bg(t.file_viewer_bg)
                .child(crate::ui::file_viewer_editor(editor, cx)),
        );
    }

    if let Some(old) = &diff.old_text {
        for line in old.lines() {
            block = block.child(diff_line(
                line,
                t.file_diff_del_bg,
                t.file_diff_del_text,
                '-',
            ));
        }
    }
    for line in diff.new_text.lines() {
        block = block.child(diff_line(
            line,
            t.file_diff_add_bg,
            t.file_diff_add_text,
            '+',
        ));
    }
    block
}

fn diff_line(line: &str, bg: Hsla, fg: Hsla, marker: char) -> impl IntoElement + use<> {
    div()
        .w_full()
        .px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
        .bg(bg)
        .text_color(fg)
        .font_family(theme::FONT_FAMILY_MONOSPACE)
        .whitespace_normal()
        .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
        .child(SharedString::from(format!("{marker} {line}")))
}

/// Map a tool status to its badge label + colour.
fn tool_status_badge(status: ToolStatusView, t: &theme::DarudaTheme) -> (SharedString, Hsla) {
    match status {
        ToolStatusView::Pending => (s::agent_chat_tool_status_pending().into(), t.text_muted),
        ToolStatusView::InProgress => (s::agent_chat_tool_status_running().into(), t.text_body),
        ToolStatusView::Completed => (
            s::agent_chat_tool_status_done().into(),
            t.file_diff_stat_add,
        ),
        ToolStatusView::Failed => (
            s::agent_chat_tool_status_failed().into(),
            t.banner_error_text,
        ),
    }
}

/// Inline permission card — title + one button per choice. Once resolved,
/// the buttons are gone and the chosen option is shown instead.
fn permission_card(
    pane_id: PaneId,
    ix: usize,
    card: &PermissionItem,
    t: &theme::DarudaTheme,
    cx: &mut Context<Workspace>,
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
                .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                .child(SharedString::from(s::agent_chat_permission_title())),
        )
        .child(
            div()
                .text_color(t.text_primary)
                .text_size(px(theme::AGENT_CHAT_MSG_FONT_SIZE))
                .child(title),
        );

    match &card.resolved {
        Some(option_id) => {
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
                    .text_color(t.text_muted)
                    .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                    .child(SharedString::from(format!(
                        "{} {}",
                        s::agent_chat_permission_resolved_prefix(),
                        chosen
                    ))),
            );
        }
        None => {
            let mut row = div().flex().flex_row().flex_wrap().gap(px(theme::GAP_SM));
            for (choice_ix, choice) in card.options.iter().enumerate() {
                row = row.child(permission_button(pane_id, ix, choice_ix, choice, cx));
            }
            root = root.child(row);
        }
    }

    root
}

/// One permission choice button. Allow kinds use the accent (primary)
/// treatment; reject kinds use the danger treatment.
fn permission_button(
    pane_id: PaneId,
    ix: usize,
    choice_ix: usize,
    choice: &PermissionChoice,
    cx: &mut Context<Workspace>,
) -> impl IntoElement + use<> {
    let id = ("agent-chat-perm", ix * 16 + choice_ix);
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
    button.on_click(cx.listener(move |ws, _, _window, cx| {
        ws.respond_agent_permission(pane_id, option_id.clone(), kind, cx);
    }))
}
