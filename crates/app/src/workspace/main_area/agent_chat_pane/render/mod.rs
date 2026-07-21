//! Pure view of an `&AgentChatView`.
//!
//! MVU view purity: event closures one-line dispatch into view ops, so state
//! changes stay outside render. Message selection keys use append-only item
//! indexes; tool diffs embed cached read-only editors when available.

mod blocks;
mod chrome;
mod diff;
mod plan;
mod tool;

use daruda_acp::{ChatItem, ToolStatusView};
use gpui::{
    AnyElement, App, ElementId, Entity, IntoElement, ListSizingBehavior, MouseButton, SharedString,
    canvas, div, list, prelude::*, px,
};

/// Read-only diff editor entities keyed by `"{tool_call_id}#{diff_index}"`
/// (built in the ops layer; this view only embeds them).
pub(super) type DiffEditors = std::collections::HashMap<String, Entity<crate::ui::InputState>>;

/// Per-diff `+N −M` line counts keyed by `"{tool_call_id}#{diff_index}"`
/// (built in the ops layer; this view only reads them for the collapsed
/// diff summary).
pub(super) type DiffStats = std::collections::HashMap<String, DiffStat>;

/// Rendered mermaid diagrams keyed by source hash. Shared so the cached
/// markdown code-block hook can see async image arrivals after parse.
pub(super) type MermaidImages = std::sync::Arc<
    std::sync::Mutex<
        std::collections::HashMap<
            u64,
            crate::workspace::main_area::file_view_pane::render::CachedImage,
        >,
    >,
>;

/// Decoded tool-output images keyed by base64-content hash (`tool_image_key`).
/// `Some` = decoded & GPU-ready; `None` = a cached decode failure. Shared so
/// `output_block_view` sees async decode arrivals landed by
/// `reconcile_tool_images`.
pub(super) type ToolImages = std::sync::Arc<
    std::sync::Mutex<
        std::collections::HashMap<
            u64,
            Option<crate::workspace::main_area::file_view_pane::render::CachedImage>,
        >,
    >,
>;

use blocks::{
    assistant_block, assistant_markdown, conclusion_block, error_block, thinking_block, user_bubble,
};
use chrome::{ActivityBarProps, activity_bar, status_banner, working_indicator};
use plan::plan_region;
use tool::{permission_card, tool_card};

use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{
    ContextMenuExt, Disclosure, IconName, PopupMenuItem, StatusPulseClock, active_text_selection,
    button_bare, disclosure, menu_builder,
};
use crate::workspace::main_area::agent_chat_pane::agent_chat_helpers::{
    DiffStat, activity_bar_title, is_active, summary_preview_line, tool_fold_key,
};
use crate::workspace::main_area::agent_chat_pane::fold::{FoldKey, FoldState};
use crate::workspace::main_area::agent_chat_pane::rows::{RenderRow, RowKind};
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;
use crate::workspace::main_area::pane_tree::PaneId;

