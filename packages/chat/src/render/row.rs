//! Per-row element builders: one projected row, and the conversation item it
//! stands for.

use super::bars::{
    response_bar, thinking_group_bar, thought_icon, tool_group_bar, turn_stats_element,
};
use super::*;

/// Render one projected row: an item, a synthetic fold header, or a zero-height
/// `Empty` when collapsed under an ancestor fold (the row stays in the sequence
/// so the count is fold-stable). Applies per-row padding (`PAD_Y` on the first
/// and last visible rows, `LIST_GAP` between) and a left indent per level.
pub(super) fn render_row(
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
        RowKind::GroupThought(i) => match this.items.get(*i) {
            Some(ChatItem::Thinking { text, .. }) => group_thought_block(
                *i,
                text,
                thought_icon(),
                MarkdownRender::new(
                    &this.assets.mermaid_images,
                    this.dim_amount,
                    AgentChatMarkdownLinks::new(cx.weak_entity()),
                ),
                cx,
            ),
            _ => gpui::Empty.into_any_element(),
        },
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
            thoughts,
            collapsed,
        } => tool_group_bar(
            this,
            gid,
            calls,
            thoughts,
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
                    fold_context_at(&key, *i, &this.items, this.turn_boundary, &this.live_units),
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
                        AgentChatMarkdownLinks::new(cx.weak_entity()),
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
            &this.command_analysis,
            row.filter_revealed,
            this.turn_boundary,
            RenderAssets::of(&this.assets),
            &this.fold,
            &this.activity.tool_started_at,
            this.tail_calls.value(),
            t,
            this.dim_amount,
            this.preferred_editor(),
            window,
            cx,
        ),
        None => gpui::Empty.into_any_element(),
    }
}

/// One conversation row. Message bodies render as selectable markdown via
/// `daruda_ui::markdown`, keyed by `ix` for stable selection identity. `fold` /
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
    command_analysis: &daruda_transcript::command_analysis::CommandAnalysisIndex,
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
    preferred_editor: &str,
    window: &mut Window,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let mermaid_images = assets.mermaid_images;
    let markdown = MarkdownRender::new(
        mermaid_images,
        dim,
        AgentChatMarkdownLinks::new(cx.weak_entity()),
    );
    match item {
        ChatItem::UserText(text) => user_bubble(ix, text, dim, cx).into_any_element(),
        // Prose reaching here is never the conclusion (that is
        // `RowKind::ConclusionItem`), and its response bar already labels the
        // speaker, so it renders inline with no per-block header or fold.
        ChatItem::AssistantText { text, .. } => assistant_markdown(ix, text, markdown, cx),
        ChatItem::Thinking { text, .. } => {
            let key = FoldKey::Thinking(ix);
            let expanded =
                fold.is_expanded(&key, fold_context_at(&key, ix, items, boundary, live_units));
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
                command_analysis,
                filter_revealed,
                boundary,
                assets,
                fold,
                tool_started_at,
                call_window,
                t,
                dim,
                preferred_editor,
            },
            window,
            cx,
        )
        .into_any_element(),
        ChatItem::Permission(card) => permission_card(ix, card, t, dim, cx).into_any_element(),
        ChatItem::Failure(failure) => failure_block(ix, failure, t, cx).into_any_element(),
        // Owns a top-level row (`RowKind::Interrupted`), so it never reaches
        // the per-item dispatch.
        ChatItem::Interrupted => gpui::Empty.into_any_element(),
    }
}
