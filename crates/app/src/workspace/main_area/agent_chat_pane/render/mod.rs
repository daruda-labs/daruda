//! Pure view of an `&AgentChatView`.
//!
//! MVU view purity: event closures one-line dispatch into view ops, so state
//! changes stay outside render. Message selection keys use append-only item
//! indexes; tool diffs embed cached read-only editors when available.

mod axis_chip;
mod blocks;
mod chrome;
mod diff;
/// The height-capped editor embed. Reachable from `workspace/tests` so the
/// layout probe measures the shipped builder rather than a copy of it.
pub(in crate::workspace) mod embed;
mod filter;
mod fold_header;
mod fold_mode;
/// Reachable from `workspace::screenshot_scenario` so the
/// `mermaid-lightbox` capture scenario can drive it directly.
pub(in crate::workspace) mod image_lightbox;
mod links;
mod mermaid;
mod plan;
mod status_icon;
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
pub(super) type DiffEditors = std::collections::HashMap<String, Entity<crate::ui::InputState>>;

/// Read-only editor entities for verbatim tool-output blocks keyed by
/// `"{tool_call_id}#{block_index}"` (built in the ops layer; this view only
/// embeds them). `pub(super)` rather than `pub(super)` for the
/// same reason as [`DiffEditors`]: `AgentChatView::assets` uses it as a field
/// type too.
pub(super) type OutputEditors = std::collections::HashMap<String, Entity<crate::ui::InputState>>;

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

/// Decoded local-image `ResourceLink`s keyed by tool output block identity.
/// `Some` = decoded & GPU-ready; `None` = a cached read/decode failure. Kept
/// separate from `ToolImages` because the source is a mutable path rather than
/// immutable inline base64 content.
pub(super) type ResourceImages = std::sync::Arc<
    std::sync::Mutex<
        std::collections::HashMap<
            String,
            Option<crate::workspace::main_area::file_view_pane::render::CachedImage>,
        >,
    >,
>;

/// The `AgentChatView::assets` caches the render pass reads, borrowed as one
/// parameter instead of six threaded through `render_item` → `tool_card` →
/// `output_block_view` (and `tool_card`'s recursion into flattened subagent
/// children). Read-only here: entries are built in the reconcile layer.
#[derive(Clone, Copy)]
pub(super) struct RenderAssets<'a> {
    pub(super) diff_editors: &'a DiffEditors,
    pub(super) diff_stats: &'a DiffStats,
    pub(super) output_editors: &'a OutputEditors,
    pub(super) tool_images: &'a ToolImages,
    pub(super) resource_images: &'a ResourceImages,
    pub(super) mermaid_images: &'a MermaidImages,
}

impl<'a> RenderAssets<'a> {
    fn of(assets: &'a AssetCache) -> Self {
        Self {
            diff_editors: &assets.diff_editors,
            diff_stats: &assets.diff_stats,
            output_editors: &assets.output_editors,
            tool_images: &assets.tool_images,
            resource_images: &assets.resource_images,
            mermaid_images: &assets.mermaid_images,
        }
    }
}

use blocks::{
    MarkdownRender, assistant_markdown, conclusion_block, failure_block, thinking_block,
    user_bubble,
};
use chrome::{ActivityBarProps, activity_bar, status_banner, working_indicator};
use fold_header::{FoldHeader, FoldRow, SummaryLine, interrupted_row, rollup_glyph};
use links::AgentChatMarkdownLinks;
use plan::plan_region;
use tail_row::{tail_more_bar, tool_group_tail_more_bar};
use tool::{CardContext, permission_card, tool_card};

