//! The fixed conversation the agent-chat `--screenshot` scenarios seed.
//!
//! Screenshot-only, so it never reaches the shipping binary. It exists because
//! the group bars, the tail window and the display filter are only judgeable on
//! screen, and a live ACP session is neither reproducible nor available in a
//! capture. The shape is chosen to put all of them on screen at once: enough
//! work cycles for the tail window to have something to hide, mixed tool kinds,
//! a failed call so the status rollup shows, and one edit carrying a diff.

use std::path::PathBuf;

use daruda_acp::{ChatItem, DiffView, MessagePhase, ToolCallItem, ToolKindView, ToolStatusView};

use super::agent_chat_ops::SHOT_GROUP_TAIL_WINDOW;

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

/// One work cycle: reasoning, the sentence that introduces the work, and the
/// calls themselves — at least two, so the run earns a group bar.
struct Cycle {
    thinking: &'static str,
    prose: &'static str,
    tools: Vec<Call>,
}

/// The sentence the conversation ends on — trailing prose follows every tool
/// run, so this renders as the response's conclusion.
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
        // Each cycle's prose is a preamble, not a reply — the shape an agent
        // that labels its messages sends, which is what the prose filter's
        // preamble facet keys off.
        items.push(assistant(cycle.prose, MessagePhase::Commentary));
        for call in cycle.tools {
            items.push(tool_call(next_id, call));
            next_id += 1;
        }
    }
    items.push(assistant(CONCLUSION, MessagePhase::Answer));
    items
}

/// The parent call every [`subagent_transcript`] child names.
pub(in crate::workspace) const SUBAGENT_PARENT_ID: &str = "shot-subagent";
/// More children than the capture's window keeps, so the card's own boundary
/// has something to hold back.
const SUBAGENT_CHILDREN: [(&str, ToolKindView); 7] = [
    ("Read crates/app/src/workspace/mod.rs", ToolKindView::Read),
    ("rg -n \"LaneRef\" crates/app/src", ToolKindView::Search),
    ("Read crates/app/src/lane/mod.rs", ToolKindView::Read),
    ("rg -n \"last_active_lane_id\" crates", ToolKindView::Search),
    (
        "Read crates/daruda_store/src/project/lane.rs",
        ToolKindView::Read,
    ),
    ("cargo test -p daruda lane", ToolKindView::Execute),
    (
        "Read crates/app/src/workspace/persistence.rs",
        ToolKindView::Read,
    ),
];

/// A conversation whose one turn delegates to a subagent: the `Task` launch and
/// the calls the adapter flattened under it, linked by `parent_tool_id`.
///
/// Its own seed rather than another cycle in [`sample_transcript`]: those
/// children own no row, so they would change nothing in the transcript's list —
/// the parent's card is the only place they appear.
pub(in crate::workspace) fn subagent_transcript() -> Vec<ChatItem> {
    let mut items = vec![
        ChatItem::UserText("how does a lane get its last active session back?".to_string()),
        assistant(
            "Delegating the survey so the answer comes back with the call sites attached.",
            MessagePhase::Commentary,
        ),
        ChatItem::ToolCall(ToolCallItem {
            id: SUBAGENT_PARENT_ID.to_string(),
            title: "Explore how a lane restores its last active session".to_string(),
            kind: ToolKindView::Think,
            tool_name: Some("Task".to_string()),
            status: ToolStatusView::Completed,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: Some(serde_json::json!({
                "subagent_type": "general-purpose",
                "prompt": "Trace how a lane's last active session is persisted and restored.",
            })),
            parent_tool_id: None,
            exit: None,
        }),
    ];
    for (i, (title, kind)) in SUBAGENT_CHILDREN.into_iter().enumerate() {
        items.push(ChatItem::ToolCall(ToolCallItem {
            id: format!("{SUBAGENT_PARENT_ID}-child-{i}"),
            title: title.to_string(),
            kind,
            tool_name: None,
            status: ToolStatusView::Completed,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: Some(SUBAGENT_PARENT_ID.to_string()),
            exit: None,
        }));
    }
    items.push(assistant(
        "`SerializedLane::last_active_lane_id` is the snap target: `restore_workspace` \
         reads it back and `activate_lane` writes it on every switch.",
        MessagePhase::Answer,
    ));
    items
}