/// Build the element tree for an Agent chat pane.
pub(in crate::workspace) fn render(
    view: &AgentChatView,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement {
    let pane_id = view.pane_id;
    let content = view;
    // Own the palette so the render body can use `cx` mutably (listener
    // binding) while reading theme colours — `current` borrows `cx`.
    let dim = content.dim_amount;
    let t = theme::current(cx).dimmed(dim);

    let status_banner = status_banner(
        &content.status,
        pane_id,
        content.window_handle,
        content.cwd.is_some(),
        &t,
        cx,
    );

    // Activity bar: title left, fold buttons right. Title resolves to the
    // session title, else the first prompt, else the configured agent name.
    let title = activity_bar_title(content.session_title.as_deref(), &content.items);
    let agent_title = if content.agent_name.trim().is_empty() {
        content.agent_id.as_str()
    } else {
        content.agent_name.as_str()
    };
    let bar = activity_bar(
        ActivityBarProps {
            pane_id,
            agent_id: &content.agent_id,
            title: title.as_deref().or(Some(agent_title)),
            last_active: content.session_updated_at.as_deref(),
            usage: content.session_usage.as_ref(),
            has_items: !content.items.is_empty(),
            dim,
        },
        cx,
    );

    // Scroll-to-bottom button, shown when scrolled up off the bottom
    // (tail-follow released). Anchors to the body slot so it floats above the
    // working footer. At-bottom is read from the list geometry.
    let scroll_btn: Option<AnyElement> = (!content.items.is_empty()
        && !crate::ui::scrollbar::list_at_bottom(
            &content.list_state,
            theme::AGENT_CHAT_SCROLL_BOTTOM_SLACK,
        ))
    .then(|| scroll_to_bottom_button(pane_id, cx).into_any_element());

    let body: AnyElement = if content.items.is_empty() {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(theme::agent_chat_font_size(cx)))
            .text_color(theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim))
            .child(SharedString::from(s::agent_chat_empty()))
            .into_any_element()
    } else {
        // Virtualized conversation: `list` renders only visible rows, so draw
        // cost is bounded by the viewport, not the conversation length. The
        // closure indexes the projected `rows` (see `rows::project`) and
        // `render_row` dispatches by kind with per-row padding + nesting rail.
        let t_items = t.clone();
        // The last *visible* row gets bottom `PAD_Y` (vs `LIST_GAP` between
        // rows); collapsed rows are zero-height so they don't count.
        let last_visible = content.rows.iter().rposition(|r| !r.hidden).unwrap_or(0);
        let list_el = list(
            content.list_state.clone(),
            cx.processor(move |this, ix, _window, cx| match this.rows.get(ix) {
                Some(row) => render_row(this, ix, row, last_visible, &t_items, cx),
                None => gpui::Empty.into_any_element(),
            }),
        )
        .with_sizing_behavior(ListSizingBehavior::Auto)
        .size_full();
        // Wrap the list so the absolute-fill scrollbar overlay and the
        // scroll-to-bottom button can sit over it (parent must be `relative`,
        // sized to the viewport). Capture the list viewport bounds each paint
        // (sanctioned MVU layout-geometry cache) so the drag-selection
        // autoscroll poll can tell when the cursor has left the pane. Painted
        // behind the list and non-interactive, so it never intercepts mouse.
        let bounds_capture = {
            let view = cx.entity();
            canvas(
                move |bounds, _window, cx| {
                    view.update(cx, |v, _| v.list_bounds = Some(bounds));
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full()
        };
        div()
            .relative()
            .flex_1()
            .min_h(px(0.))
            // Left mouse-down starts the autoscroll poll. Bubbles after the
            // block's own selection start and never stops propagation, so it
            // doesn't disturb normal click/scroll handling.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _ev, window, cx| this.start_selection_autoscroll(window, cx)),
            )
            // Left mouse-up ends the drag on the always-painted container, so
            // the poll stops on release even if the selected child block is no
            // longer painted.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _ev, _window, _cx| this.end_selection_drag()),
            )
            // Mouse-move catches an off-window release on re-entry (the button is
            // no longer held); mirrors the terminal's implicit mouse-up.
            .on_mouse_move(cx.listener(|this, ev, _window, cx| this.on_selection_drag_move(ev, cx)))
            .child(bounds_capture)
            .child(list_el)
            .children(crate::ui::scrollbar::vertical_thumb_for_list(
                ("agent-chat-scrollbar", pane_id as usize),
                &content.list_state,
                px(0.),
                t.scrollbar_thumb,
                t.file_viewer_scrollbar_thumb_hover,
            ))
            .children(scroll_btn)
            // Right-click over selected text offers Copy. The selected text is
            // captured at menu-build time (the right-click) because clicking the
            // item is a left-click outside the text block, which clears the
            // selection before the item handler runs. No selection → empty menu,
            // which `ContextMenu` suppresses (nothing shows on an empty area).
            .context_menu(menu_builder(|menu, _window, cx| {
                let Some(text) = active_text_selection(cx)
                    .and_then(|sel| sel.selection_text(cx))
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                else {
                    return menu;
                };
                menu.item(
                    PopupMenuItem::new(SharedString::from(s::menu_copy())).on_click(
                        move |_, _window, app| {
                            app.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
                        },
                    ),
                )
            }))
            .into_any_element()
    };

    div()
        .size_full()
        .relative()
        .flex()
        .flex_col()
        // The view owns its focus handle and tracks it here (like
        // `TerminalView`), so the pane walker embeds it as a plain cached
        // `AnyView` and `wrapper_focus_handle` returns `None` for this kind.
        .track_focus(&content.focus_handle)
        // Background: the terminal color theme at the window opacity, so this
        // pane matches the terminal in color and translucency. `agent_chat_bg`
        // is opaque, so `.opacity(alpha)` yields exactly the window alpha.
        // Applied only to the pane fill — bubbles and header keep their own
        // opaque backgrounds for legibility.
        .bg(
            crate::ui::theme::dim_toward_gray(crate::ui::theme::agent_chat_bg(cx), dim)
                .opacity(crate::ui::theme::background_alpha(cx)),
        )
        .children(status_banner)
        .child(bar)
        .child(body)
        // The plan region is a flex-none sibling below the `flex_1` body, so it
        // claims its own space and the conversation shrinks to fit. `None` when
        // the agent has published no plan, costing no vertical space.
        .children(plan_region(
            pane_id,
            &content.plan,
            content.plan_collapsed,
            &content.plan_scroll,
            &t,
            dim,
            cx,
        ))
}

