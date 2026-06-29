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

use daruda_acp::{
    ChatItem, DiffView, PermissionChoice, PermissionItem, PermissionKindView, PermissionResolution,
    PlanEntryView, PlanStatus, ToolCallItem, ToolStatusView,
};
use gpui::{
    AnyElement, ElementId, Entity, Hsla, IntoElement, ListSizingBehavior, SharedString, div, list,
    prelude::*, px, relative,
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
use crate::ui::{
    ButtonVariants as _, Disclosure, IconName, Sizable as _, StatusPulseClock, button_bare,
    disclosure,
};
use crate::workspace::main_area::agent_chat_pane::agent_chat_ops::{
    DiffStat, diff_editor_key, is_active, mermaid_key,
};
use crate::workspace::main_area::agent_chat_pane::fold::{FoldKey, FoldState};
use crate::workspace::main_area::agent_chat_pane::rows::{RenderRow, RowKind};
use crate::workspace::main_area::agent_chat_pane::view::{AgentChatView, AgentSessionStatus};
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
    let t = theme::current(cx).clone();

    let status_banner = status_banner(&content.status, &t);

    // Activity bar: session title on the left, fold buttons on the right.
    // Always visible — it holds the title even while the conversation is empty
    // or still connecting. Fold buttons appear only once there are items.
    let bar = activity_bar(
        pane_id,
        content.session_title.as_deref(),
        !content.items.is_empty(),
        &t,
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
            .text_size(px(theme::AGENT_CHAT_MSG_FONT_SIZE))
            .text_color(t.text_muted)
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
        .bg(t.file_viewer_bg)
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
            &t,
            cx,
        ))
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
                user_bubble(*i, text, &this.mermaid_images, t).into_any_element()
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
                &this.diff_editors,
                &this.diff_stats,
                &this.mermaid_images,
                &this.fold,
                t,
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
                conclusion_block(*i, key, expanded, text, &this.mermaid_images, t, cx)
                    .into_any_element()
            }
            _ => gpui::Empty.into_any_element(),
        },
        RowKind::WorkingIndicator => working_indicator(this, t, cx).into_any_element(),
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

/// The shared scaffold for every collapsible header in the pane: a full-width,
/// borderless clickable row that toggles `key` and leads with the disclosure
/// chevron. Callers append their own label / summary / trailing glyph. One
/// source for the row's layout + click target, so the section bars
/// (`response_bar` / `tool_group_bar`) and the inline blocks (`foldable_block`)
/// can never drift apart — e.g. one growing box chrome the others lack.
fn disclosure_row(
    base: impl Into<ElementId>,
    key: FoldKey,
    expanded: bool,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> gpui::Stateful<gpui::Div> {
    // One base id yields both the row's click target and the chevron glyph's
    // identity, so the two stay distinct yet stable across renders.
    let base: ElementId = base.into();
    let chevron: Disclosure = disclosure((base.clone(), "chevron"), expanded).color(t.text_subtle);
    div()
        .id((base, "row"))
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _ev, _window, cx| this.toggle_fold(key.clone(), cx)))
        .child(chevron)
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
fn pulse_opacity(cx: &gpui::App) -> f32 {
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
        .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
        .child(SharedString::from(glyph))
}