use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::theme::PaneSurfaceTokens;
use crate::ui::{Icon, IconName, Sizable as _, StatusPulseClock, button_bare};
use crate::workspace::main_area::agent_chat_pane::agent_chat_helpers::{
    DiffStat, Rollup, TurnBoundary, fold_context_at,
};
use crate::workspace::main_area::agent_chat_pane::fold::{FoldKey, FoldState};
use crate::workspace::main_area::agent_chat_pane::rows::tail::TailWindow;
use crate::workspace::main_area::agent_chat_pane::rows::{
    FilterMatchIndex, FilteredAway, LiveSubagentUnits, RenderRow, RowKind,
};
use crate::workspace::main_area::agent_chat_pane::transcript_structure::response_run;
use crate::workspace::main_area::agent_chat_pane::view::{
    AgentChatView, AssetCache, ChatContentWidth, TurnRecord,
};
use crate::workspace::main_area::pane_tree::PaneId;

/// Build the element tree for an Agent chat pane.
pub(super) fn render(view: &AgentChatView, cx: &mut Context<AgentChatView>) -> impl IntoElement {
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
            tail: tail_window::TailChoices {
                steps: content.tail_steps,
                calls: content.tail_calls,
            },
            display_filter: content.display_filter,
            fold_mode: content.fold.mode_choice(),
            fold_editor: content.fold_editor,
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
        // `render_row` dispatches by kind with per-row padding + nesting indent.
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
        .font_family(theme::agent_chat_font_family(cx))
        .line_height(gpui::relative(theme::agent_chat_line_height(cx)))
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
        RowKind::Interrupted(_) => interrupted_row(this.dim_amount, cx),
        RowKind::ResponseHeader {
            run_start,
            categories,
            collapsed,
            filtered,
        } => response_bar(
            this,
            *run_start,
            categories,
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
        RowKind::ToolGroupTailMore {
            gid,
            hidden_calls,
            kept_calls,
            collapsed,
        } => tool_group_tail_more_bar(this, gid, *hidden_calls, *kept_calls, *collapsed, cx),
        RowKind::ToolGroupHeader {
            gid,
            calls,
            collapsed,
        } => tool_group_bar(this, gid, calls, *collapsed, row.filter_revealed, t, cx)
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
                    turn_stats_element(this, *i, cx),
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
    let bottom = if ix == visible.last {
        theme::AGENT_CHAT_PAD_Y
    } else {
        theme::AGENT_CHAT_LIST_GAP
    };
    // A new turn (a `User` row past the first) gets extra top space so
    // consecutive turns read as distinct exchanges.
    let turn_break = ix != visible.first && matches!(row.kind, RowKind::User(_));
    let body = div()
        .w_full()
        .min_w_0()
        .pb(px(bottom))
        // Depth is carried by the indent alone — one content-pad unit per level
        // (group members sit under their bar), no rule down the left.
        .when(row.indent > 0, |d| {
            d.pl(px(theme::AGENT_CHAT_PAD_X * row.indent as f32))
        })
        .child(inner);
    let row_el = div()
        .w_full()
        .min_w_0()
        .px(px(theme::AGENT_CHAT_PAD_X))
        .when(ix == visible.first, |d| d.pt(px(theme::AGENT_CHAT_PAD_Y)))
        .when(turn_break, |d| d.mt(px(theme::AGENT_CHAT_TURN_GAP)))
        .child(body);
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

/// Human-readable elapsed time. `"5s"` under a minute, `"1m05s"` at or over.
/// Shared so the run timer in the working indicator and a tool call's age in
/// its badge are one unit of measure rather than two that happen to agree.
///
/// The arithmetic stays here and the units come from the locale: the seconds
/// are zero-padded before translation because the badge ticks, and a width that
/// changes as the digit rolls over shifts everything beside it.
fn format_elapsed(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        s::agent_chat_elapsed_seconds(secs)
    } else {
        s::agent_chat_elapsed_minutes(secs / 60, format!("{:02}", secs % 60))
    }
}

/// The blink opacity for the shared 2-tick `StatusPulseClock` pulse:
/// `1.0` on even half-ticks (bright), `STATUS_INDICATOR_PULSE_OPACITY_MIN`
/// on odd half-ticks (dim). Read by [`status_icon`](status_icon::status_icon)'s
/// assembly, the one place a live mark blinks, so every surface pulses in
/// lockstep.
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
#[allow(clippy::too_many_arguments)]
fn response_bar(
    this: &AgentChatView,
    run_start: usize,
    categories: &[(crate::transcript::tool_category::ToolCategory, usize)],
    collapsed: bool,
    filtered: FilteredAway,
    filter_revealed: bool,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let run = response_run(&this.items, run_start);
    // The response's opening prose — the first item that yields a preview, so a
    // turn that opened with reasoning still previews something and an empty
    // leading block (a streaming placeholder that has not filled yet) falls
    // through to the next rather than blanking the summary.
    // The bar says what the turn *did*, in both fold states — it is the turn's
    // own identity, not a preview of what the fold hides.
    //
    // A turn that called no tool has no such tally, and an empty slot says less
    // than the old preview did. There, what the agent said *is* what it did, so
    // the bar falls back to the opening prose — collapsed-only, as a preview of
    // hidden content rather than an identity.
    let mut header = if categories.is_empty() {
        let summary_run = run.clone();
        let items = &this.items;
        FoldHeader::with_summary(move || {
            summary_run
                .filter_map(|k| match items.get(k) {
                    Some(
                        ChatItem::AssistantText { text, .. } | ChatItem::Thinking { text, .. },
                    ) => SummaryLine::from_markdown(text),
                    _ => None,
                })
                .next()
        })
    } else {
        FoldHeader::with_title(category_segments(this, categories, cx))
    }
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
    // Sits left of the rollup so the glyph keeps the right edge and the bar does
    // not shift as the timer appears and goes.
    if let Some(elapsed) = running_elapsed(
        AgentChatView::run_start_of(&this.items) == Some(run_start),
        this.activity_elapsed(),
    ) {
        header = header.trailing(trailing_label(
            s::agent_chat_turn_running(format_elapsed(elapsed)),
            this,
            cx,
        ));
    }
    let header = header.trailing(fold_group_status_icon(rollup_glyph(
        Rollup::of_kept_run(&this.items, run, &this.live_units, |item| {
            filter_revealed || this.filter_matches.matches(item)
        }),
        t,
        this.dim_amount,
        cx,
    )));
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

/// How long the turn on the bar has been running, when it is the one still
/// going. `None` for a settled turn: its length already sits on the answer row
/// below, and saying it twice in one turn is what this split avoids — the bar
/// answers "what is happening now", the answer row "how long that took".
///
/// Only the transcript's last run can be live, so a bar whose run is not the
/// last one never carries this however busy the pane is.
fn running_elapsed(
    is_last_run: bool,
    elapsed: Option<std::time::Duration>,
) -> Option<std::time::Duration> {
    is_last_run.then_some(elapsed).flatten()
}

/// A right-anchored label in a fold header's trailing slot. One place so the
/// bar's live timer and the answer row's facts cannot drift on colour or size.
fn trailing_label(label: String, this: &AgentChatView, cx: &Context<AgentChatView>) -> AnyElement {
    div()
        .flex_none()
        .text_color(this.dim(theme::agent_chat_fg_subtle(cx)))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(SharedString::from(label))
        .into_any_element()
}

fn fold_group_status_icon(icon: AnyElement) -> AnyElement {
    div()
        .flex_none()
        .pr(px(theme::AGENT_CHAT_FOLD_STATUS_INSET))
        .child(icon)
        .into_any_element()
}

/// The answer row's trailing facts, when the run that owns this conclusion has
/// a record. Keyed by the run's start so the lookup matches where the record
/// was filed; a restored transcript has none and the row renders bare.
fn turn_stats_element(
    this: &AgentChatView,
    conclusion_ix: usize,
    cx: &Context<AgentChatView>,
) -> Option<AnyElement> {
    // The run that *contains* this conclusion, not one starting at it: the key
    // is the item after the prompt, so the lookup takes the prefix up to and
    // including the conclusion and asks where that prompt's run began.
    let run_start = AgentChatView::run_start_of(this.items.get(..=conclusion_ix)?)?;
    let record = this.activity.turn_records.get(&run_start)?;
    Some(trailing_label(turn_stats_label(record), this, cx))
}

/// The answer row's trailing facts: how long the turn worked, when it finished,
/// and what it emitted. Absent facts are omitted rather than shown as a dash — a
/// restored session has no record, and an agent may report no usage.
///
/// This is what gives the row a reason to exist: it carries no label, so
/// without these it is a bare chevron on an empty line. It mirrors the response
/// bar's own trailing counts one level up.
fn turn_stats_label(record: &TurnRecord) -> String {
    let mut parts = vec![s::agent_chat_turn_worked(format_elapsed(record.worked_for))];
    // Through `timestamp`, not assembled here: that module is where a wall
    // clock becomes text, and it is what keeps ko on a 24-hour clock in this
    // row as it is everywhere else.
    parts.push(crate::surface::timestamp::local_time(record.finished_at));
    if let Some(out) = record.output_tokens {
        parts.push(s::agent_chat_turn_output(abbreviate_tokens(out)));
    }
    parts.join(&s::agent_chat_turn_separator())
}

/// Token counts shortened for a trailing badge: `842`, `1.5k`, `1.6M`. The row
/// is a fixed slot beside the chevron, so the full figure would push the header
/// off the line.
fn abbreviate_tokens(n: u64) -> String {
    // Branch on the *rounded* magnitude: 999_999 is under a million but rounds
    // to "1000.0k", six characters in a slot sized for four.
    match n {
        n if n < 1_000 => n.to_string(),
        n if n < 999_950 => format!("{:.1}k", n as f64 / 1_000.0),
        n if n < 999_950_000 => format!("{:.1}M", n as f64 / 1_000_000.0),
        n => format!("{:.1}B", n as f64 / 1_000_000_000.0),
    }
}

/// A tool group's title: one icon-and-count segment per category it holds,
/// most-numerous first. An over-long title is cut by layout rather than trimmed
/// to the first N categories — a header that dropped a category silently would
/// under-report what the group did. See [`category_segments`] for why the cut is
/// a clip and not an ellipsis.
fn group_category_title(
    this: &AgentChatView,
    calls: &[usize],
    filter_revealed: bool,
    cx: &Context<AgentChatView>,
) -> AnyElement {
    let tally = crate::transcript::tool_category::tally_categories(kept_tool_calls(
        this,
        calls.iter().copied(),
        filter_revealed,
    ));
    category_segments(this, &tally, cx)
}

/// A tally rendered as one icon-and-count segment per category, separated and
/// most-numerous first. Shared by the two bars that carry one — they differ in
/// *what* they count (a group's own calls, filter-aware; a turn's top-level
/// calls, filter-blind) but not in how it reads.
fn category_segments(
    this: &AgentChatView,
    tally: &[(crate::transcript::tool_category::ToolCategory, usize)],
    cx: &Context<AgentChatView>,
) -> AnyElement {
    let fg = this.dim(theme::agent_chat_fg_muted(cx));
    let font_size = px(theme::agent_chat_font_size(cx));
    // The segments are fixed-size, so the row has to be the thing that yields:
    // without `min_w_0` a flex row sizes to its content and runs out under the
    // trailing badges instead of clipping at the slot's edge.
    //
    // It clips rather than ellipsizing — a row of elements has no text to put a
    // `…` on — so the gap has to be reserved by an outer box. gpui masks at the
    // *border* box (`Style::overflow_mask`), inset only for a coloured border
    // and never for padding, so padding on the clipping box itself would be
    // overflowed straight through and the cut would land flush against the
    // badge beside it.
    let mut row = div()
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .flex()
        .flex_row()
        .items_center();
    for (ix, (category, count)) in tally.iter().copied().enumerate() {
        if ix > 0 {
            row = row.child(
                div()
                    .flex_none()
                    .text_color(this.dim(theme::agent_chat_fg_subtle(cx)))
                    .text_size(font_size)
                    .child(SharedString::from(s::agent_chat_group_separator())),
            );
        }
        row = row.child(
            div()
                .flex_none()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::GAP_SM))
                .text_color(fg)
                .text_size(font_size)
                .child(
                    Icon::empty()
                        .path(category_icon(category))
                        .xsmall()
                        .text_color(fg),
                )
                .child(SharedString::from(s::agent_chat_group_category(
                    category.token(),
                    count,
                ))),
        );
    }
    div()
        .w_full()
        .min_w_0()
        .pr(px(theme::AGENT_CHAT_TRAILING_GAP))
        .child(row)
        .into_any_element()
}

