use super::*;
use crate::remote_channel::bridge::PaneRef;
use crate::telegram::command::tests::{listing_of, pane};
#[test]
fn start_handles_do_not_change_phone_confirmation_text() {
    let lane = crate::control::result::LaneHandle {
        workspace: pane(1).workspace,
        project: 1,
        lane: 2,
    };
    let cases = [
        (
            ControlResult::FlowStarting {
                handle: "flow:run-1".parse().unwrap(),
                name: "ship.yaml".into(),
                origin: crate::control::result::FlowOriginKind::Repo,
                lane,
            },
            s::control::flow_starting("ship.yaml"),
        ),
        (
            ControlResult::TaskStarted {
                handle: "task:task-1/exec-1".parse().unwrap(),
                task: "task-1".into(),
                lane,
                chat: None,
            },
            s::control::task_started(),
        ),
    ];
    for (result, expected) in cases {
        assert_eq!(
            render(&Ok(result), Absorbed::Nothing, &CommandState::default()).text,
            expected
        );
    }
}

/// The phone reads the coded reason in its own language, with the agent
/// the flow names — never the raw code an MCP caller branches on.
#[test]
fn a_refused_flow_start_names_its_reason() {
    let rendered = render_error(&ControlError::FlowNotStarted {
        name: "ship.yaml".into(),
        reason: FlowNotStartedReason::SessionHostUnusable {
            agent: "claude".into(),
        },
    });
    assert_eq!(
        rendered,
        s::control::error_flow_not_started("ship.yaml", s::flow::session_host_unusable("claude"))
    );
    assert!(!rendered.contains("session_host_unusable"), "{rendered}");
    assert_eq!(
        render_error(&ControlError::FlowNotStarted {
            name: "ship.yaml".into(),
            reason: FlowNotStartedReason::LaneUnresolvable,
        }),
        s::control::error_flow_not_started(
            "ship.yaml",
            s::control::flow_reason_lane_unresolvable()
        )
    );
}

#[test]
fn stale_run_errors_are_localized() {
    assert_eq!(
        render_error(&ControlError::RunMismatch),
        s::control::error_run_mismatch()
    );
}

#[test]
fn a_long_listing_is_truncated_under_the_telegram_limit() {
    let chats: Vec<PaneRef> = (0..400).map(pane).collect();
    let mut state = CommandState::default();
    let listing = listing_of(&chats);
    state.record_listing(&listing);
    let rendered = render(
        &Ok(ControlResult::Listing(listing)),
        Absorbed::Nothing,
        &state,
    );
    assert!(
        rendered.text.len() <= REPLY_MAX_BYTES,
        "rendered {} bytes",
        rendered.text.len()
    );
    let shown = rendered.text.lines().count() - 2; // header + the omitted line
    assert!(shown > 0 && shown < 400, "some rows shown, not all");
    assert!(
        rendered
            .text
            .contains(&s::control::listing_omitted((400 - shown) as u32)),
        "the omitted count must be visible: {}",
        rendered.text
    );
}

#[test]
fn listing_buttons_are_folded_into_rows_and_capped() {
    let chats: Vec<PaneRef> = (0..40).map(pane).collect();
    let mut state = CommandState::default();
    let listing = listing_of(&chats);
    state.record_listing(&listing);
    let keyboard = render(
        &Ok(ControlResult::Listing(listing)),
        Absorbed::Nothing,
        &state,
    )
    .keyboard
    .expect("buttons for a non-empty listing");
    let total: usize = keyboard.rows.iter().map(Vec::len).sum();
    assert_eq!(total, LISTING_BUTTON_MAX);
    assert!(keyboard.rows.iter().all(|r| r.len() <= BUTTONS_PER_ROW));
}

/// A tab the user named reaches the phone beside the agent, and is what a
/// selection is confirmed by — two chats in one worktree read alike
/// without it.
#[test]
fn a_named_tab_is_carried_beside_the_agent() {
    let mut summary = ChatSummary {
        target: PaneRef {
            workspace: Default::default(),
            pane: 1,
        },
        agent: "claude".into(),
        agent_name: "Claude Code".into(),
        is_active_lane: true,
        activity: Activity::Idle,
        health: Health::Ok,
        unread: false,
        title: Some("session title".into()),
        last_activity: None,
        tab_name: None,
    };
    assert_eq!(agent_label(&summary), "Claude Code");
    assert_eq!(bare_label(&summary), "session title");
    summary.tab_name = Some("review".into());
    assert_eq!(agent_label(&summary), "Claude Code · review");
    assert_eq!(bare_label(&summary), "review");
}

