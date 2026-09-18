//! Shared fixtures for the row-projection tests, and the modules that use
//! them. Split by the question each group asks; every helper lives here so
//! the modules cannot drift onto private copies of the same fixture.

use super::*;
use crate::transcript::display_filter::{DisplayFilter, FilterFacet};
use crate::transcript::fold_mode::FoldPreset;
use crate::workspace::main_area::agent_chat_pane::fold::FoldContext;
use crate::workspace::main_area::agent_chat_pane::rows::tail::TailWindow;
use crate::workspace::main_area::agent_chat_pane::tool_hierarchy::SUBAGENT_NEST_DEPTH_CAP;
use daruda_acp::{
    PermissionItem, PermissionResolution, ToolCallItem, ToolKindView, ToolStatusView,
};

mod delegation;
mod filter;
mod fold;
mod group;
mod row;
mod windows;

fn tool(id: &str, status: ToolStatusView) -> ChatItem {
    ChatItem::ToolCall(ToolCallItem {
        id: id.to_owned(),
        title: format!("Tool {id}"),
        kind: ToolKindView::Edit,
        tool_name: None,
        status,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: None,
        locations: Vec::new(),
        parent_tool_id: None,
        exit: None,
    })
}

fn perm(resolved: bool) -> ChatItem {
    ChatItem::Permission(PermissionItem {
        id: 0,
        tool_title: Some("Write /tmp/x".to_owned()),
        raw_input_summary: None,
        options: Vec::new(),
        resolved: resolved.then_some(PermissionResolution::Cancelled),
    })
}

fn asst(s: &str) -> ChatItem {
    ChatItem::AssistantText {
        text: s.to_owned(),
        streaming: false,
        message_id: None,
        phase: Default::default(),
    }
}

fn kinds(rows: &[RenderRow]) -> Vec<(&'static str, bool)> {
    rows.iter()
        .map(|r| {
            let k = match r.kind {
                RowKind::User(_) => "user",
                RowKind::Interrupted(_) => "interrupted",
                RowKind::ResponseHeader { .. } => "response",
                RowKind::AgentItem(_) | RowKind::ConclusionItem(_) => "item",
                RowKind::TailMore { .. } => "tail",
                RowKind::ToolGroupTailMore { .. } => "grouptail",
                RowKind::ToolGroupHeader { .. } => "group",
                RowKind::ThinkingGroupHeader { .. } => "thinkgroup",
                RowKind::WorkingIndicator => "working",
            };
            (k, r.hidden)
        })
        .collect()
}

fn two_settled_turns() -> [ChatItem; 8] {
    use ToolStatusView::Completed;
    [
        ChatItem::UserText("first".into()),
        asst("a1"),
        tool("t1", Completed),
        tool("t2", Completed),
        ChatItem::UserText("second".into()),
        asst("a2"),
        tool("t3", Completed),
        tool("t4", Completed),
    ]
}

fn project_under(items: &[ChatItem], fold: &FoldState) -> Vec<RenderRow> {
    project(
        items,
        fold,
        false,
        &LiveSubagentUnits::of(items),
        StepWindow::uniform(TailWindow::All),
        &DisplayFilter::default(),
    )
}

fn tool_of(items: &[ChatItem], id: &str) -> ToolCallItem {
    items
        .iter()
        .find_map(|it| match it {
            ChatItem::ToolCall(tc) if tc.id == id => Some(tc.clone()),
            _ => None,
        })
        .expect("tool call present")
}

fn child_of(id: &str, parent: &str, status: ToolStatusView) -> ChatItem {
    let mut c = tool(id, status);
    if let ChatItem::ToolCall(tc) = &mut c {
        tc.parent_tool_id = Some(parent.to_owned());
    }
    c
}

fn one_turn_of_cycles(cycles: usize) -> Vec<ChatItem> {
    use ToolStatusView::Completed;
    let mut items = vec![ChatItem::UserText("q".into())];
    for c in 0..cycles {
        items.push(think(&format!("why {c}")));
        items.push(think(&format!("also {c}")));
        items.push(asst(&format!("plan {c}")));
        for t in 0..4 {
            items.push(tool(&format!("t{c}-{t}"), Completed));
        }
    }
    items.push(asst("done"));
    items
}

fn project_all(items: &[ChatItem]) -> Vec<RenderRow> {
    project(
        items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(items),
        StepWindow::uniform(TailWindow::All),
        &DisplayFilter::default(),
    )
}

// ── Tool runs ──────────────────────────────────────────────────────────────
//
fn think(s: &str) -> ChatItem {
    ChatItem::Thinking {
        text: s.to_owned(),
        streaming: false,
        message_id: None,
    }
}

// ── Tail window ────────────────────────────────────────────────────────────