/// The glyph for one tool category. A category an ACP kind can name borrows the
/// per-call mapping through a representative kind; the two no kind can name go
/// through [`tool::name_keyed_icon`], the same source their cards use.
fn category_icon(category: crate::transcript::tool_category::ToolCategory) -> SharedString {
    use crate::transcript::tool_category::ToolCategory;
    if let Some(icon) = tool::name_keyed_icon(category) {
        return icon;
    }
    tool::tool_kind_icon(match category {
        ToolCategory::Read => daruda_acp::ToolKindView::Read,
        ToolCategory::Edit => daruda_acp::ToolKindView::Edit,
        ToolCategory::Delete => daruda_acp::ToolKindView::Delete,
        ToolCategory::Search => daruda_acp::ToolKindView::Search,
        ToolCategory::Run => daruda_acp::ToolKindView::Execute,
        ToolCategory::Fetch => daruda_acp::ToolKindView::Fetch,
        // Answered above; the kind they share names something else.
        ToolCategory::Agent | ToolCategory::Mcp | ToolCategory::Other => {
            daruda_acp::ToolKindView::Other
        }
    })
}

/// Tool calls in `run` that the current projection displays — filter matches
/// normally, or the whole run while the filtered-row disclosure is open. What a
/// group bar describes, because expanding it is what puts those rows on screen.
///
/// The turn bar counts by a different rule and does not come through here: it
/// summarizes the turn rather than disclosing rows, so it stays filter-blind
/// and drops a subagent's inner calls (already counted inside their card). That
/// tally is taken in the projection, where the hierarchy already exists.
fn kept_tool_calls(
    this: &AgentChatView,
    indices: impl Iterator<Item = usize>,
    filter_revealed: bool,
) -> impl Iterator<Item = &daruda_acp::ToolCallItem> {
    indices.filter_map(move |k| match this.items.get(k) {
        Some(ChatItem::ToolCall(tc)) if filter_revealed || this.filter_matches.keeps_tool(tc) => {
            Some(tc)
        }
        _ => None,
    })
}

