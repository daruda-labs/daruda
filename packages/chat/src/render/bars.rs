//! Fold-header bars: the response bar, tool-group and thinking-group bars, and
//! the trailing turn facts and category segments they carry.

use super::*;

/// Collapsible header for an agent response (agent items under a user message).
/// The whole row toggles `FoldKey::Response`; the agent label leads, the
/// response's first line fills the row when collapsed, and the tool count plus
/// the status-rollup glyph sit at the right edge.
#[allow(clippy::too_many_arguments)]
pub(super) fn response_bar(
    this: &AgentChatView,
    run_start: usize,
    categories: &[(daruda_transcript::tool_category::ToolCategory, usize)],
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
        FoldHeader::with_title(category_segments(this, categories, 0, cx))
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
            s::agent_chat::turn_running(format_elapsed(elapsed)),
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
pub(super) fn running_elapsed(
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
pub(super) fn turn_stats_element(
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
pub(super) fn turn_stats_label(record: &TurnRecord) -> String {
    let mut parts = vec![s::agent_chat::turn_worked(format_elapsed(
        record.worked_for,
    ))];
    // Through `timestamp`, not assembled here: that module is where a wall
    // clock becomes text, and it is what keeps ko on a 24-hour clock in this
    // row as it is everywhere else.
    parts.push(crate::stamp::local_time(record.finished_at));
    if let Some(out) = record.output_tokens {
        parts.push(s::agent_chat::turn_output(abbreviate_tokens(out)));
    }
    parts.join(&s::agent_chat::turn_separator())
}

/// Token counts shortened for a trailing badge: `842`, `1.5k`, `1.6M`. The row
/// is a fixed slot beside the chevron, so the full figure would push the header
/// off the line.
pub(super) fn abbreviate_tokens(n: u64) -> String {
    // Branch on the *rounded* magnitude: 999_999 is under a million but rounds
    // to "1000.0k", six characters in a slot sized for four.
    match n {
        n if n < 1_000 => n.to_string(),
        n if n < 999_950 => format!("{:.1}k", n as f64 / 1_000.0),
        n if n < 999_950_000 => format!("{:.1}M", n as f64 / 1_000_000.0),
        n => format!("{:.1}B", n as f64 / 1_000_000_000.0),
    }
}

/// A tool group's title: its category segments, most-numerous first, then a
/// thought segment so a thoughts-only filter reads as what it shows. Overflow
/// is clipped by layout — see [`category_segments`] — never dropped silently.
fn group_category_title(
    this: &AgentChatView,
    calls: &[usize],
    thoughts: &[usize],
    filter_revealed: bool,
    cx: &Context<AgentChatView>,
) -> AnyElement {
    let tally = daruda_transcript::tool_category::tally_categories(kept_tool_calls(
        this,
        calls.iter().copied(),
        filter_revealed,
    ));
    let thoughts = kept_thoughts(this, thoughts.iter().copied(), filter_revealed);
    category_segments(this, &tally, thoughts, cx)
}

/// A tally rendered as one icon-and-count segment per category, separated and
/// most-numerous first, then a thought segment when `thoughts` is non-zero.
/// Whether each count also carries its words is the pane's
/// [`ToolSummaryStyle`](crate::view::ToolSummaryStyle).
/// Shared by the two bars that carry one — they differ in *what* they count (a
/// group's own calls, filter-aware; a turn's top-level calls, filter-blind) but
/// not in how it reads.
fn category_segments(
    this: &AgentChatView,
    tally: &[(daruda_transcript::tool_category::ToolCategory, usize)],
    thoughts: usize,
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
    let style = this.defaults.tool_summary;
    let segments = tally
        .iter()
        .map(|&(category, count)| {
            (
                category_icon(category),
                style.segment(count, |n| {
                    s::agent_chat::group_category(category.token(), n)
                }),
            )
        })
        .chain((thoughts > 0).then(|| {
            (
                thought_icon(),
                style.segment(thoughts, s::agent_chat::thinking_group_count),
            )
        }));
    for (ix, (icon, label)) in segments.enumerate() {
        if ix > 0 {
            row = row.child(
                div()
                    .flex_none()
                    .text_color(this.dim(theme::agent_chat_fg_subtle(cx)))
                    .text_size(font_size)
                    .child(SharedString::from(s::agent_chat::group_separator())),
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
                .child(Icon::empty().path(icon).xsmall().text_color(fg))
                .child(SharedString::from(label)),
        );
    }
    div()
        .w_full()
        .min_w_0()
        .pr(px(theme::AGENT_CHAT_TRAILING_GAP))
        .child(row)
        .into_any_element()
}

/// The glyph a thought carries wherever it stands for itself — a group bar's
/// count, a thought row inside a group. The one a `Think` tool call wears, so
/// reasoning reads the same whichever channel delivered it.
pub(super) fn thought_icon() -> SharedString {
    tool::tool_kind_icon(daruda_acp::ToolKindView::Think)
}

/// The glyph for one tool category. A category an ACP kind can name borrows the
/// per-call mapping through a representative kind; the two no kind can name go
/// through [`tool::name_keyed_icon`], the same source their cards use.
pub(super) fn category_icon(
    category: daruda_transcript::tool_category::ToolCategory,
) -> SharedString {
    use daruda_transcript::tool_category::ToolCategory;
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
    indices: impl Iterator<Item = usize>,
    filter_revealed: bool,
) -> usize {
    indices.filter(|&k| {
        matches!(this.items.get(k), Some(item @ ChatItem::Thinking { .. }) if filter_revealed || this.filter_matches.matches(item))
    })
    .count()
}

/// Collapsible header for a consecutive tool-call group. The whole row toggles
/// the group's fold (`FoldKey::ToolGroup`); shows a chevron, one segment per
/// category the group holds, a thought count, and a status-rollup glyph.
#[allow(clippy::too_many_arguments)]
pub(super) fn tool_group_bar(
    this: &AgentChatView,
    gid: &str,
    calls: &[usize],
    thoughts: &[usize],
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
    // group's calls and thoughts the display filter keeps.
    let header = FoldHeader::with_title(group_category_title(
        this,
        calls,
        thoughts,
        filter_revealed,
        cx,
    ))
    .trailing(fold_group_status_icon(rollup_glyph(
        rollup,
        t,
        this.dim_amount,
        cx,
    )));
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
pub(super) fn thinking_group_bar(
    this: &AgentChatView,
    first_ix: usize,
    run: std::ops::Range<usize>,
    collapsed: bool,
    filter_revealed: bool,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    // The count is the group's own identity, so it shows in both fold states —
    // and what it names is the part of the run the display filter keeps.
    let label = s::agent_chat::thinking_group_count(kept_thoughts(this, run, filter_revealed));
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