/// One prose row plus a two-call run per cycle, so every cycle contributes one
/// tool run — the population the tail window counts.
fn turn_of_cycles(cycles: usize) -> Vec<ChatItem> {
    let mut items = vec![ChatItem::UserText("q".into())];
    for i in 0..cycles {
        items.push(asst(&format!("cycle {i}")));
        items.push(tool(&format!("t{i}"), ToolStatusView::Completed));
        items.push(tool(&format!("t{i}b"), ToolStatusView::Completed));
    }
    items.push(asst("done"));
    items
}

fn project_tail(items: &[ChatItem], tail: StepWindow) -> Vec<RenderRow> {
    project(
        items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(items),
        tail,
        &DisplayFilter::default(),
    )
}

fn group_visibility(rows: &[RenderRow]) -> Vec<bool> {
    rows.iter()
        .filter(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
        .map(|r| !r.hidden)
        .collect()
}

fn tail_row(rows: &[RenderRow]) -> &RenderRow {
    rows.iter()
        .find(|r| matches!(r.kind, RowKind::TailMore { .. }))
        .expect("a run with tool calls gets a tail row")
}

/// The group's calls only earn rows once the group is open, which is what makes
/// the in-group window observable at all.
/// A turn projected with a window on the *call* level only, the response's own
/// steps left whole — so whatever a group trims is attributable to that level
/// alone.
fn project_open_group_calls(items: &[ChatItem], calls: TailWindow) -> Vec<RenderRow> {
    project_open_group(
        items,
        StepWindow {
            steps: TailWindow::All,
            calls,
        },
    )
}

fn project_open_group(items: &[ChatItem], tail: StepWindow) -> Vec<RenderRow> {
    project_open_group_under(
        items,
        &FoldState::with_mode(FoldPreset::Expanded.mode()),
        tail,
    )
}

fn project_open_group_under(
    items: &[ChatItem],
    fold: &FoldState,
    tail: StepWindow,
) -> Vec<RenderRow> {
    project(
        items,
        fold,
        false,
        &LiveSubagentUnits::of(items),
        tail,
        &DisplayFilter::default(),
    )
}

fn group_tail_row(rows: &[RenderRow]) -> &RenderRow {
    rows.iter()
        .find(|r| matches!(r.kind, RowKind::ToolGroupTailMore { .. }))
        .expect("a tool group gets a boundary of its own")
}

fn group_tail_counts(rows: &[RenderRow]) -> (usize, usize) {
    match &group_tail_row(rows).kind {
        RowKind::ToolGroupTailMore {
            hidden_calls,
            kept_calls,
            ..
        } => (*hidden_calls, *kept_calls),
        _ => unreachable!(),
    }
}

/// Visibility of each of the group's calls, in transcript order.
fn call_visibility(items: &[ChatItem], rows: &[RenderRow]) -> Vec<bool> {
    rows.iter()
        .filter(
            |r| matches!(r.kind, RowKind::AgentItem(ix) if matches!(items[ix], ChatItem::ToolCall(_))),
        )
        .map(|r| !r.hidden)
        .collect()
}

/// Row identity and visibility together — what the two windows decide between
/// them.
fn marks(rows: &[RenderRow]) -> Vec<(&'static str, bool)> {
    rows.iter()
        .map(|r| {
            let kind = match r.kind {
                RowKind::User(_) => "user",
                RowKind::Interrupted(_) => "interrupted",
                RowKind::ResponseHeader { .. } => "response",
                RowKind::AgentItem(_) | RowKind::ConclusionItem(_) => "item",
                RowKind::TailMore { .. } => "tail",
                RowKind::ToolGroupTailMore { .. } => "grouptail",
                RowKind::ToolGroupHeader { .. } => "group",
                RowKind::ThinkingGroupHeader { .. } => "thinkgroup",
                RowKind::WorkingIndicator => "working",
            };
            (kind, r.hidden)
        })
        .collect()
}

/// The response boundary's `(hidden, kept)` pair, read off the row that carries
/// it — the step-level twin of [`group_tail_counts`].
fn tail_counts(rows: &[RenderRow]) -> (usize, usize) {
    match tail_row(rows).kind {
        RowKind::TailMore {
            hidden_steps,
            kept_steps,
            ..
        } => (hidden_steps, kept_steps),
        _ => unreachable!(),
    }
}

// ── Display filter ─────────────────────────────────────────────────────────

fn project_filtered(items: &[ChatItem], filter: &DisplayFilter) -> Vec<RenderRow> {
    project(
        items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(items),
        StepWindow::uniform(TailWindow::All),
        filter,
    )
}

/// The run's tally, read off the bar that carries it.
fn filtered_away(rows: &[RenderRow]) -> FilteredAway {
    rows.iter()
        .find_map(|r| match r.kind {
            RowKind::ResponseHeader { filtered, .. } => Some(filtered),
            _ => None,
        })
        .expect("an answered run has a bar")
}

fn filtered_count(rows: &[RenderRow]) -> usize {
    filtered_away(rows).revealable
}

fn one_run_turn() -> Vec<ChatItem> {
    vec![
        ChatItem::UserText("q".into()),
        asst("looking"),
        tool("a", ToolStatusView::Completed),
        tool("b", ToolStatusView::Completed),
        asst("done"),
    ]
}

fn live_run_turn() -> Vec<ChatItem> {
    let mut items = one_run_turn();
    items[3] = tool("b", ToolStatusView::InProgress);
    items
}

fn only_reads() -> DisplayFilter {
    DisplayFilter::from_tokens(["tool_read"])
}

/// A launch the way the adapter builds one — `subagent_type` on its input is
/// what makes it structure rather than one more Edit-kind call.
fn subagent_launch(id: &str, status: ToolStatusView) -> ChatItem {
    let mut launch = tool(id, status);
    if let ChatItem::ToolCall(tc) = &mut launch {
        tc.tool_name = Some("Task".into());
        tc.kind = ToolKindView::Think;
        tc.raw_input = Some(serde_json::json!({ "subagent_type": "general-purpose" }));
    }
    launch
}

/// One turn that delegates: a subagent launch followed by the children the
/// adapter flattened under it, none of which earns a row.
fn turn_with_subagent(children: usize, running: bool) -> Vec<ChatItem> {
    let mut items = vec![ChatItem::UserText("q".into()), asst("delegating")];
    items.push(subagent_launch("task", ToolStatusView::Completed));
    for i in 0..children {
        let last = i + 1 == children;
        let status = if last && running {
            ToolStatusView::InProgress
        } else {
            ToolStatusView::Completed
        };
        items.push(child_of(&format!("c{i}"), "task", status));
    }
    items.push(asst("done"));
    items
}

/// A read-kind call, so a filter aimed at edits leaves it on screen.
fn read_tool(id: &str, status: ToolStatusView) -> ChatItem {
    let mut item = tool(id, status);
    if let ChatItem::ToolCall(tc) = &mut item {
        tc.kind = ToolKindView::Read;
    }
    item
}

/// One run: prose, then a single group of `calls` edit calls, the last of which
/// is still running when `running`.
fn turn_of_one_group(calls: usize, running: bool) -> Vec<ChatItem> {
    let mut items = vec![ChatItem::UserText("q".into()), asst("looking")];
    for i in 0..calls {
        let last = i + 1 == calls;
        let status = if last && running {
            ToolStatusView::InProgress
        } else {
            ToolStatusView::Completed
        };
        items.push(tool(&format!("g{i}"), status));
    }
    items
}

fn hides_edits() -> DisplayFilter {
    DisplayFilter::default().toggled(FilterFacet::ToolEdit)
}

fn kinded_tool(id: &str, kind: ToolKindView, status: ToolStatusView) -> ChatItem {
    ChatItem::ToolCall(ToolCallItem {
        id: id.to_owned(),
        title: format!("Tool {id}"),
        kind,
        tool_name: None,
        status,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: None,
        locations: Vec::new(),
        parent_tool_id: None,
        exit: None,
    })
}

fn preamble(text: &str) -> ChatItem {
    ChatItem::AssistantText {
        text: text.to_owned(),
        streaming: false,
        message_id: None,
        phase: daruda_acp::MessagePhase::Commentary,
    }
}

// ── Thinking groups ─────────────────────────────────────────────────────────

fn think_group_headers(rows: &[RenderRow]) -> Vec<&RenderRow> {
    rows.iter()
        .filter(|r| matches!(r.kind, RowKind::ThinkingGroupHeader { .. }))
        .collect()
}

/// Every thinking group's `(first_ix, count)`, in projection order.
fn think_group_spans(rows: &[RenderRow]) -> Vec<(usize, usize)> {
    rows.iter()
        .filter_map(|r| match r.kind {
            RowKind::ThinkingGroupHeader {
                first_ix, count, ..
            } => Some((first_ix, count)),
            _ => None,
        })
        .collect()
}

/// A subagent whose calls span two categories and two levels: `c1` holds the
/// only Read under it, so a Read filter rescues `c1` as an ancestor while
/// cutting its Edit sibling — the shape that tells "walk through a kept child"
/// apart from "stop at a rejected one".
fn turn_with_nested_subagent() -> Vec<ChatItem> {
    let kinded = |id: &str, parent: &str, kind| {
        let mut c = child_of(id, parent, ToolStatusView::Completed);
        if let ChatItem::ToolCall(tc) = &mut c {
            tc.kind = kind;
        }
        c
    };
    vec![
        ChatItem::UserText("q".into()),
        asst("delegating"),
        subagent_launch("task", ToolStatusView::Completed),
        kinded("c0", "task", ToolKindView::Read),
        kinded("c1", "task", ToolKindView::Edit),
        kinded("g0", "c1", ToolKindView::Read),
        kinded("g1", "c1", ToolKindView::Edit),
        kinded("c2", "task", ToolKindView::Edit),
        asst("done"),
    ]
}
