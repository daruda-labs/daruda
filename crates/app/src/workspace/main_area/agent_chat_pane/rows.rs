//! Render-row projection over the flat `items` model.
//!
//! The conversation model (`AgentChatView.items`) stays a flat, append-only
//! `Vec<ChatItem>` (the ACP / zed standard). Folding structure — turns, agent
//! responses, consecutive tool-call groups — is *derived at render time* by
//! [`project`], never baked into the model. The result is a stable sequence of
//! [`RenderRow`]s the virtualized `list` indexes over: synthetic header rows
//! (`ResponseHeader`, `ToolGroupHeader`) interleaved with the item rows, each
//! carrying a `hidden` flag (collapsed under an ancestor → rendered as a
//! zero-height `Empty`) and a nesting `indent` depth.
//!
//! Folding never *removes* rows — it flips `hidden` — so the row count is
//! stable across a fold toggle (only heights change → `remeasure`), and only an
//! `items` append (or a run becoming a group) changes the count. `same_slot`
//! lets `rebuild_rows` find the longest unchanged prefix and splice only the
//! tail, preserving scroll.

use daruda_acp::{ChatItem, ToolCallItem};

use super::agent_chat_helpers::fold_active;
use super::fold::{FoldKey, FoldState};

/// Minimum consecutive tool-call run length that earns a collapsible group. A
/// lone tool call renders as its own (block-foldable) card — wrapping one tool
/// in a "1 tool call" group bar is noise.
const TOOL_GROUP_MIN: usize = 2;

/// What a projected row represents. The payload is the row's stable *slot key*
/// (item index / tool-group id) plus cached display data; `same_slot` compares
/// only the key.
pub(in crate::workspace) enum RowKind {
    /// A user message — the always-visible turn anchor (never folded).
    User(usize),
    /// Synthetic header for an agent response (the run of agent items after a
    /// user message), keyed by the anchoring `UserText` index. Emitted only for
    /// a non-trivial response (≥ 1 tool or ≥ 2 blocks); `collapsed` hides the
    /// whole response and drives the header chevron / summary.
    ResponseHeader { anchor: usize, collapsed: bool },
    /// An individual agent block (assistant / thinking / tool / permission /
    /// error), keyed by its `items` index.
    AgentItem(usize),
    /// Synthetic header for a consecutive run of ≥ [`TOOL_GROUP_MIN`] tool
    /// calls. `gid` is the first tool call's id (stable under append-only);
    /// `first_ix` + `count` locate the run for the summary; `collapsed` drives
    /// the header chevron + whether the summary line is shown.
    ToolGroupHeader {
        gid: String,
        first_ix: usize,
        count: usize,
        collapsed: bool,
    },
    /// The turn's conclusion — the run's final assistant-text block, when it
    /// sits under a response bar. Keyed by its `items` index. Unlike the inline
    /// process prose (a plain `AgentItem` at indent > 0, hidden when the
    /// response folds), the conclusion stays visible when the response is
    /// collapsed *and* carries its own fold toggle (`FoldKey::Assistant`), so it
    /// can be collapsed to a one-line summary independently of the process.
    ConclusionItem(usize),
    /// Synthetic "agent is working" indicator pinned at the tail of the last
    /// turn's run for the whole time a turn is in flight (through tool execution
    /// and streaming), so the live "working … + elapsed" signal stays
    /// consistently present in the conversation flow. Carries no payload; at
    /// most one exists (always the last row). Suppressed only while blocked on a
    /// permission prompt (via `awaiting_response`); it stays visible even when
    /// the response is manually collapsed — pinned below the still-visible
    /// conclusion — so a folded in-flight turn still shows the live signal.
    WorkingIndicator,
}

/// One row in the projected, renderable sequence.
pub(in crate::workspace) struct RenderRow {
    pub(in crate::workspace) kind: RowKind,
    /// Collapsed under an ancestor fold → render as a zero-height `Empty` (the
    /// row stays in the sequence so the count is fold-stable).
    pub(in crate::workspace) hidden: bool,
    /// Nesting depth (left indent): 0 = top level (user / response header),
    /// 1 = inside a response, 2 = inside a tool group within a response.
    pub(in crate::workspace) indent: u8,
}

