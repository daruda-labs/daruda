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
//! - Assistant / user / thinking text: every message body renders as
//!   rendered, drag-selectable / copyable markdown via `crate::ui::markdown`
//!   (a `TextView` wrapper). Selection state is GPUI keyed-state, so each
//!   body's id is keyed by the item's index — stable because `items` is
//!   append-only. The collapsed summary stays plain text. Streaming bodies
//!   render their partial markdown fine; the streaming signal lives on the
//!   input dock, so no per-message caret is shown.
//! - Tool-call diffs embed the read-only diff-editor entities that
//!   `reconcile_diff_editors` builds from the diff ops (the `diff_editors`
//!   cache); when an editor can't be built (window gone) or the diff is
//!   identical, the card falls back to inline old/new colored monospace
//!   lines using the file-viewer diff palette.

use daruda_acp::{
    ChatItem, DiffView, PermissionChoice, PermissionItem, PermissionKindView, PermissionResolution,
    ToolCallItem, ToolStatusView,
};
use gpui::{
    AnyElement, ElementId, Entity, Hsla, IntoElement, SharedString, div, prelude::*, px, relative,
};

/// Read-only diff editor entities keyed by `"{tool_call_id}#{diff_index}"`
/// (built in the ops layer; this view only embeds them).
type DiffEditors = std::collections::HashMap<String, Entity<crate::ui::InputState>>;

/// Per-diff `+N −M` line counts keyed by `"{tool_call_id}#{diff_index}"`
/// (built in the ops layer; this view only reads them for the collapsed
/// diff summary).
type DiffStats = std::collections::HashMap<String, DiffStat>;

/// Rendered mermaid diagrams (GPU-ready [`CachedImage`]) keyed by source hash
/// (filled async in the ops layer). Shared `Arc<Mutex<…>>` so the
/// `code_block_render` closure — bound into `TextView`'s cached parse — reads
/// the *live* cache, not a snapshot (the image lands after parse; see
/// `AgentChatContent::mermaid_images`).
type MermaidImages = std::sync::Arc<
    std::sync::Mutex<
        std::collections::HashMap<
            u64,
            crate::workspace::main_area::file_view_pane::render::CachedImage,
        >,
    >,
>;

use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{ButtonVariants as _, Disclosure, IconName, Sizable as _, button_bare, disclosure};
use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_pane::agent_chat_ops::{
    DiffStat, diff_editor_key, is_active, mermaid_key,
};
use crate::workspace::main_area::agent_chat_pane::fold::{FoldKey, FoldState};
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

    // Expand-all / collapse-all chrome sits between the banner and the list,
    // but only once there is a conversation to fold.
    let fold_toolbar: Option<AnyElement> = (!content.items.is_empty())
        .then(|| fold_toolbar(pane_id, content.modes.as_ref(), &t, cx).into_any_element());

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
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&content.scroll_handle)
            .on_scroll_wheel(
                cx.listener(move |ws, _ev, _window, cx| ws.agent_chat_on_scroll(pane_id, cx)),
            )
            .flex()
            .flex_col()
            .gap(px(theme::AGENT_CHAT_LIST_GAP))
            .px(px(theme::AGENT_CHAT_PAD_X))
            .py(px(theme::AGENT_CHAT_PAD_Y));
        for (ix, item) in content.items.iter().enumerate() {
            list = list.child(render_item(
                pane_id,
                ix,
                item,
                &content.diff_editors,
                &content.diff_stats,
                &content.mermaid_images,
                &content.fold,
                &t,
                cx,
            ));
        }
        // Wrap the scroll region so the live-tracking scrollbar overlay can
        // sit over it: the overlay is absolute-fill, so its parent must be
        // `relative` and sized to the viewport (this `flex_1` body slot).
        div()
            .relative()
            .flex_1()
            .min_h(px(0.))
            .child(list)
            .children(agent_chat_scrollbar(pane_id, &content.scroll_handle, &t))
            .into_any_element()
    };

    // The scroll-to-bottom button overlays the list when the user has scrolled
    // up (follow mode released); the pane root's `relative` anchors it.
    let scroll_btn: Option<AnyElement> = (!content.stick_to_bottom && !content.items.is_empty())
        .then(|| scroll_to_bottom_button(pane_id, cx).into_any_element());

    div()
        .size_full()
        .relative()
        .flex()
        .flex_col()
        .bg(t.file_viewer_bg)
        .children(status_banner)
        .children(fold_toolbar)
        .child(body)
        .children(scroll_btn)
}