fn agent_display_name(view: &AgentChatView) -> &str {
    let name = view.agent_name.trim();
    if name.is_empty() {
        view.agent_id.as_str()
    } else {
        name
    }
}

/// Render one projected row: an item, a synthetic fold header, or a zero-height
/// `Empty` when collapsed under an ancestor fold (the row stays in the sequence
/// so the count is fold-stable). Applies per-row padding (top on the first row,
/// `PAD_Y` on the last visible, `LIST_GAP` between) and a left indent per level.
fn render_row(
    this: &AgentChatView,
    ix: usize,
    row: &RenderRow,
    last_visible: usize,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    if row.hidden {
        return gpui::Empty.into_any_element();
    }
    let inner: AnyElement = match &row.kind {
        RowKind::User(i) => match this.items.get(*i) {
            Some(ChatItem::UserText(text)) => {
                user_bubble(*i, text, this.dim_amount, cx).into_any_element()
            }
            _ => gpui::Empty.into_any_element(),
        },
        RowKind::ResponseHeader { anchor, collapsed } => {
            response_bar(this, *anchor, *collapsed, t, cx).into_any_element()
        }
        RowKind::AgentItem(i) => match this.items.get(*i) {
            Some(item) => render_item(
                *i,
                item,
                row.indent > 0,
                &this.items,
                &this.diff_editors,
                &this.diff_stats,
                &this.mermaid_images,
                &this.tool_images,
                &this.fold,
                t,
                this.dim_amount,
                agent_display_name(this),
                cx,
            ),
            None => gpui::Empty.into_any_element(),
        },
        RowKind::ToolGroupHeader {
            gid,
            first_ix,
            count,
            collapsed,
        } => tool_group_bar(this, gid, *first_ix, *count, *collapsed, t, cx).into_any_element(),
        RowKind::ConclusionItem(i) => match this.items.get(*i) {
            Some(item @ ChatItem::AssistantText { text, .. }) => {
                let key = FoldKey::Assistant(*i);
                let expanded = this.fold.is_expanded(&key, is_active(item));
                conclusion_block(
                    *i,
                    key,
                    expanded,
                    text,
                    &this.mermaid_images,
                    this.dim_amount,
                    cx,
                )
                .into_any_element()
            }
            _ => gpui::Empty.into_any_element(),
        },
        RowKind::WorkingIndicator => working_indicator(this, cx).into_any_element(),
    };
    let bottom = if ix == last_visible {
        theme::AGENT_CHAT_PAD_Y
    } else {
        theme::AGENT_CHAT_LIST_GAP
    };
    // A new turn (a `User` row past the first) gets extra top space so
    // consecutive turns read as distinct exchanges.
    let turn_break = ix != 0 && matches!(row.kind, RowKind::User(_));
    div()
        .w_full()
        .min_w_0()
        .px(px(theme::AGENT_CHAT_PAD_X))
        .when(ix == 0, |d| d.pt(px(theme::AGENT_CHAT_PAD_Y)))
        .when(turn_break, |d| d.mt(px(theme::AGENT_CHAT_TURN_GAP)))
        .pb(px(bottom))
        // Nest one content-pad unit per level (group members sit under the ⚙ bar).
        .when(row.indent > 0, |d| {
            d.pl(px(theme::AGENT_CHAT_PAD_X * (row.indent as f32 + 1.0)))
        })
        .child(inner)
        .into_any_element()
}

