//! The fixed conversation the agent-chat `--screenshot` scenarios seed.
//!
//! Screenshot-only, so it never reaches the shipping binary. It exists because
//! the Step layer, the tail window and the display filter are only judgeable on
//! screen, and a live ACP session is neither reproducible nor available in a
//! capture. The shape is chosen to put all of them on screen at once: enough
//! work cycles for the tail window to have something to hide, mixed tool kinds
//! so the Step header glyph is not the same terminal icon nine times, a failed
//! call so the status rollup shows, and one edit carrying a diff.

use std::path::PathBuf;

use daruda_acp::{ChatItem, DiffView, MessagePhase, ToolCallItem, ToolKindView, ToolStatusView};

/// A file modification a sample call reports.
struct SampleDiff {
    path: &'static str,
    old_text: &'static str,
    new_text: &'static str,
}

/// One tool run inside a cycle. `tool_name` is `Some` for the agents that
/// report one (Claude) and `None` for the ones that do not (Codex) — both
/// shapes appear here so the header's name-vs-kind fallback is visible.
struct Call {
    title: &'static str,
    kind: ToolKindView,
    tool_name: Option<&'static str>,
    status: ToolStatusView,
    diff: Option<SampleDiff>,
}

impl Call {
    fn new(title: &'static str, kind: ToolKindView, tool_name: Option<&'static str>) -> Self {
        Self {
            title,
            kind,
            tool_name,
            status: ToolStatusView::Completed,
            diff: None,
        }
    }

    fn failed(mut self) -> Self {
        self.status = ToolStatusView::Failed;
        self
    }

    fn with_diff(mut self, diff: SampleDiff) -> Self {
        self.diff = Some(diff);
        self
    }
}

/// One work cycle. A Step header is earned by a tool run that has prose in
/// front of it, so every cycle carries reasoning, the sentence that introduces
/// the work, and the calls themselves.
struct Cycle {
    thinking: &'static str,
    prose: &'static str,
    tools: Vec<Call>,
}

/// The sentence the conversation ends on — trailing prose belongs to no step,
/// so this renders as the response's conclusion rather than inside a fold.
const CONCLUSION: &str = "The failure was a stale `rust-version` floor in `daruda_terminal`: CI pins 1.95, \
     the crate inherited nothing, and `incompatible_msrv` never fired at the call site. \
     Declaring the floor and rebuilding gets the workspace green again.";

const PROMPT: &str = "why is the build failing on CI but not locally?";

fn cycles() -> Vec<Cycle> {
    vec![
        Cycle {
            thinking: "**Reading the failure first** — the CI log names the crate, \
                       guessing from the source tree does not.",
            prose: "Let me start from what CI actually printed.",
            tools: vec![
                Call::new(
                    "grep -n \"error\\[E\" ci/build.log",
                    ToolKindView::Search,
                    Some("Grep"),
                ),
                Call::new("Read ci/build.log (lines 840-980)", ToolKindView::Read, Some("Read")),
            ],
        },
        Cycle {
            thinking: "**Locating the crate** — the error points at `daruda_terminal`, \
                       so its manifest and the workspace floor are the two places to look.",
            prose: "Now I'll read the manifests that decide the toolchain floor.",
            tools: vec![
                Call::new("Read crates/daruda_terminal/Cargo.toml", ToolKindView::Read, Some("Read")),
                Call::new("Read Cargo.toml", ToolKindView::Read, Some("Read")),
                Call::new("Read .github/workflows/ci.yml", ToolKindView::Read, Some("Read")),
            ],
        },
        Cycle {
            thinking: "**Reproducing under CI's toolchain** — locally I am on a newer \
                       compiler, which is exactly why the call site compiles here.",
            prose: "Reproducing with the pinned toolchain.",
            tools: vec![
                Call::new("rustup run 1.95.0 cargo check -p daruda_terminal", ToolKindView::Execute, Some("Bash")),
                Call::new("cargo tree -p daruda_terminal --depth 1", ToolKindView::Execute, Some("Bash")),
            ],
        },
        Cycle {
            thinking: "**The floor is missing** — every first-party crate inherits \
                       `rust-version` from the workspace; this one declares nothing.",
            prose: "Declaring the floor so `incompatible_msrv` arms in this crate too.",
            tools: vec![
                Call::new("Edit crates/daruda_terminal/Cargo.toml", ToolKindView::Edit, Some("Edit")).with_diff(
                    SampleDiff {
                        path: "crates/daruda_terminal/Cargo.toml",
                        old_text: "[package]\nname = \"daruda_terminal\"\nedition.workspace = true\nlicense.workspace = true\n",
                        new_text: "[package]\nname = \"daruda_terminal\"\nedition.workspace = true\nrust-version.workspace = true\nlicense.workspace = true\n",
                    },
                ),
                Call::new("Edit crates/daruda_terminal/src/ux/theme.rs", ToolKindView::Edit, Some("Edit")),
            ],
        },
        Cycle {
            thinking: "**Checking the rebuild** — the clippy pass is the one that \
                       actually enforces the floor, so run that, not just `check`.",
            prose: "Rebuilding under the pinned toolchain.",
            tools: vec![
                Call::new("rustup run 1.95.0 cargo clippy -p daruda_terminal --all-targets", ToolKindView::Execute, None).failed(),
                Call::new("tail -40 target/clippy.log", ToolKindView::Execute, None),
                Call::new("rustup run 1.95.0 cargo clippy -p daruda_terminal --all-targets", ToolKindView::Execute, None),
            ],
        },
        Cycle {
            thinking: "**Looking for the same gap elsewhere** — if one crate lost the \
                       declaration, the others are worth a sweep before claiming a fix.",
            prose: "Sweeping the rest of the workspace for the same omission.",
            tools: vec![
                Call::new("rg -n \"rust-version\" crates/*/Cargo.toml", ToolKindView::Search, None),
                Call::new("rg -n \"incompatible_msrv\" crates", ToolKindView::Search, None),
                Call::new("rg --files-without-match \"rust-version\" -g Cargo.toml crates", ToolKindView::Search, None),
            ],
        },
        Cycle {
            thinking: "**Confirming the vendored carve-out** — `gpui_component` is \
                       deliberately undeclared, so it is not part of the defect.",
            prose: "Confirming the vendored crates are an intended exception.",
            tools: vec![
                Call::new("Read patches/README.md", ToolKindView::Read, Some("Read")),
                Call::new("Read crates/ferrum_flow/Cargo.toml", ToolKindView::Read, Some("Read")),
                Call::new("Edit CLAUDE.md", ToolKindView::Edit, Some("Edit")),
            ],
        },
        Cycle {
            thinking: "**Running the gate CI runs** — the workspace clippy list is what \
                       the pipeline calls, so match it exactly.",
            prose: "Running the same clippy list CI gates on.",
            tools: vec![
                Call::new("rustup run 1.95.0 cargo clippy -p ghostty_vt -p daruda_terminal -p daruda", ToolKindView::Execute, Some("Bash")),
                Call::new("cargo fmt --all -- --check", ToolKindView::Execute, Some("Bash")),
            ],
        },
        Cycle {
            thinking: "**Last pass** — record the floor in the docs that promise it, \
                       then run the suite once more.",
            prose: "Recording the rule and re-running the suite.",
            tools: vec![
                Call::new("Edit crates/daruda_terminal/CLAUDE.md", ToolKindView::Edit, Some("Edit")),
                Call::new("Read scripts/lint-file-size.sh", ToolKindView::Read, Some("Read")),
                Call::new("cargo test -p daruda_terminal", ToolKindView::Execute, Some("Bash")),
            ],
        },
    ]
}