/// Floating "jump to bottom" affordance shown over the list when the user has
/// scrolled up (follow mode released). One-line dispatch into
/// `agent_chat_scroll_to_bottom` (render purity); positioned bottom-right via
/// the pane root's `relative`.
fn scroll_to_bottom_button(
    pane_id: PaneId,
    cx: &mut Context<Workspace>,
) -> impl IntoElement + use<> {
    div()
        .absolute()
        .bottom(px(theme::AGENT_CHAT_SCROLL_BTN_INSET))
        .right(px(theme::AGENT_CHAT_SCROLL_BTN_INSET))
        .child(
            button_bare(("agent-chat-scroll-bottom", pane_id as usize))
                .icon(IconName::ArrowDown)
                .on_click(cx.listener(move |ws, _ev, _window, cx| {
                    ws.agent_chat_scroll_to_bottom(pane_id, cx)
                })),
        )
}

/// daruda's thin scrollbar thumb for the conversation list — same chrome as the
/// files / git / file-viewer panes (`crate::ui::scrollbar::vertical_thumb`), so
/// it reads as one widget across the app rather than a stray gpui_component bar.
/// Geometry is read from the `ScrollHandle` at *render* time, so it only tracks
/// because `agent_chat_on_scroll` notifies on scroll. `None` when the content
/// fits (no thumb). Positioned `.right(..)`; the caller's parent is `.relative()`.
fn agent_chat_scrollbar(
    pane_id: PaneId,
    handle: &gpui::ScrollHandle,
    t: &theme::DarudaTheme,
) -> Option<AnyElement> {
    let viewport_h = handle.bounds().size.height;
    let content_h = viewport_h + handle.max_offset().y;
    crate::ui::scrollbar::vertical_thumb(
        ("agent-chat-scrollbar", pane_id as usize),
        viewport_h,
        content_h,
        handle.offset().y,
        px(0.),
        t.scrollbar_thumb,
        t.file_viewer_scrollbar_thumb_hover,
    )
}

/// A toolbar row with "Expand all" / "Collapse all" buttons on the right and
/// an optional mode-selector chip on the left. Dev-tool chrome that should
/// recede: ghost, `xsmall`, muted until hover. Each button / chip is a
/// one-line dispatch into a `Workspace` op (render purity — no logic here).
/// `justify_between` pushes the mode chip left and the fold controls right.
/// Shown only when the conversation is non-empty (the caller gates on
/// `content.items`).
fn fold_toolbar(
    pane_id: PaneId,
    modes: Option<&daruda_acp::ModeStateView>,
    t: &theme::DarudaTheme,
    cx: &mut Context<Workspace>,
) -> impl IntoElement + use<> {
    let expand = crate::ui::button(
        ("agent-chat-expand-all", pane_id as usize),
        SharedString::from(s::agent_chat_expand_all()),
    )
    .ghost()
    .xsmall()
    .on_click(cx.listener(move |ws, _ev, _window, cx| ws.set_all_agent_folds(pane_id, true, cx)));
    let collapse = crate::ui::button(
        ("agent-chat-collapse-all", pane_id as usize),
        SharedString::from(s::agent_chat_collapse_all()),
    )
    .ghost()
    .xsmall()
    .on_click(cx.listener(move |ws, _ev, _window, cx| ws.set_all_agent_folds(pane_id, false, cx)));

    // Left slot: the mode chip when modes are advertised and non-empty;
    // an empty div otherwise so `justify_between` still pushes the fold
    // controls to the right.
    let left: AnyElement = if let Some(m) = modes.filter(|m| !m.available.is_empty()) {
        super::mode_chip::mode_chip(pane_id, m, cx).into_any_element()
    } else {
        div().into_any_element()
    };

    // Right slot: expand / collapse fold controls.
    let right = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .child(expand)
        .child(collapse);

    div()
        .flex_none()
        .w_full()
        .flex()
        .flex_row()
        .justify_between()
        .items_center()
        .px(px(theme::AGENT_CHAT_PAD_X))
        .py(px(theme::AGENT_CHAT_PAD_Y))
        .text_color(t.text_muted)
        .child(left)
        .child(right)
}