/// Where the fold-toggle click lives on a [`disclosure_row`] / [`foldable_block`]
/// header: the whole row, or the chevron glyph alone.
#[derive(Clone, Copy)]
pub(super) enum ToggleTarget {
    /// The whole header row toggles the fold — generous hit area (section bars,
    /// text blocks).
    Row,
    /// Only the chevron toggles; the rest of the header stays inert so a header
    /// carrying selectable content (the tool-card title) doesn't fight text
    /// selection.
    Chevron,
}

/// Shared scaffold for every collapsible header: a full-width borderless row
/// that toggles `key` and leads with the disclosure chevron. Callers append
/// their own label / summary / trailing glyph; `target` picks where the click
/// lives. One source for layout + click target, so the section bars and inline
/// blocks can't drift apart.
fn disclosure_row(
    base: impl Into<ElementId>,
    key: FoldKey,
    expanded: bool,
    target: ToggleTarget,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> gpui::Stateful<gpui::Div> {
    // One base id yields both the row's click target and the chevron glyph's
    // identity — distinct yet stable across renders.
    let base: ElementId = base.into();
    let chevron: Disclosure = disclosure((base.clone(), "chevron"), expanded)
        .color(theme::dim_toward_gray(theme::agent_chat_fg_subtle(cx), dim));
    let row = div()
        .id((base, "row"))
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::AGENT_CHAT_MSG_GAP));
    match target {
        ToggleTarget::Row => row
            .cursor_pointer()
            .on_click(cx.listener(move |this, _ev, _window, cx| this.toggle_fold(key.clone(), cx)))
            .child(chevron),
        // Bind the click to the chevron itself; the row carries no click
        // handler, so selectable header content stays freely selectable.
        ToggleTarget::Chevron => row.child(chevron.on_toggle(
            cx.listener(move |this, _ev, _window, cx| this.toggle_fold(key.clone(), cx)),
        )),
    }
}

/// The outcome a section header's rollup glyph summarizes over its run (see
/// [`rollup_glyph`]). Single source for the response bar and tool-group bar so
/// the two never disagree on treatment.
enum Rollup {
    /// At least one child still in progress / streaming (not settled).
    Running,
    /// All children succeeded.
    Ok,
    /// A mix — at least one failure alongside at least one success.
    Partial,
    /// Everything failed; nothing succeeded.
    Failed,
}

/// Classify a run from whether anything is active, anything failed, and anything
/// succeeded. `Running` wins (not settled yet); otherwise a failure mixed with a
/// success is `Partial`, all-failure is `Failed`, no failure is `Ok`.
fn rollup_of(running: bool, any_failed: bool, any_ok: bool) -> Rollup {
    if running {
        Rollup::Running
    } else if !any_failed {
        Rollup::Ok
    } else if any_ok {
        Rollup::Partial
    } else {
        Rollup::Failed
    }
}

