//! Pure view of an `&AgentChatView`.
//!
//! MVU view purity: event closures one-line dispatch into view ops, so state
//! changes stay outside render. Message selection keys use append-only item
//! indexes; tool diffs embed cached read-only editors when available.

mod blocks;
mod chrome;
mod diff;
/// The height-capped editor embed. Reachable from `workspace/tests` so the
/// layout probe measures the shipped builder rather than a copy of it.
pub(in crate::workspace) mod embed;
mod filter;
mod fold_header;
mod fold_mode;
mod links;
mod mermaid;
/// Reachable from `workspace::screenshot_scenario` so the
/// `mermaid-lightbox` capture scenario can drive it directly.
pub(in crate::workspace) mod mermaid_lightbox;
mod options_panel;
mod plan;
mod tail_row;
mod tail_window;
mod tool;

use daruda_acp::ChatItem;
use gpui::{
    AnyElement, AnyWindowHandle, Entity, IntoElement, ListSizingBehavior, MouseButton,
    SharedString, Window, canvas, div, list, prelude::*, px,
};

/// Read-only diff editor entities keyed by `"{tool_call_id}#{diff_index}"`
/// (built in the ops layer; this view only embeds them). `pub(in
/// crate::workspace)` rather than `pub(super)`: `AgentChatView::assets`
/// (`view/mod.rs`) uses this as its field type too, so both the owning cache
/// and its read-only render-side view share one definition.
pub(in crate::workspace) type DiffEditors =
    std::collections::HashMap<String, Entity<crate::ui::InputState>>;

/// Read-only editor entities for verbatim tool-output blocks keyed by
/// `"{tool_call_id}#{block_index}"` (built in the ops layer; this view only
/// embeds them). `pub(in crate::workspace)` rather than `pub(super)` for the
/// same reason as [`DiffEditors`]: `AgentChatView::assets` uses it as a field
/// type too.
pub(in crate::workspace) type OutputEditors =
    std::collections::HashMap<String, Entity<crate::ui::InputState>>;

/// Per-diff `+N −M` line counts keyed by `"{tool_call_id}#{diff_index}"`
/// (built in the ops layer; this view only reads them for the collapsed
/// diff summary).
pub(in crate::workspace) type DiffStats = std::collections::HashMap<String, DiffStat>;

/// Rendered mermaid diagrams keyed by source hash. Shared so the cached
/// markdown code-block hook can see async image arrivals after parse.
pub(in crate::workspace) type MermaidImages = std::sync::Arc<
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
pub(in crate::workspace) type ToolImages = std::sync::Arc<
    std::sync::Mutex<
        std::collections::HashMap<
            u64,
            Option<crate::workspace::main_area::file_view_pane::render::CachedImage>,
        >,
    >,
>;

/// The `AgentChatView::assets` caches the render pass reads, borrowed as one
/// parameter instead of five threaded through `render_item` → `tool_card` →
/// `output_block_view` (and `tool_card`'s recursion into flattened subagent
/// children). Read-only here: entries are built in the reconcile layer.
#[derive(Clone, Copy)]
pub(super) struct RenderAssets<'a> {
    pub(super) diff_editors: &'a DiffEditors,
    pub(super) diff_stats: &'a DiffStats,
    pub(super) output_editors: &'a OutputEditors,
    pub(super) tool_images: &'a ToolImages,
    pub(super) mermaid_images: &'a MermaidImages,
}

impl<'a> RenderAssets<'a> {
    fn of(assets: &'a AssetCache) -> Self {
        Self {
            diff_editors: &assets.diff_editors,
            diff_stats: &assets.diff_stats,
            output_editors: &assets.output_editors,
            tool_images: &assets.tool_images,
            mermaid_images: &assets.mermaid_images,
        }
    }
}

use blocks::{
    MarkdownRender, assistant_block, assistant_markdown, conclusion_block, failure_block,
    thinking_block, user_bubble,
};
use chrome::{ActivityBarProps, activity_bar, status_banner, working_indicator};
use fold_header::{FoldHeader, FoldRow, SummaryLine, outside_window_rail, rollup_glyph};
use links::AgentChatMarkdownLinks;
use plan::plan_region;
use tail_row::tail_more_bar;
use tool::{permission_card, tool_card};

