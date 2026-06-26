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
pub(in crate::workspace) fn project(items: &[ChatItem], fold: &FoldState) -> Vec<RenderRow> {
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

        if let (true, Some(a)) = (non_trivial, anchor) {
            let streaming = run.clone().any(|k| is_active(&items[k]));
            let collapsed = !fold.is_expanded(&FoldKey::Response(a), is_last_turn || streaming);
            rows.push(RenderRow {
                kind: RowKind::ResponseHeader {
                    anchor: a,
                    collapsed,
                },
                hidden: false,
                indent: 0,
            });
            project_run(items, fold, run, 1, collapsed, &mut rows);
        } else {
            project_run(items, fold, run, 0, false, &mut rows);
        }
    }
    rows
}

/// Project one agent run (the items of a single response) into rows at
/// `base_indent`, grouping consecutive tool-call runs. `response_collapsed`
/// hides every row in the run (the response is folded); a settled tool group
/// additionally hides its own members.
fn project_run(
    items: &[ChatItem],
    fold: &FoldState,
    run: std::ops::Range<usize>,
    base_indent: u8,
    response_collapsed: bool,
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
            rows.push(RenderRow {
                kind: RowKind::AgentItem(k),
                hidden: response_collapsed,
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
    use daruda_acp::{ToolCallItem, ToolKindView, ToolStatusView};

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
    fn asst(s: &str) -> ChatItem {
        ChatItem::AssistantText {
            text: s.to_owned(),
            streaming: false,
        }
    }
    fn kinds(rows: &[RenderRow]) -> Vec<(&'static str, bool)> {
        rows.iter()
            .map(|r| {
                let k = match r.kind {
                    RowKind::User(_) => "user",
                    RowKind::ResponseHeader { .. } => "response",
                    RowKind::AgentItem(_) => "item",
                    RowKind::ToolGroupHeader { .. } => "group",
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
        let rows = project(&items, &FoldState::default());
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
        let rows = project(&items, &FoldState::default());
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
        let rows = project(&items, &FoldState::default());
        // Past response (first turn, settled, not last) collapses → its run is
        // hidden; current response (last turn) expands → its run visible.
        assert_eq!(
            kinds(&rows),
            vec![
                ("user", false),     // first
                ("response", false), // header always shown
                ("item", true),      // a1 hidden (response collapsed)
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
    fn lone_tool_call_is_not_grouped() {
        let items = [asst("x"), tool("a", ToolStatusView::Completed), asst("y")];
        let rows = project(&items, &FoldState::default());
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
        let rows = project(&items, &FoldState::default());
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
        let rows = project(&items, &fold);
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
