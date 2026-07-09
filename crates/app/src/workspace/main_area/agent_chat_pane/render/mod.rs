//! Pure view of an `&AgentChatView` — the scrolling conversation and inline
//! permission cards. Built under the view's own `Context<AgentChatView>` (the
//! view embeds this via `AnyView::cached(..)`). The prompt input lives in the
//! shared bottom dock (`send_terminal_input` routes to the focused AgentChat
//! pane's session), so this view carries no input field of its own.
//!
//! MVU view purity: every event closure is a one-line dispatch into an
//! `AgentChatView` op (`this.respond_permission`, `this.toggle_fold`,
//! `this.on_scroll`, …) — each notifies the view itself, so a scroll / fold
//! dirties only this cached subtree. No state transition lives here.
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

mod blocks;
mod chrome;
mod diff;
mod plan;
mod tool;

use daruda_acp::{ChatItem, ToolStatusView};
use gpui::{
    AnyElement, App, ElementId, Entity, IntoElement, ListSizingBehavior, SharedString, div, list,
    prelude::*, px,
};

/// Read-only diff editor entities keyed by `"{tool_call_id}#{diff_index}"`
/// (built in the ops layer; this view only embeds them).
pub(super) type DiffEditors = std::collections::HashMap<String, Entity<crate::ui::InputState>>;

/// Per-diff `+N −M` line counts keyed by `"{tool_call_id}#{diff_index}"`
/// (built in the ops layer; this view only reads them for the collapsed
/// diff summary).
pub(super) type DiffStats = std::collections::HashMap<String, DiffStat>;

/// Rendered mermaid diagrams (GPU-ready [`CachedImage`]) keyed by source hash
/// (filled async in the ops layer). Shared `Arc<Mutex<…>>` so the
/// `code_block_render` closure — bound into `TextView`'s cached parse — reads
/// the *live* cache, not a snapshot (the image lands after parse; see
/// `AgentChatContent::mermaid_images`).
pub(super) type MermaidImages = std::sync::Arc<
    std::sync::Mutex<
        std::collections::HashMap<
            u64,
            crate::workspace::main_area::file_view_pane::render::CachedImage,
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
use crate::ui::{Disclosure, IconName, StatusPulseClock, button_bare, disclosure};
use crate::workspace::main_area::agent_chat_pane::agent_chat_helpers::{
    DiffStat, activity_bar_title, is_active,
};
use crate::workspace::main_area::agent_chat_pane::fold::{FoldKey, FoldState};
use crate::workspace::main_area::agent_chat_pane::rows::{RenderRow, RowKind};
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;
use crate::workspace::main_area::pane_tree::PaneId;