use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::theme::PaneSurfaceTokens;
use crate::ui::{IconName, StatusPulseClock, button_bare};
use crate::workspace::main_area::agent_chat_pane::agent_chat_helpers::{
    DiffStat, Rollup, TurnBoundary, agent_run, fold_context_at, tool_fold_key,
};
use crate::workspace::main_area::agent_chat_pane::fold::{FoldKey, FoldState};
use crate::workspace::main_area::agent_chat_pane::rows::{
    FilterMatchIndex, FilteredAway, LiveSubagentUnits, RenderRow, RowKind,
};
use crate::workspace::main_area::agent_chat_pane::view::{
    AgentChatView, AssetCache, ChatContentWidth,
};
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
    #[cfg(feature = "screenshot")]
    let (filter_popover_open, fold_popover_open, options_popover_open) = (
        content.screenshot_filter_open,
        content.screenshot_fold_open,
        content.screenshot_options_open,
    );
    #[cfg(not(feature = "screenshot"))]
    let (filter_popover_open, fold_popover_open, options_popover_open) = (false, false, false);
    // Screenshot scenarios force the layout that owns the requested popover.
    let compact_options = match (
        options_popover_open,
        filter_popover_open || fold_popover_open,
    ) {
        (true, _) => true,
        (false, true) => false,
        (false, false) => content.activity_bar_is_compact(cx),
    };

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
    let bar = activity_bar(
        ActivityBarProps {
            pane_id,
            agent_id: &content.agent_id,
            title: content
                .activity_title()
                .or(Some(content.agent_name.as_str())),
            last_active: content.session_updated_at.as_deref(),
            usage: content.session_usage.as_ref(),
            has_items: !content.items.is_empty(),
            content_width: content.content_width,
            tail: content.tail,
            display_filter: content.display_filter,
            fold_mode: content.fold.mode_choice(),
            fold_editor_turn: content.fold_editor_turn,
            custom_fold_mode: content.custom_fold_mode,
            activity_options_tab: content.activity_options_tab,
            compact_options,
            filter_popover_open,
            fold_popover_open,
            options_popover_open,
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
        // The first and last *visible* rows carry the list's outer `PAD_Y` (vs
        // `LIST_GAP` between rows); hidden rows are zero-height so they don't
        // count — row 0 is often the always-emitted filter placeholder.
        let visible = VisibleEnds::of(&content.rows);
        let list_el = list(
            content.list_state.clone(),
            cx.processor(move |this, ix, window, cx| match this.rows.get(ix) {
                Some(row) => render_row(this, ix, row, visible, &t_items, window, cx),
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
            // longer painted. Capture phase, not bubble: a floating descendant
            // (the jump-to-bottom button, a diagram card's action row) stops
            // propagation on its own click so it doesn't double-fire the row
            // beneath, and a bubble listener here would never see that release
            // — leaving the poll ticking until the next mouse move.
            .capture_any_mouse_up(cx.listener(|this, ev: &gpui::MouseUpEvent, _window, _cx| {
                if ev.button == MouseButton::Left {
                    this.end_selection_drag();
                }
            }))
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
            .into_any_element()
    };

    // Capture pane-root width even when the transcript list is absent.
    let width_capture = {
        let view = cx.entity();
        canvas(
            move |bounds, _window, cx| {
                view.update(cx, |v, cx| v.set_pane_width(bounds.size.width, cx));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    };

    div()
        .size_full()
        .relative()
        .flex()
        .flex_col()
        .child(width_capture)
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

/// The first and last rows that actually paint. Hidden rows are zero-height, so
/// the list's outer padding belongs to these two rather than to index `0` and
/// `len - 1`.
#[derive(Clone, Copy)]
struct VisibleEnds {
    first: usize,
    last: usize,
}

impl VisibleEnds {
    fn of(rows: &[RenderRow]) -> Self {
        Self {
            first: rows.iter().position(|r| !r.hidden).unwrap_or(0),
            last: rows.iter().rposition(|r| !r.hidden).unwrap_or(0),
        }
    }
}

/// Render one projected row: an item, a synthetic fold header, or a zero-height
/// `Empty` when collapsed under an ancestor fold (the row stays in the sequence
/// so the count is fold-stable). Applies per-row padding (`PAD_Y` on the first
/// and last visible rows, `LIST_GAP` between) and a left indent per level.
fn render_row(
    this: &AgentChatView,
    ix: usize,
    row: &RenderRow,
    visible: VisibleEnds,
    t: &theme::DarudaTheme,
    window: &mut Window,
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
        RowKind::ResponseHeader {
            run_start,
            collapsed,
            filtered,
        } => response_bar(
            this,
            *run_start,
            *collapsed,
            *filtered,
            row.filter_revealed,
            t,
            cx,
        )
        .into_any_element(),
        // One block among siblings — it reports nothing about the run, so no
        // rollup glyph; the response bar above it carries the run's.
        RowKind::AgentItem(i) => render_agent_item(this, *i, row, t, window, cx),
        RowKind::TailMore {
            run_start,
            hidden_steps,
            kept_steps,
            collapsed,
        } => tail_more_bar(this, *run_start, *hidden_steps, *kept_steps, *collapsed, cx),
        RowKind::ToolGroupHeader {
            gid,
            first_ix,
            count,
            collapsed,
        } => tool_group_bar(
            this,
            gid,
            *first_ix..*first_ix + *count,
            *collapsed,
            row.filter_revealed,
            t,
            cx,
        )
        .into_any_element(),
        RowKind::ThinkingGroupHeader {
            first_ix,
            count,
            collapsed,
        } => thinking_group_bar(
            this,
            *first_ix,
            *first_ix..*first_ix + *count,
            *collapsed,
            row.filter_revealed,
            cx,
        )
        .into_any_element(),
        RowKind::ConclusionItem(i) => match this.items.get(*i) {
            Some(ChatItem::AssistantText { text, .. }) => {
                let key = FoldKey::Assistant(*i);
                let expanded = this.fold.is_expanded(
                    &key,
                    fold_context_at(&key, *i, &this.items, this.turn_boundary),
                );
                conclusion_block(
                    *i,
                    key,
                    expanded,
                    text,
                    MarkdownRender::new(
                        &this.assets.mermaid_images,
                        this.dim_amount,
                        AgentChatMarkdownLinks::new(this.pane_id, this.window_handle),
                    ),
                    cx,
                )
                .into_any_element()
            }
            _ => gpui::Empty.into_any_element(),
        },
        RowKind::WorkingIndicator => working_indicator(this, cx).into_any_element(),
    };
    // Rows outside the tail window's kept range, whatever surfaced them.
    let inner = if row.outside_window {
        outside_window_rail(inner, this.dim_amount, cx)
    } else {
        inner
    };
    let bottom = if ix == visible.last {
        theme::AGENT_CHAT_PAD_Y
    } else {
        theme::AGENT_CHAT_LIST_GAP
    };
    // A new turn (a `User` row past the first) gets extra top space so
    // consecutive turns read as distinct exchanges.
    let turn_break = ix != visible.first && matches!(row.kind, RowKind::User(_));
    let row_el = div()
        .w_full()
        .min_w_0()
        .px(px(theme::AGENT_CHAT_PAD_X))
        .when(ix == visible.first, |d| d.pt(px(theme::AGENT_CHAT_PAD_Y)))
        .when(turn_break, |d| d.mt(px(theme::AGENT_CHAT_TURN_GAP)))
        .pb(px(bottom))
        // Nest one content-pad unit per level (group members sit under their bar).
        .when(row.indent > 0, |d| {
            d.pl(px(theme::AGENT_CHAT_PAD_X * (row.indent as f32 + 1.0)))
        })
        .child(inner);
    match this.content_width {
        ChatContentWidth::Full => row_el.into_any_element(),
        ChatContentWidth::Reading => div()
            .w_full()
            .min_w_0()
            .flex()
            .justify_center()
            .child(row_el.max_w(px(theme::agent_chat_reading_width(cx))))
            .into_any_element(),
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

/// Collapsible header for an agent response (agent items under a user message).
/// The whole row toggles `FoldKey::Response`; the agent label leads, the
/// response's first line fills the row when collapsed, and the tool count plus
/// the status-rollup glyph sit at the right edge.
fn response_bar(
    this: &AgentChatView,
    run_start: usize,
    collapsed: bool,
    filtered: FilteredAway,
    filter_revealed: bool,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let run = agent_run(&this.items, run_start);
    let tools = run_tools(&this.items, run.clone());
    // The response's opening prose — the first item that yields a preview, so a
    // turn that opened with reasoning still previews something and an empty
    // leading block (a streaming placeholder that has not filled yet) falls
    // through to the next rather than blanking the summary.
    let summary_run = run.clone();
    let items = &this.items;
    let mut header = FoldHeader::with_summary(move || {
        summary_run
            .filter_map(|k| match items.get(k) {
                Some(ChatItem::AssistantText { text, .. } | ChatItem::Thinking { text, .. }) => {
                    SummaryLine::from_markdown(text)
                }
                _ => None,
            })
            .next()
    })
    .leading(agent_label(this, cx).into_any_element());
    // The filter's reveal sits left of the run's own counts, so the numbers that
    // are always there keep the right edge and do not shift when it appears.
    if filtered.offers_reveal() {
        let surface = PaneSurfaceTokens::agent_chat(cx).dimmed(this.dim_amount);
        header = header.trailing(filter::filtered_chip(
            run.start,
            filtered,
            filter_revealed,
            &surface,
            cx,
        ));
    }
    // Trailing content is fold-state-independent (see `FoldHeader::trailing`), so
    // the count reads the same expanded or collapsed — and, unlike a group bar's,
    // the same filtered or not. A group that loses every call to the filter stops
    // rendering, but this bar always renders, so a filter-aware count here would
    // leave the turn showing no trace of work it did.
    if tools > 0 {
        header = header.trailing(count_label(s::agent_chat_tool_group_count(tools), this, cx));
    }
    let header = header.trailing(rollup_glyph(
        Rollup::of_kept_run(&this.items, run, &this.live_units, |item| {
            filter_revealed || this.filter_matches.matches(item)
        }),
        t,
        cx,
    ));
    // Borderless section bar, matching the block headers — section headers stay
    // light; only content cards (`tool_card`) carry box chrome.
    FoldRow::section(
        SharedString::from(format!("agent-chat-response-{run_start}")),
        FoldKey::Response(run_start),
        !collapsed,
        header,
    )
    .render(this.dim_amount, cx)
}

/// The agent name as a fold header's leading label.
fn agent_label(this: &AgentChatView, cx: &Context<AgentChatView>) -> impl IntoElement + use<> {
    div()
        .flex_none()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(this.dim(theme::agent_chat_fg(cx)))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(SharedString::from(agent_display_name(this).to_string()))
}

/// Every tool call the run made, whatever the display filter hides.
///
/// The response bar summarizes the turn rather than disclosing a fixed set of
/// rows, so its number describes what happened. [`kept_tools`] is the other
/// half of that split: a group bar *is* the disclosure over its calls, so its
/// number has to be what expanding it puts on screen.
fn run_tools(items: &[ChatItem], run: std::ops::Range<usize>) -> usize {
    run.filter(|&k| matches!(items.get(k), Some(ChatItem::ToolCall(_))))
        .count()
}

/// Tool calls in `run` that the current projection displays — filter matches
/// normally, or the whole run while the filtered-row disclosure is open. The
/// count a group bar prints, because expanding it is what puts those rows on
/// screen; see [`run_tools`] for the response bar's different rule.
fn kept_tools(this: &AgentChatView, run: std::ops::Range<usize>, filter_revealed: bool) -> usize {
    run.filter(|&k| {
        matches!(this.items.get(k), Some(ChatItem::ToolCall(tc)) if filter_revealed || this.filter_matches.keeps_tool(tc))
    })
    .count()
}

/// Thinking items in `run` that the current projection displays. Mirrors
/// [`kept_tools`]: the number on a disclosure has to be what expanding it puts
/// on screen.
fn kept_thoughts(
    this: &AgentChatView,
    run: std::ops::Range<usize>,
    filter_revealed: bool,
) -> usize {
    run.filter(|&k| {
        matches!(this.items.get(k), Some(item @ ChatItem::Thinking { .. }) if filter_revealed || this.filter_matches.matches(item))
    })
    .count()
}

/// A right-anchored count in a fold header's trailing slot.
fn count_label(label: String, this: &AgentChatView, cx: &Context<AgentChatView>) -> AnyElement {
    div()
        .flex_none()
        .text_color(this.dim(theme::agent_chat_fg_subtle(cx)))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(SharedString::from(label))
        .into_any_element()
}

/// Collapsible header for a consecutive tool-call group. The whole row toggles
/// the group's fold (`FoldKey::ToolGroup`); shows a chevron, the "N tool calls"
/// count, and a status-rollup glyph.
fn tool_group_bar(
    this: &AgentChatView,
    gid: &str,
    run: std::ops::Range<usize>,
    collapsed: bool,
    filter_revealed: bool,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let rollup = Rollup::of_kept_run(&this.items, run.clone(), &this.live_units, |item| {
        filter_revealed || this.filter_matches.matches(item)
    });
    // The count is the group's own identity, not a preview of folded content, so
    // it shows in both states — hence `plain` rather than a markdown summary.
    // `count` is the group's structural span; what the row offers is the part of
    // it the display filter keeps.
    let label = s::agent_chat_tool_group_count(kept_tools(this, run, filter_revealed));
    let header =
        FoldHeader::with_title(group_title(label, this, cx)).trailing(rollup_glyph(rollup, t, cx));
    // Borderless section bar, same as the response bar.
    FoldRow::section(
        SharedString::from(format!("agent-chat-toolgroup-{gid}")),
        FoldKey::ToolGroup(gid.to_string()),
        !collapsed,
        header,
    )
    .render(this.dim_amount, cx)
}

/// A group bar's count, as the identifier its stretch slot shows in both fold
/// states. Shared so the two group bars cannot drift apart on colour or size.
fn group_title(label: String, this: &AgentChatView, cx: &Context<AgentChatView>) -> AnyElement {
    div()
        .text_color(this.dim(theme::agent_chat_fg_muted(cx)))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(SharedString::from(label))
        .into_any_element()
}

/// Collapsible header for a consecutive thinking run. The whole row toggles the
/// group's fold (`FoldKey::ThinkingGroup`). No leading marker and no rollup
/// glyph: a thought has no success or failure state to report.
fn thinking_group_bar(
    this: &AgentChatView,
    first_ix: usize,
    run: std::ops::Range<usize>,
    collapsed: bool,
    filter_revealed: bool,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    // The count is the group's own identity, so it shows in both fold states —
    // and what it names is the part of the run the display filter keeps.
    let label = s::agent_chat_thinking_group_count(kept_thoughts(this, run, filter_revealed));
    let header = FoldHeader::with_title(group_title(label, this, cx));
    // Borderless section bar, same as the tool-group bar.
    FoldRow::section(
        SharedString::from(format!("agent-chat-thinkgroup-{first_ix}")),
        FoldKey::ThinkingGroup(first_ix),
        !collapsed,
        header,
    )
    .render(this.dim_amount, cx)
}

/// Floating "jump to bottom" affordance shown when the user has scrolled up
/// (tail-follow released). Positioned bottom-right via the parent's `relative`.
fn scroll_to_bottom_button(
    pane_id: PaneId,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    div()
        .debug_selector(|| "agent-chat-scroll-bottom".into())
        .absolute()
        .bottom(px(theme::AGENT_CHAT_SCROLL_BTN_INSET))
        .right(px(theme::AGENT_CHAT_SCROLL_BTN_INSET))
        .child(
            button_bare(("agent-chat-scroll-bottom", pane_id as usize))
                .icon(IconName::ArrowDown)
                .on_click(cx.listener(move |this, _ev, _window, cx| {
                    // This button floats over the transcript, and gpui
                    // hit-tests every hitbox under the pointer — without this
                    // the click also lands on whatever row happens to sit
                    // beneath it (a fold header toggles, a diagram card opens
                    // its lightbox).
                    cx.stop_propagation();
                    this.scroll_to_bottom(cx);
                })),
        )
}

/// Look up an item row's `ChatItem` and render it. The run's verdict glyph is the
/// response bar's — a block never reports one for the response it sits in.
fn render_agent_item(
    this: &AgentChatView,
    ix: usize,
    row: &RenderRow,
    t: &theme::DarudaTheme,
    window: &mut Window,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    match this.items.get(ix) {
        Some(item) => render_item(
            ix,
            item,
            row.indent > 0,
            &this.items,
            &this.live_units,
            &this.filter_matches,
            row.filter_revealed,
            this.turn_boundary,
            RenderAssets::of(&this.assets),
            &this.fold,
            t,
            this.dim_amount,
            agent_display_name(this),
            this.pane_id,
            this.window_handle,
            window,
            cx,
        ),
        None => gpui::Empty.into_any_element(),
    }
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
    live_units: &LiveSubagentUnits,
    filter_matches: &FilterMatchIndex,
    filter_revealed: bool,
    boundary: TurnBoundary,
    assets: RenderAssets<'_>,
    fold: &FoldState,
    t: &theme::DarudaTheme,
    dim: f32,
    agent_label: &str,
    pane_id: PaneId,
    window_handle: AnyWindowHandle,
    window: &mut Window,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let mermaid_images = assets.mermaid_images;
    let markdown = MarkdownRender::new(
        mermaid_images,
        dim,
        AgentChatMarkdownLinks::new(pane_id, window_handle),
    );
    match item {
        ChatItem::UserText(text) => user_bubble(ix, text, dim, cx).into_any_element(),
        // Under a response bar the speaker is already labeled with the agent
        // name; render the prose inline with no redundant per-block header/fold.
        // A trivial / top-level reply keeps the labeled, foldable block.
        ChatItem::AssistantText { text, .. } if under_response => {
            assistant_markdown(ix, text, markdown, cx)
        }
        ChatItem::AssistantText { text, .. } => {
            let key = FoldKey::Assistant(ix);
            let expanded = fold.is_expanded(&key, fold_context_at(&key, ix, items, boundary));
            assistant_block(ix, key, expanded, text, agent_label, markdown, cx)
        }
        ChatItem::Thinking { text, .. } => {
            let key = FoldKey::Thinking(ix);
            let expanded = fold.is_expanded(&key, fold_context_at(&key, ix, items, boundary));
            thinking_block(ix, key, expanded, text, markdown, cx).into_any_element()
        }
        ChatItem::ToolCall(tc) => {
            let key = tool_fold_key(tc);
            let expanded = fold.is_expanded(&key, fold_context_at(&key, ix, items, boundary));
            tool_card(
                key,
                expanded,
                ix,
                tc,
                items,
                live_units,
                filter_matches,
                filter_revealed,
                boundary,
                assets,
                fold,
                t,
                dim,
                0,
                pane_id,
                window_handle,
                window,
                cx,
            )
            .into_any_element()
        }
        ChatItem::Permission(card) => permission_card(ix, card, t, dim, cx).into_any_element(),
        ChatItem::Failure(failure) => {
            failure_block(ix, failure, pane_id, window_handle, t, cx).into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: RowKind, hidden: bool) -> RenderRow {
        RenderRow::at(kind, hidden, 0)
    }

    fn tool(id: &str) -> ChatItem {
        ChatItem::ToolCall(daruda_acp::ToolCallItem {
            id: id.to_owned(),
            title: "Tool".into(),
            kind: daruda_acp::ToolKindView::Read,
            tool_name: None,
            status: daruda_acp::ToolStatusView::Completed,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: None,
            exit: None,
        })
    }

    /// The response bar summarizes the turn, so its count says what the turn
    /// did. The bar renders whatever the filter hides, so a count that shrank
    /// with the filter would leave a fully narrowed turn showing no trace of
    /// its work — `kept_tools` is the disclosure half of that split.
    #[test]
    fn the_runs_tool_count_ignores_what_the_filter_hides() {
        let items = [
            ChatItem::AssistantText {
                text: "looking".into(),
                streaming: false,
                message_id: None,
                phase: daruda_acp::MessagePhase::Answer,
            },
            tool("a"),
            tool("b"),
            ChatItem::Thinking {
                text: "hm".into(),
                streaming: false,
                message_id: None,
            },
            tool("c"),
        ];
        assert_eq!(run_tools(&items, 0..items.len()), 3);
        // Scoped to the run it is given, not the whole transcript.
        assert_eq!(run_tools(&items, 0..2), 1);
    }

    /// A run can open with a hidden row — a tail boundary covering nothing, a
    /// folded block — so the list's top padding has to follow the first row
    /// that paints, not index 0.
    #[test]
    fn the_outer_padding_follows_the_rows_that_paint() {
        let rows = [
            row(
                RowKind::TailMore {
                    run_start: 0,
                    hidden_steps: 0,
                    kept_steps: 0,
                    collapsed: true,
                },
                true,
            ),
            row(RowKind::AgentItem(0), false),
            row(RowKind::AgentItem(1), false),
            row(RowKind::AgentItem(2), true),
        ];
        let ends = VisibleEnds::of(&rows);
        assert_eq!(ends.first, 1);
        assert_eq!(ends.last, 2);
    }

    #[test]
    fn an_all_hidden_list_collapses_both_ends_onto_row_zero() {
        let rows = [row(RowKind::AgentItem(0), true)];
        let ends = VisibleEnds::of(&rows);
        assert_eq!((ends.first, ends.last), (0, 0));
    }
}