impl RenderRow {
    /// Whether two rows occupy the same logical slot — same kind + same key,
    /// ignoring `hidden` / cached display fields. Drives `rebuild_rows`'
    /// longest-common-prefix diff so a fold toggle (same slots, only `hidden`
    /// changed) splices nothing and a streamed append splices only the tail.
    pub(in crate::workspace) fn same_slot(&self, other: &Self) -> bool {
        match (&self.kind, &other.kind) {
            (RowKind::User(a), RowKind::User(b))
            | (RowKind::AgentItem(a), RowKind::AgentItem(b))
            | (
                RowKind::ResponseHeader { anchor: a, .. },
                RowKind::ResponseHeader { anchor: b, .. },
            ) => a == b,
            (RowKind::ToolGroupHeader { gid: a, .. }, RowKind::ToolGroupHeader { gid: b, .. }) => {
                a == b
            }
            (RowKind::ConclusionItem(a), RowKind::ConclusionItem(b)) => a == b,
            // At most one indicator exists (always the tail), so any two occupy
            // the same logical slot.
            (RowKind::WorkingIndicator, RowKind::WorkingIndicator) => true,
            _ => false,
        }
    }
}

/// Threshold for a response to earn its own collapsible bar: at least one tool
/// call, or ≥ this many agent blocks. A lone short reply renders inline (no
/// response bar on every trivial answer).
const RESPONSE_MIN_BLOCKS: usize = 2;

/// Project the flat `items` into renderable rows, deciding each row's
/// visibility from `fold`. Structure (derived, not stored):
/// - A **turn** is a `UserText` (the always-visible anchor) plus the run of
///   agent items up to the next `UserText`.
/// - A **non-trivial response** (≥ 1 tool or ≥ [`RESPONSE_MIN_BLOCKS`] blocks)
///   gets a collapsible `ResponseHeader`; collapsing it hides the whole run.
///   A trivial response renders inline (no header, no fold).
/// - Inside a response, a consecutive run of ≥ [`TOOL_GROUP_MIN`] tool calls
///   gets a collapsible `ToolGroupHeader`. Nesting deepens `indent`.
///
/// Defaults via `ExpandedWhileActive`: the last turn / a streaming response is
/// expanded, settled past responses collapse; a settled tool group collapses.
/// Pure and total.
///
/// `awaiting_response` (the view's "the agent is actively working" flag —
/// `activity_state() == Working`, which folds in background-tool activity as
/// well as an in-flight prompt turn) drives a
/// trailing [`RowKind::WorkingIndicator`] pinned to the last turn's tail: it
/// stays for the whole turn — through tool execution and streaming — so the
/// "working … + elapsed" signal is consistently present, not just in the gap
/// between blocks. Suppressed only when blocked on a permission prompt (the
/// card carries that state); it stays visible even when the turn's response is
/// manually collapsed, pinned below the still-visible conclusion.
pub(in crate::workspace) fn project(
    items: &[ChatItem],
    fold: &FoldState,
    awaiting_response: bool,
) -> Vec<RenderRow> {
    let mut rows = Vec::with_capacity(items.len() + 4);
    let mut i = 0;
    while i < items.len() {
        // A turn = optional leading user message + the agent run until the next
        // user message.
        let anchor = match &items[i] {
            ChatItem::UserText(_) => {
                rows.push(RenderRow {
                    kind: RowKind::User(i),
                    hidden: false,
                    indent: 0,
                });
                let a = i;
                i += 1;
                Some(a)
            }
            // Leading agent items with no preceding user message (uncommon):
            // render the run at the top level with no response header.
            _ => None,
        };

        let run_start = i;
        while i < items.len() && !matches!(items[i], ChatItem::UserText(_)) {
            i += 1;
        }
        let run = run_start..i;
        let is_last_turn = i >= items.len();

        let tools = run
            .clone()
            .filter(|&k| matches!(items[k], ChatItem::ToolCall(_)))
            .count();
        let non_trivial = anchor.is_some() && (tools >= 1 || run.len() >= RESPONSE_MIN_BLOCKS);
        // The run's final assistant-text block is the turn's conclusion: it
        // stays visible when the response is collapsed (see `project_run`).
        let conclusion_ix = run
            .clone()
            .rev()
            .find(|&k| matches!(items[k], ChatItem::AssistantText { .. }));

        // Indent of this turn's run, reused to place the trailing working
        // indicator at the same nesting as the run's conclusion.
        let run_indent = if let (true, Some(a)) = (non_trivial, anchor) {
            let collapsed = !fold.is_expanded(
                &FoldKey::Response(a),
                fold_active(&FoldKey::Response(a), items),
            );
            rows.push(RenderRow {
                kind: RowKind::ResponseHeader {
                    anchor: a,
                    collapsed,
                },
                hidden: false,
                indent: 0,
            });
            project_run(
                items,
                fold,
                run.clone(),
                1,
                collapsed,
                conclusion_ix,
                &mut rows,
            );
            1u8
        } else {
            project_run(items, fold, run.clone(), 0, false, conclusion_ix, &mut rows);
            0u8
        };

        // While a turn is in flight (and not blocked on a permission prompt),
        // pin a live "agent is working" indicator to the last turn's tail — for
        // the whole turn, through tool execution and streaming alike — so the
        // "working … + elapsed" signal is consistently present instead of
        // flickering out whenever a block is active. Last turn only; it stays
        // visible even when the response is manually collapsed (pinned below the
        // still-visible conclusion, at the run's indent) so a folded in-flight
        // turn still shows it's working.
        if awaiting_response && is_last_turn {
            rows.push(RenderRow {
                kind: RowKind::WorkingIndicator,
                hidden: false,
                indent: run_indent,
            });
        }
    }
    rows
}