/// Build the element tree for an Agent chat pane. Takes the view by shared
/// reference (field reads) plus its own `Context` (listener binding); the two
/// are distinct borrows, so reading `view` while `cx` is mutably held is fine.
pub(in crate::workspace) fn render(
    view: &AgentChatView,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement {
    let pane_id = view.pane_id;
    let content = view;
    // Clone the palette to an owned value so the render body can use `cx`
    // mutably (binding listeners) while reading theme colours — `current`
    // returns a borrow tied to `cx`.
    let dim = content.dim_amount;
    let t = theme::current(cx).dimmed(dim);

    let status_banner = status_banner(&content.status, &t, cx);

    // Activity bar: session title on the left, fold buttons on the right.
    // Always visible — it holds the fold buttons even while the conversation is
    // empty or still connecting. The title resolves to the agent's session
    // title, else the first prompt, else the configured agent name. Fold buttons
    // appear only once there are items.
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

    // The scroll-to-bottom button overlays the list when the user has scrolled
    // up off the bottom (tail-follow released). It anchors to the body slot
    // (below), not the pane root, so it floats just above the working footer
    // instead of colliding with it. At-bottom is read from the list geometry.
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
        // Virtualized conversation: `list` renders only the visible rows (+
        // overdraw), so draw cost is bounded by the viewport, not the
        // conversation length. The `'static` closure re-fetches the view
        // (`this`) and indexes the projected `rows` (see `rows::project`), which
        // interleaves synthetic fold headers with item rows; `render_row`
        // dispatches by kind and applies per-row padding + nesting rail.
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
        // Wrap the list so the live-tracking scrollbar overlay can sit over it:
        // the overlay is absolute-fill, so its parent must be `relative` and
        // sized to the viewport (this `flex_1` body slot). The scroll-to-bottom
        // button also anchors here so it stays above the working footer.
        div()
            .relative()
            .flex_1()
            .min_h(px(0.))
            .child(list_el)
            .children(crate::ui::scrollbar::vertical_thumb_for_list(
                ("agent-chat-scrollbar", pane_id as usize),
                &content.list_state,
                px(0.),
                t.scrollbar_thumb,
                t.file_viewer_scrollbar_thumb_hover,
            ))
            .children(scroll_btn)
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
        // Background: the terminal color theme (config
        // `effective_colors().background`) at the window opacity (config
        // `window.opacity`), so this pane matches the terminal in both color
        // and translucency. `agent_chat_bg` is opaque (a=1.0), so
        // `.opacity(alpha)` yields exactly the window alpha; at the default
        // 1.0 this is a plain fill. Applied only to the pane fill — message
        // bubbles and the header keep their own opaque backgrounds for
        // legibility.
        .bg(
            crate::ui::theme::dim_toward_gray(crate::ui::theme::agent_chat_bg(cx), dim)
                .opacity(crate::ui::theme::background_alpha(cx)),
        )
        .children(status_banner)
        .child(bar)
        .child(body)
        // The plan region is a flex-none sibling BELOW the body: because `body`
        // is `flex_1` and this is `flex_none`, the region claims its own space
        // and the conversation shrinks to fit. Absent (`None`) when the agent
        // has published no plan, so it costs no vertical space then.
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

/// Render one projected row: an item, a synthetic fold header, or — when
/// collapsed under an ancestor fold — a zero-height `Empty` (the row stays in
/// the sequence so the count is fold-stable). Applies per-row padding (top on
/// the first row, `PAD_Y` on the last visible row, `LIST_GAP` between) and a
/// left indent per nesting level.
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
                user_bubble(*i, text, &this.mermaid_images, this.dim_amount, cx).into_any_element()
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
    // consecutive turns read as distinct exchanges, not one running column.
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
/// header. `Row` makes the whole header a generous click target; `Chevron` binds
/// the toggle to the chevron glyph alone and leaves the rest of the header inert.
#[derive(Clone, Copy)]
pub(super) enum ToggleTarget {
    /// The whole header row toggles the fold (default: section bars, text
    /// blocks). Generous hit area.
    Row,
    /// Only the chevron toggles; the rest of the header is inert — so a header
    /// carrying its own selectable content (the tool-card title) doesn't fight
    /// text selection. Used by the tool card and its nested raw-input / diff
    /// disclosures.
    Chevron,
}

/// The shared scaffold for every collapsible header in the pane: a full-width,
/// borderless row that toggles `key` and leads with the disclosure chevron.
/// Callers append their own label / summary / trailing glyph. `target` picks
/// where the click lives (whole row vs chevron only). One source for the row's
/// layout + click target, so the section bars (`response_bar` /
/// `tool_group_bar`) and the inline blocks (`foldable_block`) can never drift
/// apart — e.g. one growing box chrome the others lack.
fn disclosure_row(
    base: impl Into<ElementId>,
    key: FoldKey,
    expanded: bool,
    target: ToggleTarget,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> gpui::Stateful<gpui::Div> {
    // One base id yields both the row's click target and the chevron glyph's
    // identity, so the two stay distinct yet stable across renders.
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

/// The status-rollup glyph trailing a section header: ● running (in progress /
/// streaming, blinking), ⚠ partial (some failed, some succeeded), ✗ all failed,
/// else ✓ all done. Single source for the response bar and the tool-group bar
/// so the two never disagree on treatment.
/// The outcome a section header's rollup glyph summarizes over its run.
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
        // than a settled glyph instead of fading into body grey.
        Rollup::Running => ("●", t.status_executing_tool_dark),
        Rollup::Ok => ("✓", t.file_diff_stat_add),
        // Partial = some failed, some succeeded → warning, not a hard failure.
        Rollup::Partial => ("⚠", t.banner_warning_text),
        Rollup::Failed => ("✗", t.banner_error_text),
    };
    // Blink the running dot (1.0 ↔ MIN on the shared 2-tick pulse) so an
    // in-progress run reads as live; the settled glyphs stay solid.
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

/// Collapsible header for an agent response (the run of agent items under a
/// user message). The whole row toggles `FoldKey::Response`; shows a chevron +
/// agent label, and — when collapsed — the response's first line, a tool count,
/// and a status-rollup glyph (see [`rollup_glyph`]). The
/// user prompt stays visible above, so the summary doesn't repeat it.
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
                // Produced assistant output counts as a success for the rollup,
                // so a response that answered *and* hit a tool failure reads as
                // partial (⚠), not a hard failure (✗).
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
    // Borderless disclosure row (shared `disclosure_row`) — matches the
    // assistant / thinking block headers, so a single-block reply and a
    // multi-block / tool response share one header style. Only content cards
    // (`tool_card`) carry box chrome; section headers stay light.
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

    // Collapsed: the response's first line (ellipsized) + tool count fill the
    // row before the status glyph. Expanded: just push the glyph to the right.
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
                // Cancelled is settled but neither a success nor a failure, so
                // it sets no flag — it stops the run pulsing without turning
                // the rollup glyph red.
                ToolStatusView::Cancelled => {}
            }
        }
    }
    let rollup = rollup_of(running, failed, any_ok);
    // Borderless disclosure row (shared `disclosure_row`), same as the response
    // bar — section headers stay light; only `tool_card` (content) carries box
    // chrome.
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