/// The blink opacity for the shared 2-tick `StatusPulseClock` pulse:
/// `1.0` on even half-ticks (bright), `STATUS_INDICATOR_PULSE_OPACITY_MIN`
/// on odd half-ticks (dim). Used by [`rollup_glyph`] for the Running dot
/// and by the plan in-progress glyph so both pulse in lockstep.
pub(super) fn pulse_opacity(cx: &gpui::App) -> f32 {
    let tick = cx
        .try_global::<StatusPulseClock>()
        .map(|c| c.tick)
        .unwrap_or(0);
    if (tick / 2).is_multiple_of(2) {
        1.0
    } else {
        theme::STATUS_INDICATOR_PULSE_OPACITY_MIN
    }
}

fn rollup_glyph(
    rollup: Rollup,
    t: &theme::DarudaTheme,
    cx: &gpui::App,
) -> impl IntoElement + use<> {
    let (glyph, color) = match rollup {
        // Amber "executing tool" accent so an in-progress run reads stronger
        // than a settled glyph.
        Rollup::Running => ("●", t.status_executing_tool_dark),
        Rollup::Ok => ("✓", t.file_diff_stat_add),
        // Partial = some failed, some succeeded → warning, not a hard failure.
        Rollup::Partial => ("⚠", t.banner_warning_text),
        Rollup::Failed => ("✗", t.banner_error_text),
    };
    // Blink the running dot on the shared 2-tick pulse so it reads as live;
    // settled glyphs stay solid.
    let opacity = if matches!(rollup, Rollup::Running) {
        pulse_opacity(cx)
    } else {
        1.0
    };
    div()
        .flex_none()
        .opacity(opacity)
        .text_color(color)
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(SharedString::from(glyph))
}

/// Collapsible header for an agent response (agent items under a user message).
/// The whole row toggles `FoldKey::Response`; shows a chevron + agent label and,
/// when collapsed, the response's first line, tool count, and status-rollup
/// glyph (see [`rollup_glyph`]).
fn response_bar(
    this: &AgentChatView,
    anchor: usize,
    collapsed: bool,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let mut tools = 0usize;
    let mut first_line: Option<String> = None;
    let (mut running, mut failed, mut any_ok) = (false, false, false);
    for item in this.items.iter().skip(anchor + 1) {
        match item {
            ChatItem::UserText(_) => break,
            ChatItem::ToolCall(tc) => {
                tools += 1;
                match tc.status {
                    ToolStatusView::InProgress | ToolStatusView::Pending => running = true,
                    ToolStatusView::Failed => failed = true,
                    ToolStatusView::Completed => any_ok = true,
                    // Settled, neither success nor failure — sets no flag.
                    ToolStatusView::Cancelled => {}
                }
            }
            ChatItem::AssistantText {
                text, streaming, ..
            }
            | ChatItem::Thinking {
                text, streaming, ..
            } => {
                if first_line.is_none() {
                    first_line = text
                        .lines()
                        .find(|l| !l.trim().is_empty())
                        .map(str::to_owned);
                }
                // Produced assistant output counts as success, so a response
                // that answered *and* hit a tool failure reads partial (⚠),
                // not a hard failure (✗).
                if !text.trim().is_empty() {
                    any_ok = true;
                }
                running |= *streaming;
            }
            ChatItem::Error(_) => failed = true,
            ChatItem::Permission(_) => {}
        }
    }
    let rollup = rollup_of(running, failed, any_ok);
    // Borderless `disclosure_row`, matching the block headers — section headers
    // stay light; only content cards (`tool_card`) carry box chrome.
    let mut row = disclosure_row(
        SharedString::from(format!("agent-chat-response-{anchor}")),
        FoldKey::Response(anchor),
        !collapsed,
        ToggleTarget::Row,
        this.dim_amount,
        cx,
    )
    .child(
        div()
            .flex_none()
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(this.dim(theme::agent_chat_fg(cx)))
            .text_size(px(theme::agent_chat_font_size(cx)))
            .child(SharedString::from(agent_display_name(this).to_string())),
    );

    // Collapsed: first line (ellipsized) + tool count fill the row before the
    // status glyph. Expanded: push the glyph to the right.
    if collapsed {
        let line = first_line.unwrap_or_default();
        row = row.child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(this.dim(theme::agent_chat_fg_subtle(cx)))
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(SharedString::from(line)),
        );
        if tools > 0 {
            row = row.child(
                div()
                    .flex_none()
                    .text_color(this.dim(theme::agent_chat_fg_subtle(cx)))
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .child(SharedString::from(s::agent_chat_tool_group_count(tools))),
            );
        }
    } else {
        row = row.child(div().flex_1());
    }

    row.child(rollup_glyph(rollup, t, cx))
}