/// Build the seeded conversation: one user prompt, then every cycle as
/// prose + tool run, closed by the conclusion.
pub(in crate::workspace) fn sample_transcript() -> Vec<ChatItem> {
    let mut items = vec![ChatItem::UserText(PROMPT.to_string())];
    let mut next_id = 0usize;
    for cycle in cycles() {
        items.push(thinking(cycle.thinking));
        // Each cycle's prose is a preamble, not a reply. The seed already
        // models the shape an agent that labels its messages sends — a thought
        // summary, the preamble, then the tools — so the label belongs here
        // too; without it the header this seed exists to show never engages.
        items.push(assistant(cycle.prose, MessagePhase::Commentary));
        for call in cycle.tools {
            items.push(tool_call(next_id, call));
            next_id += 1;
        }
    }
    items.push(assistant(CONCLUSION, MessagePhase::Answer));
    items
}

fn thinking(text: &str) -> ChatItem {
    ChatItem::Thinking {
        text: text.to_string(),
        streaming: false,
        message_id: None,
    }
}

fn assistant(text: &str, phase: MessagePhase) -> ChatItem {
    ChatItem::AssistantText {
        text: text.to_string(),
        streaming: false,
        message_id: None,
        phase,
    }
}

fn tool_call(ix: usize, call: Call) -> ChatItem {
    ChatItem::ToolCall(ToolCallItem {
        id: format!("shot-tool-{ix}"),
        title: call.title.to_string(),
        kind: call.kind,
        tool_name: call.tool_name.map(str::to_string),
        status: call.status,
        diffs: call
            .diff
            .into_iter()
            .map(|d| DiffView {
                path: PathBuf::from(d.path),
                old_text: Some(d.old_text.to_string()),
                new_text: d.new_text.to_string(),
            })
            .collect(),
        output: Vec::new(),
        raw_input: None,
        parent_tool_id: None,
        exit: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One cycle earns one Step, and the tail window can only be looked at
    /// when there is more history than the largest offered window keeps.
    #[test]
    fn the_seed_has_more_work_cycles_than_the_tail_window_keeps() {
        let items = sample_transcript();
        let seeded = items
            .iter()
            .filter(|i| matches!(i, ChatItem::Thinking { .. }))
            .count();
        assert!(seeded >= 8, "cycles: {seeded}");
        // Every cycle is prose followed by at least one call, which is what a
        // Step header is derived from.
        for cycle in cycles() {
            assert!(!cycle.tools.is_empty());
            assert!(!cycle.prose.is_empty());
        }
    }

    #[test]
    fn the_seed_varies_tool_kinds_names_and_statuses() {
        let items = sample_transcript();
        let calls: Vec<_> = items
            .iter()
            .filter_map(|i| match i {
                ChatItem::ToolCall(tc) => Some(tc),
                _ => None,
            })
            .collect();
        for kind in [
            ToolKindView::Read,
            ToolKindView::Edit,
            ToolKindView::Search,
            ToolKindView::Execute,
        ] {
            assert!(calls.iter().any(|c| c.kind == kind), "missing {kind:?}");
        }
        assert!(calls.iter().any(|c| c.tool_name.is_some()));
        assert!(calls.iter().any(|c| c.tool_name.is_none()));
        assert!(calls.iter().any(|c| c.status == ToolStatusView::Failed));
        assert!(calls.iter().any(|c| !c.diffs.is_empty()));
    }

    #[test]
    fn every_tool_call_gets_its_own_id() {
        let items = sample_transcript();
        let ids: Vec<_> = items
            .iter()
            .filter_map(|i| match i {
                ChatItem::ToolCall(tc) => Some(tc.id.clone()),
                _ => None,
            })
            .collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len());
    }
}