/// Collapsible header for an agent response (the run of agent items under a
/// user message). The whole row toggles `FoldKey::Response`; shows a chevron +
/// "Agent" label, and — when collapsed — the response's first line, a tool
/// count, and a status-rollup glyph (see [`rollup_glyph`]). The
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
        t,
        cx,
    )
    .child(
        div()
            .flex_none()
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(t.text_body)
            .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
            .child(SharedString::from(s::agent_chat_label_agent())),
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
                .text_color(t.text_subtle)
                .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                .child(SharedString::from(line)),
        );
        if tools > 0 {
            row = row.child(
                div()
                    .flex_none()
                    .text_color(t.text_subtle)
                    .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
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
        t,
        cx,
    )
    .child(
        div()
            .flex_none()
            .text_color(t.text_subtle)
            .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
            .child(SharedString::from("⚙")),
    )
    .child(
        div()
            .flex_1()
            .min_w_0()
            .text_color(t.text_muted)
            .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
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

/// Pane activity bar: session title on the LEFT, "Expand all" / "Collapse all"
/// ghost buttons on the RIGHT. Always rendered — it holds the title even while
/// the conversation is empty or still connecting. The fold buttons appear only
/// when `has_items` is true (render purity: no logic here, just `.when()`).
/// A bottom hairline separates the bar from the conversation body.
fn activity_bar(
    pane_id: PaneId,
    session_title: Option<&str>,
    has_items: bool,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let title: SharedString = session_title
        .map(|s| SharedString::from(s.to_string()))
        .unwrap_or_else(|| SharedString::from(s::agent_chat_activity_bar_title()));

    let expand = crate::ui::button(
        ("agent-chat-expand-all", pane_id as usize),
        SharedString::from(s::agent_chat_expand_all()),
    )
    .ghost()
    .xsmall()
    .on_click(cx.listener(move |this, _ev, _window, cx| this.set_all_folds(true, cx)));
    let collapse = crate::ui::button(
        ("agent-chat-collapse-all", pane_id as usize),
        SharedString::from(s::agent_chat_collapse_all()),
    )
    .ghost()
    .xsmall()
    .on_click(cx.listener(move |this, _ev, _window, cx| this.set_all_folds(false, cx)));

    div()
        .flex_none()
        .w_full()
        .flex()
        .flex_row()
        .justify_between()
        .items_center()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .px(px(theme::AGENT_CHAT_PAD_X))
        .py(px(theme::AGENT_CHAT_PAD_Y))
        .border_b_1()
        .border_color(t.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                .text_color(t.text_primary)
                .child(title),
        )
        .when(has_items, |row| {
            row.child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::AGENT_CHAT_MSG_GAP))
                    .text_color(t.text_muted)
                    .child(expand)
                    .child(collapse),
            )
        })
}

/// `(completed, total)` over the plan entries.  Returns `(0, 0)` for an empty
/// plan — callers must guard on `plan.is_empty()` before using the ratio for
/// colour logic (0 == 0 would otherwise read as "all done").
fn plan_progress(plan: &[PlanEntryView]) -> (usize, usize) {
    let done = plan
        .iter()
        .filter(|e| e.status == PlanStatus::Completed)
        .count();
    (done, plan.len())
}

/// The status glyph + its colour for one plan entry: ● done (green),
/// ● in progress (running amber), ● pending (muted). All three states share
/// the filled ● glyph and are distinguished by colour alone. Mirrors
/// [`rollup_glyph`]'s `(glyph, color)` shape; the completed green reuses
/// the diff-stat add colour (there is no dedicated `success` theme token).
fn plan_status_glyph(status: PlanStatus, t: &theme::DarudaTheme) -> (&'static str, Hsla) {
    match status {
        // file_diff_stat_add == SUCCESS (green); no dedicated plan-complete token.
        PlanStatus::Completed => ("●", t.file_diff_stat_add),
        PlanStatus::InProgress => ("●", t.status_executing_tool_dark),
        PlanStatus::Pending => ("●", t.text_muted),
    }
}

/// The content text colour for one plan entry: completed dims (muted, no
/// strikethrough), in-progress emphasizes (primary), pending stays body.
fn plan_entry_color(status: PlanStatus, t: &theme::DarudaTheme) -> Hsla {
    match status {
        PlanStatus::Completed => t.text_muted,
        PlanStatus::InProgress => t.text_primary,
        PlanStatus::Pending => t.text_body,
    }
}