/// Project one agent run (the items of a single response) into rows at
/// `base_indent`, grouping consecutive tool-call runs. `response_collapsed`
/// hides every row in the run (the response is folded); a settled tool group
/// additionally hides its own members.
///
/// `conclusion_ix`, when set, is the run's final assistant-text item — the
/// turn's conclusion. It stays visible even while the response is collapsed, so
/// a folded turn reads as "user question → conclusion" with the intermediate
/// process tucked behind the response bar.
fn project_run(
    items: &[ChatItem],
    fold: &FoldState,
    run: std::ops::Range<usize>,
    base_indent: u8,
    response_collapsed: bool,
    conclusion_ix: Option<usize>,
    rows: &mut Vec<RenderRow>,
) {
    let mut k = run.start;
    while k < run.end {
        // A subagent's inner tool call renders nested inside its parent's card
        // (see `tool_card`), so it earns no row of its own here — skip it. The
        // Claude adapter flattens subagent activity into the one session and
        // links a child to its parent only via `parent_tool_id`.
        if matches!(&items[k], ChatItem::ToolCall(tc) if is_nested_child(items, tc)) {
            k += 1;
            continue;
        }
        if matches!(items[k], ChatItem::ToolCall(_)) {
            let gstart = k;
            k += 1;
            // Extend the group over consecutive *top-level* tool calls; a nested
            // child belongs to the preceding parent, so it ends the run.
            while k < run.end
                && matches!(&items[k], ChatItem::ToolCall(t) if !is_nested_child(items, t))
            {
                k += 1;
            }
            let grun = gstart..k;
            // A tool group with a still-running member (or a live subagent
            // descendant) stays surfaced even when the response is collapsed —
            // its header pops out (still folded) below the conclusion so a
            // folded in-flight turn shows *what* is running; the members stay
            // hidden until the user expands the group.
            let group_live = grun.clone().any(
                |j| matches!(&items[j], ChatItem::ToolCall(tc) if tool_or_subtree_live(items, tc)),
            );
            if grun.len() >= TOOL_GROUP_MIN {
                let gid = tool_id(&items[gstart]);
                let group_key = FoldKey::ToolGroup(gid.clone());
                let group_collapsed = !fold.is_expanded(&group_key, fold_active(&group_key, items));
                rows.push(RenderRow {
                    kind: RowKind::ToolGroupHeader {
                        gid,
                        first_ix: gstart,
                        count: grun.len(),
                        collapsed: group_collapsed,
                    },
                    hidden: response_collapsed && !group_live,
                    indent: base_indent,
                });
                for j in grun {
                    rows.push(RenderRow {
                        kind: RowKind::AgentItem(j),
                        hidden: response_collapsed || group_collapsed,
                        indent: base_indent + 1,
                    });
                }
            } else {
                rows.push(RenderRow {
                    kind: RowKind::AgentItem(gstart),
                    hidden: response_collapsed && !group_live,
                    indent: base_indent,
                });
            }
        } else {
            // Two kinds of block stay visible even when the response is
            // collapsed; every other block hides with the fold:
            //   - the conclusion (run's last assistant text), and
            //   - a still-pending permission request — it is actionable, so
            //     folding away its Allow/Reject buttons would strand the user.
            // The conclusion under a response bar becomes a `ConclusionItem`
            // (its own fold toggle); at the top level a plain assistant block
            // already carries one.
            let is_conclusion = Some(k) == conclusion_ix;
            let pending_permission =
                matches!(&items[k], ChatItem::Permission(c) if c.resolved.is_none());
            let force_visible = is_conclusion || pending_permission;
            let kind = if is_conclusion && base_indent > 0 {
                RowKind::ConclusionItem(k)
            } else {
                RowKind::AgentItem(k)
            };
            rows.push(RenderRow {
                kind,
                hidden: response_collapsed && !force_visible,
                indent: base_indent,
            });
            k += 1;
        }
    }
}

/// Whether `tc` is a subagent's inner call whose parent is present in `items`.
/// Such a child renders nested inside its parent's card (see `tool_card`), so it
/// earns no row of its own. A child whose `parent_tool_id` matches no tool call
/// in `items` (a dangling ref — parent never arrived, or a malformed adapter) is
/// NOT nested by anyone, so it is treated as a top-level call and keeps its row
/// rather than being silently dropped.
fn is_nested_child(items: &[ChatItem], tc: &ToolCallItem) -> bool {
    let Some(pid) = tc.parent_tool_id.as_deref() else {
        return false;
    };
    items
        .iter()
        .any(|it| matches!(it, ChatItem::ToolCall(p) if p.id == pid))
}