/// Collapsible header for a consecutive tool-call group. The whole row toggles
/// the group's fold (`FoldKey::ToolGroup`); shows a chevron, a ⚙ marker, the
/// "N tool calls" count, and a status-rollup glyph (see [`rollup_glyph`]).
fn tool_group_bar(
    this: &AgentChatView,
    gid: &str,
    first_ix: usize,
    count: usize,
    collapsed: bool,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let (mut running, mut failed, mut any_ok) = (false, false, false);
    for k in first_ix..(first_ix + count).min(this.items.len()) {
        if let ChatItem::ToolCall(tc) = &this.items[k] {
            match tc.status {
                ToolStatusView::InProgress | ToolStatusView::Pending => running = true,
                ToolStatusView::Failed => failed = true,
                ToolStatusView::Completed => any_ok = true,
                // Settled but neither success nor failure: sets no flag, so the
                // run stops pulsing without turning the glyph red.
                ToolStatusView::Cancelled => {}
            }
        }
    }
    let rollup = rollup_of(running, failed, any_ok);
    // Borderless `disclosure_row`, same as the response bar.
    disclosure_row(
        SharedString::from(format!("agent-chat-toolgroup-{gid}")),
        FoldKey::ToolGroup(gid.to_string()),
        !collapsed,
        ToggleTarget::Row,
        this.dim_amount,
        cx,
    )
    .child(
        div()
            .flex_none()
            .text_color(this.dim(theme::agent_chat_fg_subtle(cx)))
            .text_size(px(theme::agent_chat_font_size(cx)))
            .child(SharedString::from("⚙")),
    )
    .child(
        div()
            .flex_1()
            .min_w_0()
            .text_color(this.dim(theme::agent_chat_fg_muted(cx)))
            .text_size(px(theme::agent_chat_font_size(cx)))
            .child(SharedString::from(s::agent_chat_tool_group_count(count))),
    )
    .child(rollup_glyph(rollup, t, cx))
}

/// Floating "jump to bottom" affordance shown when the user has scrolled up
/// (tail-follow released). Positioned bottom-right via the parent's `relative`.
fn scroll_to_bottom_button(
    pane_id: PaneId,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    div()
        .absolute()
        .bottom(px(theme::AGENT_CHAT_SCROLL_BTN_INSET))
        .right(px(theme::AGENT_CHAT_SCROLL_BTN_INSET))
        .child(
            button_bare(("agent-chat-scroll-bottom", pane_id as usize))
                .icon(IconName::ArrowDown)
                .on_click(cx.listener(move |this, _ev, _window, cx| this.scroll_to_bottom(cx))),
        )
}