/// [`sample_transcript`] as it stands mid-turn: the agent has not written its
/// answer yet, so the run's last prose is a preamble rather than a conclusion.
/// Derived from the settled seed rather than assembled again, so the two cannot
/// drift apart.
pub(in crate::workspace) fn working_transcript() -> Vec<ChatItem> {
    let mut items = sample_transcript();
    items.pop();
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

    /// One cycle earns one tool run, and the tail window can only be looked at
    /// when there is more history than the largest offered window keeps.
    #[test]
    fn the_seed_has_more_work_cycles_than_the_tail_window_keeps() {
        let items = sample_transcript();
        let seeded = items
            .iter()
            .filter(|i| matches!(i, ChatItem::Thinking { .. }))
            .count();
        assert!(seeded >= 8, "cycles: {seeded}");
        // Every cycle is prose followed by a run of at least two calls, which
        // is what earns a group bar.
        for cycle in cycles() {
            assert!(cycle.tools.len() >= 2);
            assert!(!cycle.prose.is_empty());
        }
        // The in-group boundary capture engages a window one narrower than the
        // longest group, so the seed has to hold a group of more than two —
        // otherwise `open_agent_chat_group_tail_boundary_for_shot` finds no
        // target and quietly captures the wrong state.
        assert!(
            cycles().iter().any(|c| c.tools.len() > 2),
            "no group long enough for the in-group boundary capture"
        );
    }

    /// The card's children have to be recognized as its children, and the
    /// parent as a subagent launch — both are what put them inside the card
    /// instead of in the transcript's own list.
    #[test]
    fn the_subagent_seed_links_its_children_to_a_recognized_launch() {
        let items = subagent_transcript();
        let launch = items
            .iter()
            .find_map(|i| match i {
                ChatItem::ToolCall(tc) if tc.id == SUBAGENT_PARENT_ID => Some(tc),
                _ => None,
            })
            .expect("the seed carries its Task launch");
        assert!(
            launch.is_subagent_launch(),
            "raw_input must name the subagent"
        );
        let children = items
            .iter()
            .filter(|i| {
                matches!(i, ChatItem::ToolCall(tc)
                    if tc.parent_tool_id.as_deref() == Some(SUBAGENT_PARENT_ID))
            })
            .count();
        assert_eq!(children, SUBAGENT_CHILDREN.len());
        // The capture engages a window narrower than the child count, or the
        // card's own boundary has nothing to hold back and the capture shows a
        // state the feature does not have.
        assert!(
            SUBAGENT_CHILDREN.len() > SHOT_GROUP_TAIL_WINDOW,
            "no subagent card long enough for the in-card boundary capture"
        );
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

    /// The working seed must actually be the mid-turn shape: prose last, with
    /// a tool run after it. Asserted here because the scenario it feeds exists
    /// only to put that shape on screen.
    #[test]
    fn the_working_seed_ends_on_a_preamble_rather_than_an_answer() {
        let settled = sample_transcript();
        let working = working_transcript();
        assert_eq!(working.len() + 1, settled.len());
        assert!(
            matches!(settled.last(), Some(ChatItem::AssistantText { phase, .. }) if *phase == MessagePhase::Answer),
            "the settled seed ends on the answer this one drops"
        );
        let last_prose = working
            .iter()
            .rposition(|i| matches!(i, ChatItem::AssistantText { .. }))
            .expect("the seed has prose");
        assert!(
            working[last_prose + 1..]
                .iter()
                .any(|i| matches!(i, ChatItem::ToolCall(_))),
            "a tool run follows it, so it reads as a preamble and not a conclusion"
        );
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