/// Thinking items in `run` that the current projection displays. Mirrors
/// [`kept_tool_calls`]: the number on a disclosure has to be what expanding it
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

/// Collapsible header for a consecutive tool-call group. The whole row toggles
/// the group's fold (`FoldKey::ToolGroup`); shows a chevron, one segment per
/// category the group holds, and a status-rollup glyph.
fn tool_group_bar(
    this: &AgentChatView,
    gid: &str,
    calls: &[usize],
    collapsed: bool,
    filter_revealed: bool,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let rollup = Rollup::of_kept_run(
        &this.items,
        calls.iter().copied(),
        &this.live_units,
        |item| filter_revealed || this.filter_matches.matches(item),
    );
    // The title is the group's own identity, not a preview of folded content, so
    // it shows in both states. It names each *category* the group holds rather
    // than a bare call count: a run's members are mixed in practice, and "5 tool
    // calls" says nothing about what happened. What it counts is the part of the
    // group's calls the display filter keeps.
    let header =
        FoldHeader::with_title(group_category_title(this, calls, filter_revealed, cx)).trailing(
            fold_group_status_icon(rollup_glyph(rollup, t, this.dim_amount, cx)),
        );
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
            &this.items,
            &this.live_units,
            &this.filter_matches,
            row.filter_revealed,
            this.turn_boundary,
            RenderAssets::of(&this.assets),
            &this.fold,
            &this.activity.tool_started_at,
            this.tail_calls.value(),
            t,
            this.dim_amount,
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
    items: &[ChatItem],
    live_units: &LiveSubagentUnits,
    filter_matches: &FilterMatchIndex,
    filter_revealed: bool,
    boundary: TurnBoundary,
    assets: RenderAssets<'_>,
    fold: &FoldState,
    tool_started_at: &std::collections::HashMap<String, std::time::Instant>,
    // The recent-steps axis's *call* level — the only one that reaches a
    // rendered item, through the subagent card's own boundary. A response's
    // step level is the row projection's business.
    call_window: TailWindow,
    t: &theme::DarudaTheme,
    dim: f32,
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
        // Prose reaching here is never the conclusion (that is
        // `RowKind::ConclusionItem`), and its response bar already labels the
        // speaker, so it renders inline with no per-block header or fold.
        ChatItem::AssistantText { text, .. } => assistant_markdown(ix, text, markdown, cx),
        ChatItem::Thinking { text, .. } => {
            let key = FoldKey::Thinking(ix);
            let expanded = fold.is_expanded(&key, fold_context_at(&key, ix, items, boundary));
            thinking_block(ix, key, expanded, text, markdown, cx).into_any_element()
        }
        ChatItem::ToolCall(tc) => tool_card(
            ix,
            tc,
            0,
            CardContext {
                items,
                live_units,
                filter_matches,
                filter_revealed,
                boundary,
                assets,
                fold,
                tool_started_at,
                call_window,
                t,
                dim,
                pane_id,
                window_handle,
            },
            window,
            cx,
        )
        .into_any_element(),
        ChatItem::Permission(card) => permission_card(ix, card, t, dim, cx).into_any_element(),
        ChatItem::Failure(failure) => {
            failure_block(ix, failure, pane_id, window_handle, t, cx).into_any_element()
        }
        // Owns a top-level row (`RowKind::Interrupted`), so it never reaches
        // the per-item dispatch.
        ChatItem::Interrupted => gpui::Empty.into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bar names its calls, so for the two categories no ACP kind can name
    /// its glyph has to be the one those cards show. Scoped to those two: a
    /// category the kind names is a coarser question than a card's own kind
    /// (Edit covers Move), so the two are not the same glyph by design.
    #[test]
    fn a_name_keyed_category_shows_the_glyph_its_cards_do() {
        use crate::transcript::tool_category::classify_tool;
        use crate::ui::IconNamed as _;
        use daruda_acp::{ToolCallItem, ToolKindView, ToolStatusView};

        let call = |tool_name: Option<&str>, raw_input| ToolCallItem {
            id: "t".into(),
            title: "t".into(),
            kind: ToolKindView::Other,
            tool_name: tool_name.map(str::to_owned),
            status: ToolStatusView::Completed,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input,
            locations: Vec::new(),
            parent_tool_id: None,
            exit: None,
        };
        for tc in [
            call(Some("mcp__obsidian__obsidian_put_content"), None),
            call(
                None,
                Some(serde_json::json!({ "subagent_type": "code-reviewer" })),
            ),
        ] {
            assert_eq!(
                category_icon(classify_tool(&tc)),
                tool::tool_icon(&tc),
                "{:?}",
                tc.tool_name
            );
        }

        // The one place the two part: a reported diff is what the call *did*,
        // which is what a bar tallies, while the card still names which tool it
        // was. Pinned so the split stays a decision rather than a drift.
        let mut edited = call(Some("mcp__obsidian__obsidian_put_content"), None);
        edited.diffs.push(daruda_acp::DiffView {
            path: "/tmp/x".into(),
            old_text: None,
            new_text: "changed".into(),
        });
        assert_eq!(
            category_icon(classify_tool(&edited)),
            tool::tool_kind_icon(ToolKindView::Edit),
            "the bar tallies the edit"
        );
        assert_eq!(
            tool::tool_icon(&edited),
            IconName::ExternalLink.path(),
            "the card still names the tool"
        );
    }

    /// Covers both consumers — the working indicator's run timer and a tool
    /// call's age — since they call this one function.
    #[test]
    fn format_elapsed_cases() {
        for (secs, expected) in [
            (0, "0s"),
            (5, "5s"),
            (60, "1m00s"),
            (65, "1m05s"),
            (600, "10m00s"),
        ] {
            assert_eq!(
                format_elapsed(std::time::Duration::from_secs(secs)),
                expected
            );
        }
    }

    /// The unit travels with the locale rather than being spelled into the
    /// number. Asserted per locale through `t!`'s explicit-locale form: the
    /// ambient locale is process-global, so setting it here would leak into
    /// every other test running beside this one.
    #[test]
    fn elapsed_units_are_localized() {
        for (locale, under, over) in [("en", "5s", "1m05s"), ("ko", "5초", "1분 05초")] {
            assert_eq!(
                rust_i18n::t!("agent_chat.elapsed_seconds", secs = 5, locale = locale),
                under
            );
            // The seconds arrive already padded — the badge ticks, so its width
            // must not move as the digit rolls over.
            assert_eq!(
                rust_i18n::t!(
                    "agent_chat.elapsed_minutes",
                    mins = 1,
                    secs = "05",
                    locale = locale
                ),
                over
            );
        }
    }

    /// The badge sits in a fixed slot beside the chevron, so the count is
    /// abbreviated rather than printed in full.
    #[test]
    fn token_counts_abbreviate_at_each_magnitude() {
        for (n, expected) in [
            (0, "0"),
            (842, "842"),
            (999, "999"),
            (1_000, "1.0k"),
            (14_313, "14.3k"),
            (1_602_356, "1.6M"),
            (999_999, "1.0M"),
            (999_999_999, "1.0B"),
        ] {
            assert_eq!(abbreviate_tokens(n), expected);
        }
    }

    /// Absent facts drop out rather than rendering as a placeholder: a restored
    /// transcript carries no record, and an agent may report no usage at all.
    #[test]
    fn a_turn_without_usage_omits_only_that_fact() {
        let at = chrono::Local::now();
        let with = TurnRecord {
            worked_for: std::time::Duration::from_secs(3),
            finished_at: at,
            output_tokens: Some(1_450),
        };
        let without = TurnRecord {
            output_tokens: None,
            ..with
        };
        let labelled = turn_stats_label(&with);
        let bare = turn_stats_label(&without);
        assert!(labelled.contains("1.4k"), "{labelled}");
        assert!(!bare.contains("1.4k"), "{bare}");
        assert!(
            bare.len() < labelled.len() && labelled.starts_with(&bare),
            "the surviving facts keep their order and spelling: {bare} / {labelled}"
        );
    }

    /// The bar times the turn only while it is the live one. A settled turn's
    /// length lives on its answer row, and an earlier turn is never live.
    #[test]
    fn only_the_live_last_run_carries_an_elapsed_on_its_bar() {
        let busy = Some(std::time::Duration::from_secs(9));
        assert_eq!(running_elapsed(true, busy), busy, "the live last run");
        assert_eq!(running_elapsed(true, None), None, "settled: the row has it");
        assert_eq!(running_elapsed(false, busy), None, "an earlier turn");
        assert_eq!(running_elapsed(false, None), None);
    }

    fn row(kind: RowKind, hidden: bool) -> RenderRow {
        RenderRow::at(kind, hidden, 0)
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