/// A row names the agent running the pane — the one thing that decides
/// which chat a person wants, and which no other field carries. The glyph
/// column is drawn only for a pane that has a session: after a restore a
/// dormant pane is most of the list, so marking it would say nothing
/// about any one row.
#[test]
fn a_row_names_its_agent_and_marks_only_a_pane_with_a_session() {
    use crate::control::result::{LaneGroup, ProjectGroup, WindowGroup};

    let chat =
        |pane_id, agent: &str, name: &str, health, activity, title: Option<&str>| ChatSummary {
            target: PaneRef {
                workspace: Default::default(),
                pane: pane_id,
            },
            agent: agent.into(),
            agent_name: name.into(),
            is_active_lane: true,
            activity,
            health,
            unread: false,
            title: title.map(str::to_owned),
            last_activity: None,
            tab_name: None,
        };
    let listing = Listing {
        windows: vec![WindowGroup {
            index: 0,
            projects: vec![ProjectGroup {
                name: "daruda".into(),
                lanes: vec![LaneGroup {
                    name: "main".into(),
                    chats: vec![
                        chat(
                            1,
                            "codex-acp",
                            "Codex",
                            Health::Unavailable,
                            Activity::Idle,
                            None,
                        ),
                        chat(
                            2,
                            "claude",
                            "Claude Code",
                            Health::Ok,
                            Activity::Working,
                            None,
                        ),
                        chat(
                            3,
                            "claude",
                            "Claude Code",
                            Health::Ok,
                            Activity::Idle,
                            Some("notihub routing"),
                        ),
                    ],
                }],
            }],
        }],
        omitted: 0,
    };
    let mut state = CommandState::default();
    state.record_listing(&listing);
    let text = render(
        &Ok(ControlResult::Listing(listing)),
        Absorbed::Nothing,
        &state,
    )
    .text;

    // Every pane here sits in the active worktree, and no row says so —
    // that axis is still reported to an agent reading the JSON, it just
    // no longer earns room on a phone screen. The three rows are the
    // three shapes the dash segment takes: nothing to report, a badge
    // with no title behind it, and both.
    let rows: Vec<&str> = text.lines().skip(1).collect();
    assert_eq!(rows[0], "1. daruda/main (Codex)");
    assert_eq!(
        rows[1],
        format!(
            "2. daruda/main (Claude Code) — [{}]",
            s::control::state_working()
        )
    );
    assert_eq!(
        rows[2],
        format!(
            "3. daruda/main (Claude Code) — [{}] notihub routing",
            s::control::state_idle()
        )
    );
}

#[test]
fn an_empty_listing_says_so_and_offers_no_buttons() {
    let state = CommandState::default();
    let rendered = render(
        &Ok(ControlResult::Listing(listing_of(&[]))),
        Absorbed::Nothing,
        &state,
    );
    assert_eq!(rendered.text, s::control::listing_empty());
    assert!(rendered.keyboard.is_none());
}
#[test]
fn every_ask_disposition_is_worded_differently() {
    let state = CommandState::default();
    let texts: Vec<String> = [
        AskDisposition::Connecting,
        AskDisposition::Sent,
        AskDisposition::Queued,
        AskDisposition::HandledLocally,
    ]
    .into_iter()
    .map(|disposition| {
        let reply = render(
            &Ok(ControlResult::Accepted { disposition }),
            Absorbed::Nothing,
            &state,
        );
        assert!(reply.keyboard.is_none());
        reply.text
    })
    .collect();
    assert!(texts.iter().all(|t| !t.is_empty()));
    assert_eq!(
        texts.iter().collect::<std::collections::HashSet<_>>().len(),
        4,
        "{texts:?}"
    );
}

/// Three refusals with three different fixes, so three different
/// sentences — a shared one would send the user to the wrong place.
#[test]
fn each_orchestrator_refusal_says_something_different() {
    let state = CommandState::default();
    let texts: Vec<String> = [
        ControlError::OrchestratorDisabled,
        ControlError::OrchestratorUnresolvable,
        ControlError::OrchestratorUnavailable,
    ]
    .into_iter()
    .map(|e| render(&Err(e), Absorbed::Nothing, &state).text)
    .collect();
    assert!(texts.iter().all(|t| !t.is_empty()));
    assert_eq!(
        texts.iter().collect::<std::collections::HashSet<_>>().len(),
        3,
        "{texts:?}"
    );
}

/// `/daruda` with no text names its own usage line, not `/use`'s.
#[test]
fn a_bare_daruda_is_answered_with_its_own_usage() {
    let rendered = render_parse_error(&ParseError::MissingArgument { command: "daruda" });
    assert_eq!(
        rendered.text,
        s::control::error_missing_argument(s::control::usage_daruda())
    );
}

#[test]
fn a_typo_is_answered_with_the_suggestion() {
    let rendered = render_parse_error(&ParseError::Unknown {
        input: "lst".into(),
        suggestion: Some("list"),
    });
    assert_eq!(
        rendered.text,
        s::control::error_unknown_command_did_you_mean("lst", "list")
    );
}