/// Floating "jump to bottom" affordance shown over the list when the user has
/// scrolled up (tail-follow released). One-line dispatch into
/// `this.scroll_to_bottom(cx)` (render purity); positioned bottom-right via
/// the pane root's `relative`.
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
/// `crate::ui::markdown`, keyed by `ix` for stable selection identity.
///
/// `fold` / `diff_stats` are read-only here (render purity): the foldable kinds
/// derive their expanded state purely via `fold.is_expanded(&key, active)` and
/// read the collapsed diff summary from `diff_stats`. Toggling routes through
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
    fold: &FoldState,
    t: &theme::DarudaTheme,
    dim: f32,
    agent_label: &str,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    match item {
        ChatItem::UserText(text) => {
            user_bubble(ix, text, mermaid_images, dim, cx).into_any_element()
        }
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
            let key = FoldKey::Tool(tc.id.clone());
            let expanded = fold.is_expanded(&key, is_active(item));
            tool_card(
                key,
                expanded,
                tc,
                items,
                diff_editors,
                diff_stats,
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
/// the first non-empty line of `text`, dimmed (`theme::agent_chat_fg_subtle(cx)`) and
/// single-line ellipsized via `flex_1().min_w_0()` + `overflow_hidden()` — the
/// same truncation idiom the path / title elements use, so layout (not a
/// hardcoded char limit) does the ellipsizing. `italic` matches the thinking
/// block's treatment. `None` when the text has no non-empty line (nothing to
/// summarize).
pub(super) fn collapsed_text_summary(
    text: &str,
    italic: bool,
    dim: f32,
    cx: &App,
) -> Option<AnyElement> {
    let line = text.lines().find(|l| !l.trim().is_empty())?;
    Some(
        div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .when(italic, |el| el.italic())
            .text_color(theme::dim_toward_gray(theme::agent_chat_fg_subtle(cx), dim))
            .text_size(px(theme::agent_chat_font_size(cx)))
            .child(SharedString::from(line.to_string()))
            .into_any_element(),
    )
}