/// Recursion cap for walking flattened subagent nesting (`parent_tool_id`
/// links). Real nesting is one or two levels; the cap only fires on a malformed
/// / cyclic id from the adapter, bounding the stack instead of overflowing it.
/// Shared by the render's card nesting and `subagent_subtree_live`.
pub(in crate::workspace) const SUBAGENT_NEST_DEPTH_CAP: usize = 8;

/// Whether a tool call is still working as a unit — its own status is live, or
/// (for a subagent parent the adapter marked `Completed` when its SDK call
/// returned) a flattened descendant is still running. Keeps a running tool /
/// group visible under a collapsed response and its card badge reading as
/// in-progress.
pub(in crate::workspace) fn tool_or_subtree_live(items: &[ChatItem], tc: &ToolCallItem) -> bool {
    tc.status.is_live() || subagent_subtree_live(items, &tc.id, SUBAGENT_NEST_DEPTH_CAP)
}

/// Whether the subagent unit rooted at `parent_id` is still working — some
/// (transitive, depth-bounded) descendant tool call linked via `parent_tool_id`
/// is live. `remaining_depth` bounds the walk exactly like the render's nesting
/// cap, so a malformed / cyclic `parent_tool_id` from the adapter can't recurse
/// without bound. The tool card reads this so a subagent parent the adapter
/// marked `Completed` when its SDK call returned — while the flattened child
/// tool calls stream in and run afterward — still reads as running, not "done".
pub(in crate::workspace) fn subagent_subtree_live(
    items: &[ChatItem],
    parent_id: &str,
    remaining_depth: usize,
) -> bool {
    if remaining_depth == 0 {
        return false;
    }
    items.iter().any(|it| {
        matches!(it,
            ChatItem::ToolCall(c)
                if c.parent_tool_id.as_deref() == Some(parent_id)
                    && (c.status.is_live()
                        || subagent_subtree_live(items, &c.id, remaining_depth - 1)))
    })
}