/// One conversation row. Message bodies render as selectable markdown via
/// `crate::ui::markdown`, keyed by `ix` for stable selection identity. `fold` /
/// `diff_stats` are read-only here: foldable kinds derive expanded state via
/// `fold.is_expanded(&key, active)`; toggling routes through
/// `AgentChatView::toggle_fold`, never mutating the view in render.
#[allow(clippy::too_many_arguments)]
fn render_item(
    ix: usize,
    item: &ChatItem,
    under_response: bool,
    items: &[ChatItem],
    diff_editors: &DiffEditors,
    diff_stats: &DiffStats,
    mermaid_images: &MermaidImages,
    tool_images: &ToolImages,
    fold: &FoldState,
    t: &theme::DarudaTheme,
    dim: f32,
    agent_label: &str,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    match item {
        ChatItem::UserText(text) => user_bubble(ix, text, dim, cx).into_any_element(),
        // Under a response bar the speaker is already labeled with the agent
        // name; render the prose inline with no redundant per-block header/fold.
        // A trivial / top-level reply keeps the labeled, foldable block.
        ChatItem::AssistantText { text, .. } if under_response => {
            assistant_markdown(ix, text, mermaid_images, dim, cx)
        }
        ChatItem::AssistantText { text, .. } => {
            let key = FoldKey::Assistant(ix);
            let expanded = fold.is_expanded(&key, is_active(item));
            assistant_block(
                ix,
                key,
                expanded,
                text,
                mermaid_images,
                agent_label,
                dim,
                cx,
            )
            .into_any_element()
        }
        ChatItem::Thinking { text, .. } => {
            let key = FoldKey::Thinking(ix);
            let expanded = fold.is_expanded(&key, is_active(item));
            thinking_block(ix, key, expanded, text, mermaid_images, dim, cx).into_any_element()
        }
        ChatItem::ToolCall(tc) => {
            let key = tool_fold_key(tc);
            let expanded = fold.is_expanded(&key, is_active(item));
            tool_card(
                key,
                expanded,
                tc,
                items,
                diff_editors,
                diff_stats,
                tool_images,
                fold,
                t,
                dim,
                0,
                cx,
            )
            .into_any_element()
        }
        ChatItem::Permission(card) => permission_card(ix, card, t, dim, cx).into_any_element(),
        ChatItem::Error(message) => error_block(message, t, cx).into_any_element(),
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
pub(super) fn foldable_block<
    Id: Into<ElementId>,
    F: FnOnce(gpui::Stateful<gpui::Div>) -> gpui::Stateful<gpui::Div>,
>(
    id: Id,
    key: FoldKey,
    expanded: bool,
    target: ToggleTarget,
    header: AnyElement,
    summary: Option<AnyElement>,
    body: AnyElement,
    header_chrome: F,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<Id, F> {
    // Same `disclosure_row` scaffold as the section bars (chevron + click), then
    // append this block's own header content.
    let mut header_row = disclosure_row(id, key, expanded, target, dim, cx).child(header);
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
/// the first non-empty line of `text` with inline markdown flattened to plain
/// text (via [`summary_preview_line`], so `**bold**` reads as prose, not raw
/// `**`), dimmed (`theme::agent_chat_fg_subtle(cx)`) and single-line ellipsized
/// via `flex_1().min_w_0()` + `overflow_hidden()` — the same truncation idiom
/// the path / title elements use, so layout (not a hardcoded char limit) does
/// the ellipsizing. A left margin (`AGENT_CHAT_SUMMARY_GAP`) separates it from
/// the header label. `italic` matches the thinking block's treatment. `None`
/// when the text has no non-empty line (nothing to summarize).
pub(super) fn collapsed_text_summary(
    text: &str,
    italic: bool,
    dim: f32,
    cx: &App,
) -> Option<AnyElement> {
    let line = summary_preview_line(text)?;
    Some(
        div()
            .flex_1()
            .min_w_0()
            .ml(px(theme::AGENT_CHAT_SUMMARY_GAP))
            .overflow_hidden()
            .whitespace_nowrap()
            .when(italic, |el| el.italic())
            .text_color(theme::dim_toward_gray(theme::agent_chat_fg_subtle(cx), dim))
            .text_size(px(theme::agent_chat_font_size(cx)))
            .child(SharedString::from(line))
            .into_any_element(),
    )
}
