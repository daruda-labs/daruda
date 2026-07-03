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

use daruda_acp::ChatItem;

use super::agent_chat_ops::is_active;
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
    /// most one exists (always the last row). Suppressed on a permission prompt
    /// or a manually-collapsed run.
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
/// bureaucratic "Agent" bar on every trivial answer).
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
/// `awaiting_response` (the view's "a turn is in flight and not blocked on user
/// input" flag — `turn_in_flight && pending_permission.is_none()`) drives a
/// trailing [`RowKind::WorkingIndicator`] pinned to the last turn's tail: it
/// stays for the whole turn — through tool execution and streaming — so the
/// "working … + elapsed" signal is consistently present, not just in the gap
/// between blocks. Suppressed only when blocked on a permission prompt (the
/// card carries that state) or when the turn's response is manually collapsed.
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
        let run_active = run.clone().any(|k| is_active(&items[k]));
        // The run's final assistant-text block is the turn's conclusion: it
        // stays visible when the response is collapsed (see `project_run`).
        let conclusion_ix = run
            .clone()
            .rev()
            .find(|&k| matches!(items[k], ChatItem::AssistantText { .. }));

        // Indent + collapsed state of this turn's run, reused to place the
        // trailing working indicator inside the same fold scope.
        let (run_indent, run_collapsed) = if let (true, Some(a)) = (non_trivial, anchor) {
            let collapsed = !fold.is_expanded(&FoldKey::Response(a), is_last_turn || run_active);
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
            (1u8, collapsed)
        } else {
            project_run(items, fold, run.clone(), 0, false, conclusion_ix, &mut rows);
            (0u8, false)
        };

        // While a turn is in flight (and not blocked on a permission prompt),
        // pin a live "agent is working" indicator to the last turn's tail — for
        // the whole turn, through tool execution and streaming alike — so the
        // "working … + elapsed" signal is consistently present instead of
        // flickering out whenever a block is active. Last turn only; hidden with
        // the run when the response is manually collapsed so a folded turn stays
        // quiet.
        if awaiting_response && is_last_turn {
            rows.push(RenderRow {
                kind: RowKind::WorkingIndicator,
                hidden: run_collapsed,
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
        if matches!(items[k], ChatItem::ToolCall(_)) {
            let gstart = k;
            while k < run.end && matches!(items[k], ChatItem::ToolCall(_)) {
                k += 1;
            }
            let grun = gstart..k;
            if grun.len() >= TOOL_GROUP_MIN {
                let gid = tool_id(&items[gstart]);
                let active = grun.clone().any(|j| is_active(&items[j]));
                let group_collapsed = !fold.is_expanded(&FoldKey::ToolGroup(gid.clone()), active);
                rows.push(RenderRow {
                    kind: RowKind::ToolGroupHeader {
                        gid,
                        first_ix: gstart,
                        count: grun.len(),
                        collapsed: group_collapsed,
                    },
                    hidden: response_collapsed,
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
                    hidden: response_collapsed,
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
        })
    }
    /// A permission card — `resolved=false` makes it pending (actionable).
    fn perm(resolved: bool) -> ChatItem {
        ChatItem::Permission(PermissionItem {
            tool_title: Some("Write /tmp/x".to_owned()),
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
    fn working_indicator_only_when_turn_in_flight() {
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
    fn working_indicator_hidden_when_response_collapsed() {
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
            working.hidden,
            "a manually-collapsed response hides its working indicator too"
        );
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