/// The thin top banner — shown while connecting or on error; hidden once
/// the session is live (the conversation itself signals readiness).
fn status_banner(
    status: &AgentSessionStatus,
    t: &theme::DarudaTheme,
) -> Option<impl IntoElement + use<>> {
    let (text, bg, fg): (SharedString, Hsla, Hsla) = match status {
        AgentSessionStatus::Idle => (
            s::agent_chat_idle().into(),
            t.banner_info_bg,
            t.banner_info_text,
        ),
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

/// One conversation row. Message bodies render as selectable markdown via
/// `crate::ui::markdown`, keyed by `ix` for stable selection identity.
///
/// `fold` / `diff_stats` are read-only here (render purity): the foldable kinds
/// derive their expanded state purely via `fold.is_expanded(&key, active)` and
/// read the collapsed diff summary from `diff_stats`. Toggling routes through
/// `Workspace::toggle_agent_fold`, never mutating `content` in render.
#[allow(clippy::too_many_arguments)]
fn render_item(
    pane_id: PaneId,
    ix: usize,
    item: &ChatItem,
    diff_editors: &DiffEditors,
    diff_stats: &DiffStats,
    mermaid_images: &MermaidImages,
    fold: &FoldState,
    t: &theme::DarudaTheme,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    match item {
        ChatItem::UserText(text) => user_bubble(ix, text, mermaid_images, t).into_any_element(),
        ChatItem::AssistantText { text, .. } => {
            let key = FoldKey::Assistant(ix);
            let expanded = fold.is_expanded(&key, is_active(item));
            assistant_block(pane_id, ix, key, expanded, text, mermaid_images, t, cx)
                .into_any_element()
        }
        ChatItem::Thinking { text, .. } => {
            let key = FoldKey::Thinking(ix);
            let expanded = fold.is_expanded(&key, is_active(item));
            thinking_block(pane_id, ix, key, expanded, text, mermaid_images, t, cx)
                .into_any_element()
        }
        ChatItem::ToolCall(tc) => {
            let key = FoldKey::Tool(tc.id.clone());
            let expanded = fold.is_expanded(&key, is_active(item));
            tool_card(
                pane_id,
                key,
                expanded,
                tc,
                diff_editors,
                diff_stats,
                fold,
                t,
                cx,
            )
            .into_any_element()
        }
        ChatItem::Permission(card) => permission_card(pane_id, ix, card, t, cx).into_any_element(),
        ChatItem::Error(message) => error_block(message, t).into_any_element(),
    }
}

/// Shared assembly for the four foldable block kinds (treatment C): a left
/// chevron + clickable header row, an optional dimmed inline summary shown only
/// when collapsed, and a body shown only when expanded. The whole header row is
/// the click target (generous hit area); the [`disclosure`] chevron renders as
/// a pure indicator glyph with no click handler of its own, so it never
/// double-dispatches (a `disclosure` without `.on_toggle()` registers no click
/// listener — see gpui `paint_mouse_listeners`).
///
/// `header_chrome` styles the header row itself — `|row| row` for the bare
/// assistant / thinking / tool headers, or a closure that adds the diff's
/// hunk-bg + padding. Each kind owns its outer chrome (assistant / thinking:
/// none; tool: card border + bg; diff: hunk-bg header) by wrapping this output
/// and/or styling the header row through `header_chrome`.
#[allow(clippy::too_many_arguments)]
fn foldable_block<
    Id: Into<ElementId>,
    F: FnOnce(gpui::Stateful<gpui::Div>) -> gpui::Stateful<gpui::Div>,
>(
    id: Id,
    pane_id: PaneId,
    key: FoldKey,
    expanded: bool,
    header: AnyElement,
    summary: Option<AnyElement>,
    body: AnyElement,
    header_chrome: F,
    t: &theme::DarudaTheme,
    cx: &mut Context<Workspace>,
) -> impl IntoElement + use<Id, F> {
    // One base id yields both the row's click target and the chevron glyph's
    // identity, so the two stay distinct yet stable across renders.
    let base: ElementId = id.into();
    let chevron: Disclosure = disclosure((base.clone(), "chevron"), expanded).color(t.text_subtle);

    let mut header_row = div()
        .id((base, "row"))
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .cursor_pointer()
        .on_click(
            cx.listener(move |ws, _ev, _window, cx| ws.toggle_agent_fold(pane_id, key.clone(), cx)),
        )
        .child(chevron)
        .child(header);
    // The collapsed-only inline summary sits after the header content, on the
    // same row, and is dropped entirely when expanded (the body carries the
    // detail then).
    if !expanded && let Some(summary) = summary {
        header_row = header_row.child(summary);
    }
    let header_row = header_chrome(header_row);

    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .child(header_row)
        .when(expanded, move |this| this.child(body))
}

/// The collapsed-only inline summary for a text block (assistant / thinking):
/// the first non-empty line of `text`, dimmed (`t.text_subtle`) and
/// single-line ellipsized via `flex_1().min_w_0()` + `overflow_hidden()` — the
/// same truncation idiom the path / title elements use, so layout (not a
/// hardcoded char limit) does the ellipsizing. `italic` matches the thinking
/// block's treatment. `None` when the text has no non-empty line (nothing to
/// summarize).
fn collapsed_text_summary(text: &str, italic: bool, t: &theme::DarudaTheme) -> Option<AnyElement> {
    let line = text.lines().find(|l| !l.trim().is_empty())?;
    Some(
        div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .when(italic, |el| el.italic())
            .text_color(t.text_subtle)
            .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
            .child(SharedString::from(line.to_string()))
            .into_any_element(),
    )
}

/// The `code_block_render` hook for a chat markdown body: replace a
/// ` ```mermaid ` fence with its cached diagram bitmap, leaving every other code
/// block (and a not-yet-rasterized mermaid fence) to the default code rendering
/// by returning `None`. Captures a cheap clone of the images map (`Arc` values)
/// so the closure stays `Send + Sync + 'static` (the `TextView` requirement)
/// without borrowing or cloning the image bytes.
fn mermaid_code_block_render(
    mermaid_images: &MermaidImages,
) -> impl Fn(&str, &str, &mut gpui::Window, &mut gpui::App) -> Option<AnyElement> + Send + Sync + 'static
{
    let images = mermaid_images.clone();
    move |lang, source, _window, _cx| {
        if lang != "mermaid" {
            return None;
        }
        // Read the live shared cache (not a snapshot) — see `MermaidImages`.
        // Cloning the cached `CachedImage` is an `Arc` bump, so gpui reuses the
        // already-uploaded texture instead of re-uploading the bitmap.
        let image = images.lock().ok()?.get(&mermaid_key(source)).cloned()?;
        let diagram = image.block();
        // The diagram is a bitmap (not selectable), so overlay a hover-revealed
        // button that copies the mermaid source to the clipboard.
        let key = mermaid_key(source);
        let group = SharedString::from(format!("mermaid-{key}"));
        let src = source.to_string();
        Some(
            div()
                .relative()
                .group(group.clone())
                .child(diagram)
                .child(
                    div()
                        .absolute()
                        .top_1()
                        .right_1()
                        .invisible()
                        .group_hover(group, |s| s.visible())
                        .child(
                            button_bare(SharedString::from(format!("mermaid-copy-{key}")))
                                .icon(IconName::Copy)
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        src.clone(),
                                    ));
                                }),
                        ),
                )
                .into_any_element(),
        )
    }
}