/// First tool call's id for a `ToolCall` item (the group's stable key); empty
/// for any other item (never called that way).
fn tool_id(item: &ChatItem) -> String {
    match item {
        ChatItem::ToolCall(tc) => tc.id.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_acp::{
        PermissionItem, PermissionResolution, ToolCallItem, ToolKindView, ToolStatusView,
    };

    fn tool(id: &str, status: ToolStatusView) -> ChatItem {
        ChatItem::ToolCall(ToolCallItem {
            id: id.to_owned(),
            title: format!("Tool {id}"),
            kind: ToolKindView::Edit,
            status,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: None,
        })
    }
    /// A permission card — `resolved=false` makes it pending (actionable).
    fn perm(resolved: bool) -> ChatItem {
        ChatItem::Permission(PermissionItem {
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
        }
    }
    fn kinds(rows: &[RenderRow]) -> Vec<(&'static str, bool)> {
        rows.iter()
            .map(|r| {
                let k = match r.kind {
                    RowKind::User(_) => "user",
                    RowKind::ResponseHeader { .. } => "response",
                    // The conclusion is still an item row for visibility tests;
                    // its distinct variant is asserted directly where it matters.
                    RowKind::AgentItem(_) | RowKind::ConclusionItem(_) => "item",
                    RowKind::ToolGroupHeader { .. } => "group",
                    RowKind::WorkingIndicator => "working",
                };
                (k, r.hidden)
            })
            .collect()
    }

    #[test]
    fn turn_with_tools_nests_response_and_group() {
        use ToolStatusView::Completed;
        let items = [
            ChatItem::UserText("q".into()),
            asst("working"),
            tool("a", Completed),
            tool("b", Completed),
            tool("c", Completed),
            asst("done"),
        ];
        let rows = project(&items, &FoldState::default(), false);
        // user anchor, response header (last turn → expanded), then the run at
        // indent 1: assistant, tool-group header, 3 settled tool members
        // (group collapsed → hidden), trailing assistant.
        assert_eq!(
            kinds(&rows),
            vec![
                ("user", false),
                ("response", false),
                ("item", false),
                ("group", false),
                ("item", true),
                ("item", true),
                ("item", true),
                ("item", false),
            ]
        );
        // indents: user 0, response 0, run items 1, group members 2.
        let indents: Vec<u8> = rows.iter().map(|r| r.indent).collect();
        assert_eq!(indents, vec![0, 0, 1, 1, 2, 2, 2, 1]);
    }

    #[test]
    fn trivial_response_has_no_bar() {
        // One short assistant reply, no tools → inline (no response header).
        let items = [ChatItem::UserText("hi".into()), asst("hello")];
        let rows = project(&items, &FoldState::default(), false);
        assert_eq!(kinds(&rows), vec![("user", false), ("item", false)]);
    }

    #[test]
    fn past_turn_collapses_current_expands() {
        use ToolStatusView::Completed;
        let items = [
            ChatItem::UserText("first".into()),
            asst("a1"),
            tool("t1", Completed),
            tool("t2", Completed),
            ChatItem::UserText("second".into()),
            asst("a2"),
            tool("t3", Completed),
            tool("t4", Completed),
        ];
        let rows = project(&items, &FoldState::default(), false);
        // Past response (first turn, settled, not last) collapses → its process
        // hides but its conclusion (a1, the run's last assistant text) stays
        // visible; current response (last turn) expands → its run visible.
        assert_eq!(
            kinds(&rows),
            vec![
                ("user", false),     // first
                ("response", false), // header always shown
                ("item", false),     // a1 = conclusion, stays visible
                ("group", true),
                ("item", true),
                ("item", true),
                ("user", false), // second
                ("response", false),
                ("item", false),  // a2 visible (current expanded)
                ("group", false), // settled group → collapsed members
                ("item", true),
                ("item", true),
            ]
        );
    }

    #[test]
    fn working_indicator_fills_gap_after_tool_group_settles() {
        use ToolStatusView::Completed;
        // A turn whose tools have all settled but the next assistant text has
        // not arrived yet, with a turn still in flight → trailing indicator.
        let items = [
            ChatItem::UserText("q".into()),
            asst("planning"),
            tool("a", Completed),
            tool("b", Completed),
        ];
        let rows = project(&items, &FoldState::default(), true);
        assert_eq!(
            kinds(&rows),
            vec![
                ("user", false),
                ("response", false),
                ("item", false),  // assistant prose
                ("group", false), // tool group header
                ("item", true),   // settled members collapsed
                ("item", true),
                ("working", false), // gap indicator at the run tail
            ]
        );
        // The indicator nests inside the response (indent 1), not at top level.
        assert_eq!(rows.last().unwrap().indent, 1);
    }

    #[test]
    fn working_indicator_present_while_streaming() {
        use ToolStatusView::Completed;
        // Even while the tail block streams, the indicator stays pinned to the
        // run's tail so the "working … + elapsed" signal is consistent — it no
        // longer flickers out during streaming / tool execution.
        let items = [
            ChatItem::UserText("q".into()),
            tool("a", Completed),
            ChatItem::AssistantText {
                text: "answer".into(),
                streaming: true,
                message_id: None,
            },
        ];
        let rows = project(&items, &FoldState::default(), true);
        assert!(
            kinds(&rows).iter().any(|(k, _)| *k == "working"),
            "a working turn keeps the indicator even while streaming"
        );
    }

    #[test]
    fn working_indicator_only_when_awaiting_response() {
        use ToolStatusView::Completed;
        let items = [
            ChatItem::UserText("q".into()),
            asst("planning"),
            tool("a", Completed),
            tool("b", Completed),
        ];
        // Settled turn, nothing in flight → no indicator.
        let rows = project(&items, &FoldState::default(), false);
        assert!(!kinds(&rows).iter().any(|(k, _)| *k == "working"));
    }

    #[test]
    fn working_indicator_on_first_token_wait() {
        // Prompt sent, no agent output yet, turn in flight → indicator under the
        // user message at top level (no response bar for an empty run).
        let items = [ChatItem::UserText("q".into())];
        let rows = project(&items, &FoldState::default(), true);
        assert_eq!(kinds(&rows), vec![("user", false), ("working", false)]);
        assert_eq!(rows.last().unwrap().indent, 0);
    }

    #[test]
    fn working_indicator_visible_when_response_collapsed() {
        use ToolStatusView::Completed;
        let items = [
            ChatItem::UserText("q".into()),
            asst("planning"),
            tool("a", Completed),
            tool("b", Completed),
        ];
        let mut fold = FoldState::default();
        // User manually collapses the (last, in-flight) response.
        fold.toggle(FoldKey::Response(0), true);
        let rows = project(&items, &fold, true);
        let working = rows
            .iter()
            .find(|r| matches!(r.kind, RowKind::WorkingIndicator))
            .expect("indicator still projected");
        assert!(
            !working.hidden,
            "an in-flight turn keeps its working indicator even when the response is collapsed"
        );
        // Pinned at the run's indent (1), aligned under the still-visible conclusion.
        assert_eq!(working.indent, 1);
    }

    #[test]
    fn conclusion_stays_visible_when_response_collapsed() {
        use ToolStatusView::Completed;
        let items = [
            ChatItem::UserText("q".into()),
            asst("let me look"),
            tool("a", Completed),
            tool("b", Completed),
            asst("done: fixed it"),
        ];
        let mut fold = FoldState::default();
        fold.toggle(FoldKey::Response(0), true); // collapse the response
        let rows = project(&items, &fold, false);
        assert_eq!(
            kinds(&rows),
            vec![
                ("user", false),
                ("response", false),
                ("item", true), // "let me look" process → hidden
                ("group", true),
                ("item", true),
                ("item", true),
                ("item", false), // "done: fixed it" conclusion → visible
            ]
        );
    }

    #[test]
    fn conclusion_under_a_response_is_a_separately_foldable_item() {
        use ToolStatusView::Completed;
        let items = [
            ChatItem::UserText("q".into()),
            asst("let me look"),
            tool("a", Completed),
            tool("b", Completed),
            asst("done"),
        ];
        let rows = project(&items, &FoldState::default(), false);
        // The final assistant block projects as a ConclusionItem (its own fold
        // toggle); the earlier prose stays a plain AgentItem.
        assert!(
            rows.iter()
                .any(|r| matches!(r.kind, RowKind::ConclusionItem(4))),
            "the run's last assistant text is a ConclusionItem"
        );
        assert!(
            rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(1))),
            "earlier prose stays a plain AgentItem"
        );
    }

    #[test]
    fn trivial_reply_is_not_a_conclusion_item() {
        // A lone reply has no response bar, so it renders as the normal
        // (labeled) foldable assistant block, not a bare-chevron ConclusionItem.
        let items = [ChatItem::UserText("hi".into()), asst("hello")];
        let rows = project(&items, &FoldState::default(), false);
        assert!(rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(1))));
        assert!(
            !rows
                .iter()
                .any(|r| matches!(r.kind, RowKind::ConclusionItem(_)))
        );
    }

    #[test]
    fn only_the_last_assistant_message_is_the_conclusion() {
        // Two distinct agent messages (mapping split them by messageId) with no
        // tool between → only the last is the conclusion; the earlier one folds
        // into the process.
        let items = [
            ChatItem::UserText("q".into()),
            asst("first message"),
            asst("second message"),
        ];
        let mut fold = FoldState::default();
        fold.toggle(FoldKey::Response(0), true);
        let rows = project(&items, &fold, false);
        assert_eq!(
            kinds(&rows),
            vec![
                ("user", false),
                ("response", false),
                ("item", true),  // first message → process
                ("item", false), // last message → conclusion
            ]
        );
    }

    #[test]
    fn conclusion_is_last_assistant_even_before_trailing_tool() {
        // The run ends with tools and no final text; the last assistant text
        // (mid-run) is still treated as the conclusion and stays visible.
        use ToolStatusView::Completed;
        let items = [
            ChatItem::UserText("q".into()),
            asst("answer"),
            tool("a", Completed),
            tool("b", Completed),
        ];
        let mut fold = FoldState::default();
        fold.toggle(FoldKey::Response(0), true);
        let rows = project(&items, &fold, false);
        assert_eq!(
            kinds(&rows),
            vec![
                ("user", false),
                ("response", false),
                ("item", false), // "answer" = last assistant text → visible
                ("group", true),
                ("item", true),
                ("item", true),
            ]
        );
    }

    #[test]
    fn no_conclusion_row_when_run_has_no_assistant_text() {
        use ToolStatusView::Completed;
        let items = [
            ChatItem::UserText("q".into()),
            tool("a", Completed),
            tool("b", Completed),
        ];
        let mut fold = FoldState::default();
        fold.toggle(FoldKey::Response(0), true);
        let rows = project(&items, &fold, false);
        assert_eq!(
            kinds(&rows),
            vec![
                ("user", false),
                ("response", false),
                ("group", true), // no assistant text → nothing stays visible
                ("item", true),
                ("item", true),
            ]
        );
    }

    #[test]
    fn pending_permission_stays_visible_when_response_collapsed() {
        use ToolStatusView::Completed;
        let items = [
            ChatItem::UserText("q".into()),
            tool("a", Completed),
            tool("b", Completed),
            perm(false), // pending → actionable
        ];
        let mut fold = FoldState::default();
        fold.toggle(FoldKey::Response(0), true); // collapse the response
        let rows = project(&items, &fold, false);
        let perm_row = rows
            .iter()
            .find(|r| matches!(r.kind, RowKind::AgentItem(3)))
            .expect("permission row present");
        assert!(
            !perm_row.hidden,
            "a pending permission is never folded away"
        );
        // The process (the tool group) is still hidden.
        assert!(
            rows.iter()
                .filter(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
                .all(|r| r.hidden),
            "the tool group still folds"
        );
    }

    #[test]
    fn resolved_permission_folds_with_the_response() {
        use ToolStatusView::Completed;
        let items = [
            ChatItem::UserText("q".into()),
            tool("a", Completed),
            tool("b", Completed),
            perm(true), // resolved → no longer actionable
        ];
        let mut fold = FoldState::default();
        fold.toggle(FoldKey::Response(0), true);
        let rows = project(&items, &fold, false);
        let perm_row = rows
            .iter()
            .find(|r| matches!(r.kind, RowKind::AgentItem(3)))
            .expect("permission row present");
        assert!(
            perm_row.hidden,
            "a resolved permission folds with the process"
        );
    }

    #[test]
    fn subagent_child_tool_calls_get_no_row() {
        use ToolStatusView::Completed;
        // A top-level Task/Agent parent plus one inner child linked by
        // `parent_tool_id`. The child renders nested inside the parent card
        // (see `tool_card`), so it must not appear as its own row — and the
        // parent + child must not collapse into a "2 tool calls" group.
        let mut child = tool("child", Completed);
        if let ChatItem::ToolCall(tc) = &mut child {
            tc.parent_tool_id = Some("parent".to_owned());
        }
        let items = [
            ChatItem::UserText("q".into()),
            tool("parent", Completed),
            child,
        ];
        let rows = project(&items, &FoldState::default(), false);
        assert!(
            rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(1))),
            "the parent tool call still renders as a row"
        );
        assert!(
            !rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(2))),
            "the subagent child earns no row of its own"
        );
        assert!(
            !rows
                .iter()
                .any(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. })),
            "a parent + its child are not a sibling tool group"
        );
    }

    #[test]
    fn multiple_subagent_children_all_skip_and_parent_stays_single() {
        use ToolStatusView::Completed;
        // Two inner calls under one parent: both skipped, and the parent renders
        // as a single card (not a "2 tool calls" group with its own children).
        let mut c1 = tool("c1", Completed);
        let mut c2 = tool("c2", Completed);
        for c in [&mut c1, &mut c2] {
            if let ChatItem::ToolCall(tc) = c {
                tc.parent_tool_id = Some("parent".to_owned());
            }
        }
        let items = [
            ChatItem::UserText("q".into()),
            tool("parent", Completed),
            c1,
            c2,
        ];
        let rows = project(&items, &FoldState::default(), false);
        assert!(rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(1))));
        assert!(
            !rows
                .iter()
                .any(|r| matches!(r.kind, RowKind::AgentItem(2) | RowKind::AgentItem(3))),
            "both subagent children are skipped from the main flow"
        );
        assert!(
            !rows
                .iter()
                .any(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. })),
            "a parent + its children are not a sibling tool group"
        );
    }

    #[test]
    fn orphan_child_keeps_its_row() {
        use ToolStatusView::Completed;
        // A child whose `parent_tool_id` matches no tool call in `items` (a
        // dangling ref) is nested by nobody, so it must keep a top-level row
        // rather than vanish silently.
        let mut orphan = tool("orphan", Completed);
        if let ChatItem::ToolCall(tc) = &mut orphan {
            tc.parent_tool_id = Some("missing-parent".to_owned());
        }
        let items = [ChatItem::UserText("q".into()), orphan];
        let rows = project(&items, &FoldState::default(), false);
        assert!(
            rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(1))),
            "an orphan child (no present parent) still renders as a row"
        );
    }

    fn child_of(id: &str, parent: &str, status: ToolStatusView) -> ChatItem {
        let mut c = tool(id, status);
        if let ChatItem::ToolCall(tc) = &mut c {
            tc.parent_tool_id = Some(parent.to_owned());
        }
        c
    }

    #[test]
    fn subagent_parent_reads_live_while_a_child_runs() {
        use ToolStatusView::{Completed, InProgress};
        // The reported bug: adapter marks the parent Task `Completed` (its SDK
        // call returned) while a flattened child keeps running — the unit is
        // still working, so the subtree must read live.
        let items = [
            tool("task", Completed),
            child_of("child", "task", InProgress),
        ];
        assert!(
            subagent_subtree_live(&items, "task", 8),
            "a live child keeps the subagent parent reading as running"
        );
    }

    #[test]
    fn subagent_parent_settles_when_all_children_terminal() {
        use ToolStatusView::{Completed, Failed};
        let items = [
            tool("task", Completed),
            child_of("a", "task", Completed),
            child_of("b", "task", Failed),
        ];
        assert!(
            !subagent_subtree_live(&items, "task", 8),
            "no live descendant → the parent unit is done"
        );
    }

    #[test]
    fn subagent_liveness_is_transitive() {
        use ToolStatusView::{Completed, InProgress};
        // parent → child(done) → grandchild(running): the running grandchild
        // still makes the whole unit live.
        let items = [
            tool("task", Completed),
            child_of("child", "task", Completed),
            child_of("grand", "child", InProgress),
        ];
        assert!(subagent_subtree_live(&items, "task", 8));
    }

    #[test]
    fn subagent_liveness_no_children_is_false() {
        let items = [tool("task", ToolStatusView::Completed)];
        assert!(!subagent_subtree_live(&items, "task", 8));
    }

    #[test]
    fn subagent_liveness_depth_cap_terminates_on_cycle() {
        use ToolStatusView::Completed;
        // A malformed self-parent (id == parent_tool_id) with a terminal status
        // must not recurse without bound; the depth budget forces `false`.
        let items = [child_of("x", "x", Completed)];
        assert!(!subagent_subtree_live(&items, "x", 8));
    }

    #[test]
    fn collapsed_response_surfaces_a_live_tool_group() {
        use ToolStatusView::{Completed, InProgress};
        // A non-trivial response (a 2-tool group) with one live member, response
        // manually collapsed: the group header pops out (still folded) so a
        // folded in-flight turn shows *what* is running; members stay hidden.
        let items = [
            ChatItem::UserText("q".into()),
            tool("t1", InProgress),
            tool("t2", Completed),
        ];
        let mut fold = FoldState::default();
        fold.set_all([FoldKey::Response(0)], false); // force the response collapsed
        let rows = project(&items, &fold, false);
        let header = rows
            .iter()
            .find(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
            .expect("group header present");
        assert!(
            !header.hidden,
            "a collapsed response still surfaces a live tool group header"
        );
        assert!(
            rows.iter()
                .filter(|r| matches!(r.kind, RowKind::AgentItem(_)))
                .all(|r| r.hidden),
            "the live group's members stay folded under the collapsed response"
        );
    }

    #[test]
    fn collapsed_response_hides_a_settled_tool_group() {
        use ToolStatusView::Completed;
        let items = [
            ChatItem::UserText("q".into()),
            tool("t1", Completed),
            tool("t2", Completed),
        ];
        let mut fold = FoldState::default();
        fold.set_all([FoldKey::Response(0)], false);
        let rows = project(&items, &fold, false);
        let header = rows
            .iter()
            .find(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
            .expect("group header present");
        assert!(
            header.hidden,
            "a settled tool group folds away with the collapsed response"
        );
    }

    #[test]
    fn lone_tool_call_is_not_grouped() {
        let items = [asst("x"), tool("a", ToolStatusView::Completed), asst("y")];
        let rows = project(&items, &FoldState::default(), false);
        assert_eq!(
            kinds(&rows),
            vec![("item", false), ("item", false), ("item", false)],
            "a single tool call renders as a plain item, no group header"
        );
    }

    #[test]
    fn in_progress_group_defaults_expanded() {
        use ToolStatusView::{Completed, InProgress};
        let items = [tool("a", Completed), tool("b", InProgress)];
        let rows = project(&items, &FoldState::default(), false);
        // group active (one tool in progress) → members visible.
        assert_eq!(
            kinds(&rows),
            vec![("group", false), ("item", false), ("item", false)]
        );
    }

    #[test]
    fn group_member_visibility_follows_fold_override() {
        use ToolStatusView::Completed;
        let items = [tool("a", Completed), tool("b", Completed)];
        let mut fold = FoldState::default();
        // Force-expand the (otherwise collapsed) settled group.
        fold.toggle(FoldKey::ToolGroup("a".into()), false);
        let rows = project(&items, &fold, false);
        assert_eq!(
            kinds(&rows),
            vec![("group", false), ("item", false), ("item", false)],
            "user-expanded group shows its members"
        );
    }

    #[test]
    fn same_slot_compares_key_not_hidden_or_payload() {
        let a = RenderRow {
            kind: RowKind::ToolGroupHeader {
                gid: "g".into(),
                first_ix: 1,
                count: 2,
                collapsed: false,
            },
            hidden: false,
            indent: 0,
        };
        let b = RenderRow {
            kind: RowKind::ToolGroupHeader {
                gid: "g".into(),
                first_ix: 5,
                count: 3,
                collapsed: true,
            },
            hidden: true,
            indent: 0,
        };
        assert!(
            a.same_slot(&b),
            "same gid → same slot regardless of count/hidden"
        );

        let u0 = RenderRow {
            kind: RowKind::User(0),
            hidden: false,
            indent: 0,
        };
        let u1 = RenderRow {
            kind: RowKind::User(1),
            hidden: false,
            indent: 0,
        };
        assert!(!u0.same_slot(&u1));
        assert!(!u0.same_slot(&RenderRow {
            kind: RowKind::AgentItem(0),
            hidden: false,
            indent: 0
        }));
    }
}
