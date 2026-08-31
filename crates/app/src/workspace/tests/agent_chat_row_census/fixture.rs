//! Run-length encoded conversation shapes transcribed from captured Codex and
//! Claude ACP logs. Content is synthetic; item ordering and tool fields match.

use daruda_acp::{ChatItem, CommandExit, ToolCallItem, ToolKindView, ToolStatusView};

/// `T`, `A`, and `C` encode thinking, assistant, and tool-call counts.
type Skeleton = &'static [&'static [&'static str]];

/// Codex log items 0..301: 58 cycles followed by trailing prose.
const CODEX_TURN_1: &[&str] = &[
    "T2 A1 C3",
    "T2 A1 C3",
    "T1 A1 C3",
    "T2 A1 C4",
    "T2 A1 C4",
    "T1 A1 C3",
    "T1 A1 C4",
    "T1 A1 C4",
    "T1 A1 C4",
    "T1 C4",
    "T1 A1 C4",
    "T1 C4",
    "T1 C4",
    "T1 C4",
    "T1 A1 C4",
    "T2 A1 C4",
    "T1 C5",
    "T1 A1 C4",
    "T2 A1 C3",
    "T1 C2",
    "T2 C3",
    "T1 C4",
    "T1 C1",
    "T2 A1 C3",
    "T2 A1 C3",
    "T2 A1 C4",
    "T1 A1 C3",
    "T2 A1 C4",
    "T2 C4",
    "T1 C1",
    "A1 T1 C4",
    "T2 C3",
    "T2 C2",
    "T2 C1",
    "T1 C1",
    "T2 A1 C4",
    "T2 C1",
    "T2 A1 C4",
    "T1 A1 C2",
    "T2 C5",
    "T1 A1 C3",
    "T2 C4",
    "T1 C4",
    "T1 A1 C3",
    "T2 A1 C4",
    "T1 C3",
    "T1 A1 C3",
    "T2 A1 T1 C1",
    "T2 C1",
    "T1 C1",
    "T1 C1",
    "T1 A1 C4",
    "A1 T1 A1 C1",
    "T2 C3",
    "T2 C1",
    "T2 C1",
    "T2 A1 C4",
    "A1 T2 A1 C3",
    "T2 A1",
];

/// Codex log items 301..303: one bare tool call.
const CODEX_TURN_2: &[&str] = &["C1"];

/// Codex log items 303..352: 10 short cycles.
const CODEX_TURN_3: &[&str] = &[
    "T1 A1 C4", "T1 A1 C4", "T1 A1 C4", "T1 A1 C3", "T2 A1 C1", "T1 A1 C3", "T1 A1 C1", "T1 C1",
    "T1 A1 C4", "T1 A1 C1", "T1 A1",
];

const CODEX_SESSION: Skeleton = &[CODEX_TURN_1, CODEX_TURN_2, CODEX_TURN_3];

/// Claude log items 0..2: one prose response.
const CLAUDE_TURN_1: &[&str] = &["A1"];

/// Claude log items 2..68: tool runs of 12, 13, and 36 calls.
const CLAUDE_TURN_2: &[&str] = &["A1 C12", "A1 C13", "A1 C36", "A1"];

/// Claude log items 68..132: 22 short cycles without thinking blocks.
const CLAUDE_TURN_3: &[&str] = &[
    "A1 C3", "A1 C2", "A1 C2", "A1 C2", "A1 C3", "A1 C1", "A1 C3", "A1 C2", "A1 C1", "A1 C2",
    "A1 C2", "A1 C1", "A1 C2", "A1 C2", "A1 C2", "A1 C1", "A1 C1", "A1 C1", "A1 C2", "A1 C1",
    "A1 C3", "A1 C1", "A1",
];

const CLAUDE_SESSION: Skeleton = &[CLAUDE_TURN_1, CLAUDE_TURN_2, CLAUDE_TURN_3];

/// Tool-field shape emitted by each adapter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ToolStyle {
    Codex,
    Claude,
}

/// Rotated Codex kinds reproduce the captured 68/203 exit-bearing calls.
const CODEX_KINDS: [ToolKindView; 3] = [
    ToolKindView::Read,
    ToolKindView::Execute,
    ToolKindView::Search,
];

const CLAUDE_TOOL_NAME: &str = "Bash";