/// User prompt — right-aligned accent-tinted bubble. The body renders as
/// selectable markdown via `crate::ui::markdown` inside the bubble chrome
/// (bg / padding / rounded), keyed by `ix` for stable selection identity.
/// Mermaid fences render as diagrams via the `code_block_render` hook.
fn user_bubble(
    ix: usize,
    text: &str,
    mermaid_images: &MermaidImages,
    t: &theme::DarudaTheme,
) -> impl IntoElement + use<> {
    let body = crate::ui::markdown(("agent-chat-md-user", ix), text.to_string())
        .color(t.text_primary)
        .text_size(px(theme::AGENT_CHAT_MSG_FONT_SIZE))
        .full_width(false)
        .code_block_render(mermaid_code_block_render(mermaid_images));
    let inner = div()
        .max_w(relative(0.85))
        .px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
        .py(px(theme::AGENT_CHAT_INPUT_INNER_PAD_Y))
        .rounded(px(theme::AGENT_CHAT_INPUT_RADIUS))
        .bg(t.lane_card_active_bg)
        .text_color(t.text_primary)
        .text_size(px(theme::AGENT_CHAT_MSG_FONT_SIZE))
        .child(body);
    div().flex().flex_row().justify_end().child(inner)
}

/// Assistant response — left-aligned, foldable block (default expanded). The
/// body renders as rendered, drag-selectable markdown via `crate::ui::markdown`
/// (keyed by `ix` for stable selection identity); a still-streaming block shows
/// its partial markdown fine (no per-message caret — the streaming signal lives
/// on the input dock). Collapsed, the header shows the first non-empty line of
/// `text`, dimmed and single-line ellipsized.
#[allow(clippy::too_many_arguments)]
fn assistant_block(
    pane_id: PaneId,
    ix: usize,
    key: FoldKey,
    expanded: bool,
    text: &str,
    mermaid_images: &MermaidImages,
    t: &theme::DarudaTheme,
    cx: &mut Context<Workspace>,
) -> impl IntoElement + use<> {
    let body_el = crate::ui::markdown(("agent-chat-md-assistant", ix), text.to_string())
        .color(t.text_body)
        .text_size(px(theme::AGENT_CHAT_MSG_FONT_SIZE))
        .code_block_render(mermaid_code_block_render(mermaid_images))
        .into_any_element();
    let header = div()
        .flex_none()
        .text_color(t.text_body)
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
        .child(SharedString::from(s::agent_chat_label_agent()))
        .into_any_element();
    let summary = collapsed_text_summary(text, false, t);
    foldable_block(
        ("agent-chat-assistant", ix),
        pane_id,
        key,
        expanded,
        header,
        summary,
        body_el,
        |row| row,
        t,
        cx,
    )
}