/// Dedicated bottom region rendering the agent's live execution plan as a
/// collapsible checklist. `None` (no element, no space) when the plan is empty.
/// A top hairline separates it from the conversation above (the region sits
/// below the body). The header — a disclosure row mirroring the section bars —
/// shows a chevron, the "Plan" label, and the `{done}/{total}` count; collapsed
/// it also trails the current in-progress entry's content. Clicking the header
/// toggles [`AgentChatView::toggle_plan_collapsed`]. Expanded, the checklist
/// lists each entry (status glyph + content) and scrolls internally past
/// `AGENT_CHAT_PLAN_MAX_H`.
fn plan_region(
    pane_id: PaneId,
    plan: &[PlanEntryView],
    collapsed: bool,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> Option<AnyElement> {
    if plan.is_empty() {
        return None;
    }
    let (done, total) = plan_progress(plan);
    // All-done reads green (the run finished); otherwise the count stays muted.
    let count_color = if done == total {
        t.file_diff_stat_add
    } else {
        t.text_muted
    };

    // Header: same chevron + clickable-row scaffold as the section bars, but the
    // click routes to the plan toggle (not a `FoldKey`), so it's built inline
    // rather than via `disclosure_row`. The chevron carries no `on_toggle` — it's
    // a pure indicator; the whole row is the click target.
    let chevron: Disclosure = disclosure(
        SharedString::from(format!("agent-chat-plan-chevron-{pane_id}")),
        !collapsed,
    )
    .color(t.text_subtle);
    let mut header = div()
        .id(("agent-chat-plan-header", pane_id as usize))
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .px(px(theme::AGENT_CHAT_PAD_X))
        .py(px(theme::AGENT_CHAT_PAD_Y))
        .cursor_pointer()
        .on_click(cx.listener(|this, _ev, _window, cx| this.toggle_plan_collapsed(cx)))
        .child(chevron)
        .child(
            div()
                .flex_none()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(t.text_body)
                .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                .child(SharedString::from(s::agent_chat_plan_label())),
        )
        .child(
            div()
                .flex_none()
                .text_color(count_color)
                .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                .child(SharedString::from(format!("{done}/{total}"))),
        );

    // Collapsed: trail the current in-progress entry's content (muted,
    // ellipsized) after a `·` separator, so a folded plan still hints at the
    // live step. A pulsing ● glyph precedes the separator so a folded plan
    // still shows the live pulse. Nothing trails when no entry is in progress.
    if collapsed && let Some(active) = plan.iter().find(|e| e.status == PlanStatus::InProgress) {
        header = header
            .child(
                div()
                    .flex_none()
                    .opacity(pulse_opacity(cx))
                    .text_color(t.status_executing_tool_dark)
                    .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                    .child(SharedString::from("●")),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(t.text_subtle)
                    .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                    .child(SharedString::from(format!("· {}", active.content))),
            );
    }

    let mut region = div()
        .flex_none()
        .w_full()
        .flex()
        .flex_col()
        .border_t_1()
        .border_color(t.border)
        // surface-1 panel background so the plan region reads as a distinct
        // panel stepped above the conversation (DESIGN.md: depth via the
        // surface ladder; `dock_bg` == BG_PANEL == SURFACE_1 == #0f1011).
        .bg(t.dock_bg)
        .child(header);

    if !collapsed {
        let mut list = div()
            .id(("agent-chat-plan-list", pane_id as usize))
            .flex()
            .flex_col()
            .gap(px(theme::AGENT_CHAT_MSG_GAP))
            .max_h(px(theme::AGENT_CHAT_PLAN_MAX_H))
            // WORKAROUND: plan list uses a plain Div (not `list()`) — no virtualisation.
            // Root cause: `list()` requires an `Entity<ListState>` and row-count wiring
            // that adds complexity for a typically small dataset (< 50 entries).
            // Acceptable while agent plans stay in that range; if 100+ entries become
            // common, migrate to `list()` to avoid full-tree repaint cost.
            // The Div also lacks the project's 4px daruda thumb (`vertical_thumb_for_list`
            // doesn't apply); deferred — small surface (max 168px).
            .overflow_y_scroll()
            .px(px(theme::AGENT_CHAT_PAD_X))
            .pb(px(theme::AGENT_CHAT_PAD_Y));
        for entry in plan {
            let (glyph, glyph_color) = plan_status_glyph(entry.status, t);
            let in_progress = entry.status == PlanStatus::InProgress;
            list = list.child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_baseline()
                    .gap(px(theme::AGENT_CHAT_MSG_GAP))
                    // In-progress row: accent-28% tint + slight rounding so the
                    // highlight reads as a bar (mirrors the selection token used
                    // by the completion menu and text selection).
                    .px(px(theme::GAP_SM))
                    .when(in_progress, |row| row.bg(theme::SELECTION_BG).rounded_sm())
                    .child(
                        div()
                            .flex_none()
                            .text_color(glyph_color)
                            .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                            // In-progress glyph pulses in lockstep with the section
                            // header rollup dot; settled glyphs stay solid.
                            .when(in_progress, |g| g.opacity(pulse_opacity(cx)))
                            .child(SharedString::from(glyph)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(plan_entry_color(entry.status, t))
                            .text_size(px(theme::AGENT_CHAT_MSG_FONT_SIZE))
                            .child(SharedString::from(entry.content.clone())),
                    ),
            );
        }
        region = region.child(list);
    }

    Some(region.into_any_element())
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

/// Title of the tool call currently in progress, if any — drives the
/// ExecutingTool footer label. The agent runs calls sequentially, so the last
/// `InProgress` call is the live one.
fn running_tool_title(items: &[ChatItem]) -> Option<String> {
    items.iter().rev().find_map(|item| match item {
        ChatItem::ToolCall(tc) if matches!(tc.status, ToolStatusView::InProgress) => {
            Some(tc.title.clone())
        }
        _ => None,
    })
}

/// Animated trailing dots (".", "..", "...") for any "in progress" label. Cycles
/// off the shared, CPU-gated `StatusPulseClock` — the pulse pump dirties the
/// in-flight agent chat view each tick (`Workspace::notify_in_flight_agent_chats`),
/// so callers advance without a per-frame animation. Shared by the working
/// footer / indicator and the running tool-call badge.
fn pulse_dots(cx: &gpui::App) -> String {
    let tick = cx
        .try_global::<StatusPulseClock>()
        .map(|c| c.tick)
        .unwrap_or(0);
    ".".repeat((tick % 3) as usize + 1)
}

/// The live activity label this turn: blocked on a permission prompt, running a
/// named tool, or otherwise generating prose. The animated trailing dots are
/// appended by [`working_indicator`].
fn working_status(content: &AgentChatView) -> SharedString {
    if content.pending_permission.is_some() {
        s::agent_chat_awaiting_permission().into()
    } else if let Some(title) = running_tool_title(&content.items) {
        s::agent_chat_working_tool(&title).into()
    } else {
        s::agent_chat_working().into()
    }
}

/// Inline "agent is working" indicator, projected as the tail row of the last
/// turn while a turn is in flight but nothing is streaming yet (the gap after a
/// tool group settles, before the next assistant text). It lives *in* the
/// conversation flow, so the progress signal sits where the next response will
/// appear. The label gets animated trailing dots (".", "..", "...") off the
/// shared `StatusPulseClock` — the pulse pump dirties this view while the turn
/// is in flight (`Workspace::notify_in_flight_agent_chats`), so they advance
/// without a per-frame animation. Cancelling is the bottom-dock Stop button.
fn working_indicator(
    content: &AgentChatView,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let base = working_status(content);
    let dots = pulse_dots(cx);
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(t.text_subtle)
                .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                .child(SharedString::from(format!("{base}{dots}"))),
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
    diff_editors: &DiffEditors,
    diff_stats: &DiffStats,
    mermaid_images: &MermaidImages,
    fold: &FoldState,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    match item {
        ChatItem::UserText(text) => user_bubble(ix, text, mermaid_images, t).into_any_element(),
        // Under a response bar the speaker is already labeled "Agent"; render the
        // prose inline with no redundant per-block header/fold. A trivial / top-
        // level reply (no response bar) keeps the labeled, foldable block.
        ChatItem::AssistantText { text, .. } if under_response => {
            assistant_markdown(ix, text, mermaid_images, t)
        }
        ChatItem::AssistantText { text, .. } => {
            let key = FoldKey::Assistant(ix);
            let expanded = fold.is_expanded(&key, is_active(item));
            assistant_block(ix, key, expanded, text, mermaid_images, t, cx).into_any_element()
        }
        ChatItem::Thinking { text, .. } => {
            let key = FoldKey::Thinking(ix);
            let expanded = fold.is_expanded(&key, is_active(item));
            thinking_block(ix, key, expanded, text, mermaid_images, t, cx).into_any_element()
        }
        ChatItem::ToolCall(tc) => {
            let key = FoldKey::Tool(tc.id.clone());
            let expanded = fold.is_expanded(&key, is_active(item));
            tool_card(key, expanded, tc, diff_editors, diff_stats, fold, t, cx).into_any_element()
        }
        ChatItem::Permission(card) => permission_card(ix, card, t, cx).into_any_element(),
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
    key: FoldKey,
    expanded: bool,
    header: AnyElement,
    summary: Option<AnyElement>,
    body: AnyElement,
    header_chrome: F,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<Id, F> {
    // Same `disclosure_row` scaffold as the section bars (chevron + click), then
    // append this block's own header content.
    let mut header_row = disclosure_row(id, key, expanded, t, cx).child(header);
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
/// The assistant prose body — drag-selectable rendered markdown with mermaid
/// fences rasterized. Shared by the labeled [`assistant_block`] (trivial /
/// top-level reply) and the header-less inline render used under a response bar.
fn assistant_markdown(
    ix: usize,
    text: &str,
    mermaid_images: &MermaidImages,
    t: &theme::DarudaTheme,
) -> AnyElement {
    crate::ui::markdown(("agent-chat-md-assistant", ix), text.to_string())
        .color(t.text_primary)
        .text_size(px(theme::AGENT_CHAT_MSG_FONT_SIZE))
        .code_block_render(mermaid_code_block_render(mermaid_images))
        .into_any_element()
}

fn assistant_block(
    ix: usize,
    key: FoldKey,
    expanded: bool,
    text: &str,
    mermaid_images: &MermaidImages,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let body_el = assistant_markdown(ix, text, mermaid_images, t);
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

/// The turn's conclusion — the run's final assistant message rendered under a
/// response bar. Same drag-selectable markdown body as [`assistant_block`] but
/// with no "Agent" label (the response bar above already names the speaker):
/// just the bare disclosure chevron, so the conclusion folds to its first-line
/// summary independently of the response's process fold.
fn conclusion_block(
    ix: usize,
    key: FoldKey,
    expanded: bool,
    text: &str,
    mermaid_images: &MermaidImages,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let body_el = assistant_markdown(ix, text, mermaid_images, t);
    let summary = collapsed_text_summary(text, false, t);
    foldable_block(
        ("agent-chat-conclusion", ix),
        key,
        expanded,
        gpui::Empty.into_any_element(),
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
    ix: usize,
    key: FoldKey,
    expanded: bool,
    text: &str,
    mermaid_images: &MermaidImages,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
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
    key: FoldKey,
    expanded: bool,
    tc: &ToolCallItem,
    diff_editors: &DiffEditors,
    diff_stats: &DiffStats,
    fold: &FoldState,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let (badge_text, badge_fg) = tool_status_badge(tc.status, t);
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
            &tc.id, di, diff, editor, diff_stats, fold, t, cx,
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
    }
}

/// Inline permission card — title + one button per choice. Once resolved,
/// the buttons are gone and the chosen option is shown instead.
fn permission_card(
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
    button.on_click(cx.listener(move |this, _, _window, cx| {
        this.respond_permission(option_id.clone(), kind, cx);
    }))
}

#[cfg(test)]
mod tests {
    use super::{plan_progress, running_tool_title};
    use daruda_acp::{
        ChatItem, PlanEntryView, PlanPriority, PlanStatus, ToolCallItem, ToolKindView,
        ToolStatusView,
    };

    fn plan_entry(status: PlanStatus) -> PlanEntryView {
        PlanEntryView {
            content: "step".into(),
            priority: PlanPriority::Medium,
            status,
        }
    }

    #[test]
    fn plan_progress_empty_is_zero_over_zero() {
        assert_eq!(plan_progress(&[]), (0, 0));
    }

    #[test]
    fn plan_progress_counts_only_completed() {
        let plan = [
            plan_entry(PlanStatus::Completed),
            plan_entry(PlanStatus::InProgress),
            plan_entry(PlanStatus::Pending),
            plan_entry(PlanStatus::Completed),
        ];
        assert_eq!(plan_progress(&plan), (2, 4));
    }

    #[test]
    fn plan_progress_all_completed_is_total_over_total() {
        let plan = [
            plan_entry(PlanStatus::Completed),
            plan_entry(PlanStatus::Completed),
            plan_entry(PlanStatus::Completed),
        ];
        assert_eq!(plan_progress(&plan), (3, 3));
    }

    fn tool(id: &str, status: ToolStatusView) -> ChatItem {
        ChatItem::ToolCall(ToolCallItem {
            id: id.to_owned(),
            title: format!("Tool {id}"),
            kind: ToolKindView::Edit,
            status,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
        })
    }

    #[test]
    fn running_tool_title_is_none_without_an_in_progress_call() {
        let items = [
            ChatItem::AssistantText {
                text: "a".into(),
                streaming: true,
                message_id: None,
            },
            tool("c1", ToolStatusView::Completed),
        ];
        assert_eq!(running_tool_title(&items), None);
    }

    #[test]
    fn running_tool_title_picks_the_last_in_progress_call() {
        // Completed earlier calls are skipped; the latest in-progress one wins.
        let items = [
            tool("c1", ToolStatusView::Completed),
            tool("c2", ToolStatusView::InProgress),
            tool("c3", ToolStatusView::Pending),
        ];
        assert_eq!(running_tool_title(&items), Some("Tool c2".to_owned()));
    }
}