pub(super) fn codex_session() -> Vec<ChatItem> {
    build(CODEX_SESSION, ToolStyle::Codex)
}

pub(super) fn claude_session() -> Vec<ChatItem> {
    build(CLAUDE_SESSION, ToolStyle::Claude)
}

fn build(session: Skeleton, style: ToolStyle) -> Vec<ChatItem> {
    let mut items = Vec::new();
    let mut seq = 0usize;
    let mut tool_ix = 0usize;
    for (turn, cycles) in session.iter().enumerate() {
        items.push(ChatItem::UserText(format!("prompt {turn}")));
        for token in cycles.iter().flat_map(|c| c.split_whitespace()) {
            let (kind, count) = token.split_at(1);
            let count: usize = count
                .parse()
                .unwrap_or_else(|_| panic!("skeleton token {token:?} has no count"));
            for _ in 0..count {
                seq += 1;
                items.push(match kind {
                    "T" => thinking(seq),
                    "A" => assistant(seq),
                    "C" => {
                        tool_ix += 1;
                        tool(style, seq, tool_ix - 1)
                    }
                    _ => panic!("unknown skeleton token {token:?}"),
                });
            }
        }
    }
    items
}

fn thinking(seq: usize) -> ChatItem {
    ChatItem::Thinking {
        text: format!("reasoning {seq}"),
        streaming: false,
        message_id: None,
    }
}

fn assistant(seq: usize) -> ChatItem {
    ChatItem::AssistantText {
        text: format!("prose {seq}"),
        streaming: false,
        message_id: None,
        phase: Default::default(),
    }
}

fn tool(style: ToolStyle, seq: usize, tool_ix: usize) -> ChatItem {
    let kind = match style {
        ToolStyle::Codex => CODEX_KINDS[tool_ix % CODEX_KINDS.len()],
        ToolStyle::Claude => ToolKindView::Execute,
    };
    let tool_name = match style {
        ToolStyle::Codex => None,
        ToolStyle::Claude => Some(CLAUDE_TOOL_NAME.to_owned()),
    };
    let exit = match (style, kind) {
        (ToolStyle::Codex, ToolKindView::Execute) => Some(CommandExit {
            code: Some(0),
            signal: None,
        }),
        _ => None,
    };
    ChatItem::ToolCall(ToolCallItem {
        id: format!("tool-{seq}"),
        title: format!("tool call {seq}"),
        kind,
        tool_name,
        status: ToolStatusView::Completed,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: None,
        parent_tool_id: None,
        exit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools(items: &[ChatItem]) -> Vec<&ToolCallItem> {
        items
            .iter()
            .filter_map(|it| match it {
                ChatItem::ToolCall(tc) => Some(tc),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn each_fixture_has_the_item_census_of_the_log_it_was_transcribed_from() {
        let codex = codex_session();
        assert_eq!(codex.len(), 352);
        assert_eq!(tools(&codex).len(), 203);
        let claude = claude_session();
        assert_eq!(claude.len(), 132);
        assert_eq!(tools(&claude).len(), 101);
        assert!(
            !claude
                .iter()
                .any(|it| matches!(it, ChatItem::Thinking { .. })),
            "the captured Claude session emits no reasoning blocks at all"
        );
    }

    #[test]
    fn the_two_fixtures_carry_disjoint_tool_signals() {
        let codex = codex_session();
        let codex_tools = tools(&codex);
        assert!(codex_tools.iter().all(|tc| tc.tool_name.is_none()));
        for kind in CODEX_KINDS {
            assert!(
                codex_tools.iter().any(|tc| tc.kind == kind),
                "codex spreads its calls over {kind:?} too"
            );
        }
        assert_eq!(
            codex_tools.iter().filter(|tc| tc.exit.is_some()).count(),
            68
        );

        let claude = claude_session();
        let claude_tools = tools(&claude);
        assert!(
            claude_tools
                .iter()
                .all(|tc| tc.tool_name.as_deref() == Some(CLAUDE_TOOL_NAME))
        );
        assert!(
            claude_tools
                .iter()
                .all(|tc| tc.kind == ToolKindView::Execute && tc.exit.is_none())
        );
        assert!(claude_tools.iter().all(|tc| tc.diffs.is_empty()));
    }
}