/// Agent reasoning — dimmed, foldable block under a "Thinking" label (default
/// collapsed once settled, expanded while streaming, handled by the fold
/// derivation). The body renders as rendered, drag-selectable markdown via
/// `crate::ui::markdown` (keyed by `ix`), dimmed via `t.text_subtle`. Collapsed,
/// the header shows the first non-empty line of `text`, dimmed italic.
//
// NOTE: the previous italic treatment of the body is not preserved —
// `crate::ui::markdown` (TextView) owns its own typography. The "Thinking"
// label plus the dimmer `text_subtle` colour still distinguish reasoning from
// the assistant body.
#[allow(clippy::too_many_arguments)]
fn thinking_block(
    pane_id: PaneId,
    ix: usize,
    key: FoldKey,
    expanded: bool,
    text: &str,
    mermaid_images: &MermaidImages,
    t: &theme::DarudaTheme,
    cx: &mut Context<Workspace>,
) -> impl IntoElement + use<> {
    let body_el = crate::ui::markdown(("agent-chat-md-thinking", ix), text.to_string())
        .color(t.text_subtle)
        .text_size(px(theme::AGENT_CHAT_MSG_FONT_SIZE))
        .code_block_render(mermaid_code_block_render(mermaid_images))
        .into_any_element();
    let header = div()
        .flex_none()
        .text_color(t.text_body)
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
        .child(SharedString::from(s::agent_chat_thinking_label()))
        .into_any_element();
    let summary = collapsed_text_summary(text, true, t);
    foldable_block(
        ("agent-chat-thinking", ix),
        pane_id,
        key,
        expanded,
        header,
        summary,
        body_el,
        |row| row,
        t,
        cx,
    )
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

/// Tool invocation card — foldable (default collapsed once done, expanded while
/// in progress). The header is the existing title + status-badge row, which
/// already reads as the summary, so no extra inline summary line is added. The
/// body (diffs + plain-text output) shows only when expanded; the card's
/// border / bg chrome wraps the fold assembly either way. The nested diffs are
/// independently foldable.
#[allow(clippy::too_many_arguments)]
fn tool_card(
    pane_id: PaneId,
    key: FoldKey,
    expanded: bool,
    tc: &ToolCallItem,
    diff_editors: &DiffEditors,
    diff_stats: &DiffStats,
    fold: &FoldState,
    t: &theme::DarudaTheme,
    cx: &mut Context<Workspace>,
) -> impl IntoElement + use<> {
    let (badge_text, badge_fg) = tool_status_badge(tc.status, t);

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
        )
        .into_any_element();

    // Body: nested diffs (each independently foldable) then plain-text output.
    let mut body = div().flex().flex_col().gap(px(theme::AGENT_CHAT_MSG_GAP));
    for (di, diff) in tc.diffs.iter().enumerate() {
        let editor = diff_editors.get(&diff_editor_key(&tc.id, di));
        body = body.child(diff_block(
            pane_id, &tc.id, di, diff, editor, diff_stats, fold, t, cx,
        ));
    }
    if !tc.output.is_empty() {
        body = body.child(
            div()
                .text_color(t.text_muted)
                .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                .child(SharedString::from(s::agent_chat_tool_output_label())),
        );
        for block in &tc.output {
            body = body.child(
                div()
                    .font_family(theme::FONT_FAMILY_MONOSPACE)
                    .whitespace_normal()
                    .text_color(t.text_body)
                    .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                    .child(SharedString::from(block.clone())),
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
        .bg(t.lane_card_bg)
        .border_1()
        .border_color(t.border)
        .child(foldable_block(
            SharedString::from(format!("agent-chat-tool-{}", tc.id)),
            pane_id,
            key,
            expanded,
            header,
            None,
            body.into_any_element(),
            |row| row,
            t,
            cx,
        ))
}

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
fn diff_block(
    pane_id: PaneId,
    tool_id: &str,
    di: usize,
    diff: &DiffView,
    editor: Option<&Entity<crate::ui::InputState>>,
    diff_stats: &DiffStats,
    fold: &FoldState,
    t: &theme::DarudaTheme,
    cx: &mut Context<Workspace>,
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
        .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
        .child(SharedString::from(diff.path.display().to_string()))
        .into_any_element();

    // Collapsed summary: `+N −M`. Absent entry ≡ no changes → show nothing.
    let summary = diff_stats
        .get(&diff_key)
        .map(|stat| diff_stat_summary(stat, t));

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
            pane_id,
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
            t,
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
    cx: &mut Context<Workspace>,
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
                .text_color(t.text_muted)
                .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
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

/// The collapsed diff summary `+N −M`: added count in `file_diff_add_text`
/// (green), removed count in `file_diff_del_text` (red). Built from the
/// [`DiffStat`] the ops layer caches alongside the editor (absent ≡ `0/0`, in
/// which case the caller shows no summary at all).
fn diff_stat_summary(stat: &DiffStat, t: &theme::DarudaTheme) -> AnyElement {
    div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .font_family(theme::FONT_FAMILY_MONOSPACE)
        .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
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
                    .text_color(t.text_muted)
                    .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
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
                    .text_color(t.text_muted)
                    .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                    .child(SharedString::from(s::agent_chat_permission_cancelled())),
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
