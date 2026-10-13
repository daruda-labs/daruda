//! Tests for the public `daruda_agent::hooks::fsm` API.

use daruda_agent::SessionStatus;
use daruda_agent::hooks::events::{
    CommonFields, HookEvent, NotificationType, PermissionMode, SessionEndReason, SessionStartSource,
};
use daruda_agent::hooks::fsm::*;
use std::path::PathBuf;

fn cf() -> CommonFields {
    CommonFields {
        session_id: "s1".into(),
        cwd: PathBuf::from("/tmp/x"),
        transcript_path: None,
        permission_mode: Some(PermissionMode::Default),
        agent_id: None,
        agent_type: None,
    }
}

fn sess_start() -> HookEvent {
    HookEvent::SessionStart {
        common: cf(),
        source: SessionStartSource::Startup,
        model: None,
    }
}
fn sess_end() -> HookEvent {
    HookEvent::SessionEnd {
        common: cf(),
        end_reason: SessionEndReason::Logout,
    }
}
fn user_prompt() -> HookEvent {
    HookEvent::UserPromptSubmit {
        common: cf(),
        prompt: None,
    }
}
fn pre_tool() -> HookEvent {
    HookEvent::PreToolUse {
        common: cf(),
        tool_name: "Bash".into(),
        tool_input: None,
        tool_use_id: None,
    }
}
fn post_tool() -> HookEvent {
    HookEvent::PostToolUse {
        common: cf(),
        tool_name: "Bash".into(),
        tool_input: None,
        tool_output: None,
        tool_use_id: None,
    }
}
fn post_tool_fail() -> HookEvent {
    HookEvent::PostToolUseFailure {
        common: cf(),
        tool_name: "Bash".into(),
        tool_input: None,
        tool_output: None,
        tool_use_id: None,
    }
}
fn perm_req() -> HookEvent {
    HookEvent::PermissionRequest {
        common: cf(),
        tool_name: "Bash".into(),
        tool_input: None,
    }
}
fn notif(t: NotificationType) -> HookEvent {
    HookEvent::Notification {
        common: cf(),
        notification_type: t,
    }
}
fn stop() -> HookEvent {
    HookEvent::Stop {
        common: cf(),
        response: None,
    }
}
fn stop_failure() -> HookEvent {
    HookEvent::StopFailure { common: cf() }
}

#[test]
fn session_start_yields_idle() {
    assert_eq!(
        apply_event(None, &sess_start()),
        FsmAction::Update(SessionStatus::Idle)
    );
    // Independent of prior state.
    assert_eq!(
        apply_event(Some(SessionStatus::Working), &sess_start()),
        FsmAction::Update(SessionStatus::Idle)
    );
}

#[test]
fn session_end_yields_delete() {
    assert_eq!(apply_event(None, &sess_end()), FsmAction::Delete);
    assert_eq!(
        apply_event(Some(SessionStatus::Working), &sess_end()),
        FsmAction::Delete
    );
}

#[test]
fn user_prompt_submit_yields_working() {
    for prev in [
        None,
        Some(SessionStatus::Idle),
        Some(SessionStatus::NeedsAttention),
        Some(SessionStatus::Connecting),
    ] {
        assert_eq!(
            apply_event(prev, &user_prompt()),
            FsmAction::Update(SessionStatus::Working),
            "prev={:?}",
            prev
        );
    }
}

#[test]
fn pre_tool_yields_executing_tool() {
    for prev in [
        None,
        Some(SessionStatus::Working),
        Some(SessionStatus::NeedsAttention),
    ] {
        assert_eq!(
            apply_event(prev, &pre_tool()),
            FsmAction::Update(SessionStatus::ExecutingTool),
            "prev={prev:?}"
        );
    }
}

#[test]
fn post_tool_yields_working() {
    for ev in [post_tool(), post_tool_fail()] {
        assert_eq!(
            apply_event(Some(SessionStatus::ExecutingTool), &ev),
            FsmAction::Update(SessionStatus::Working),
            "{ev:?}"
        );
    }
}

#[test]
fn permission_request_yields_needs_attention() {
    assert_eq!(
        apply_event(Some(SessionStatus::Working), &perm_req()),
        FsmAction::Update(SessionStatus::NeedsAttention)
    );
}

#[test]
fn notification_never_changes_state() {
    // Every subtype — blocking and informational alike — leaves the
    // persisted status untouched. Blocking ones (permission / idle /
    // elicitation) are surfaced as a transient desktop push on the
    // app side instead of latching `NeedsAttention`.
    for t in [
        NotificationType::PermissionPrompt,
        NotificationType::IdlePrompt,
        NotificationType::ElicitationDialog,
        NotificationType::AuthSuccess,
        NotificationType::ElicitationComplete,
        NotificationType::ElicitationResponse,
        NotificationType::Unknown,
    ] {
        // Prev = Working → stays Working.
        assert_eq!(
            apply_event(Some(SessionStatus::Working), &notif(t)),
            FsmAction::Update(SessionStatus::Working),
            "{t:?}"
        );
        // Prev = Idle → stays Idle (no idle-prompt red latch).
        assert_eq!(
            apply_event(Some(SessionStatus::Idle), &notif(t)),
            FsmAction::Update(SessionStatus::Idle),
            "{t:?}"
        );
        // Prev = NeedsAttention (latched by a real PermissionRequest)
        // → stays NeedsAttention. The paired PermissionPrompt
        // notification Claude sends alongside must not clear it.
        assert_eq!(
            apply_event(Some(SessionStatus::NeedsAttention), &notif(t)),
            FsmAction::Update(SessionStatus::NeedsAttention),
            "{t:?}"
        );
        // No prev → fall back to Idle.
        assert_eq!(
            apply_event(None, &notif(t)),
            FsmAction::Update(SessionStatus::Idle),
            "{t:?}"
        );
    }
}

#[test]
fn stop_yields_idle() {
    assert_eq!(
        apply_event(Some(SessionStatus::Working), &stop()),
        FsmAction::Update(SessionStatus::Idle)
    );
}

#[test]
fn stop_failure_yields_idle() {
    // A turn that ends due to an API error (`StopFailure`) must still
    // drop the indicator to Idle, exactly like a clean `Stop`.
    // Without this the badge stays stuck on Working / ExecutingTool:
    // in hook-only mode the JSONL fallback is off, so nothing else
    // ages the state out until the next turn completes normally.
    for prev in [
        Some(SessionStatus::Working),
        Some(SessionStatus::ExecutingTool),
    ] {
        assert_eq!(
            apply_event(prev, &stop_failure()),
            FsmAction::Update(SessionStatus::Idle),
            "prev={prev:?}"
        );
    }
}

/// Realistic full-turn sequence.
#[test]
fn full_turn_sequence() {
    // 1. Session boots up.
    let s = match apply_event(None, &sess_start()) {
        FsmAction::Update(s) => s,
        _ => panic!(),
    };
    assert_eq!(s, SessionStatus::Idle);

    // 2. User submits a prompt.
    let s = match apply_event(Some(s), &user_prompt()) {
        FsmAction::Update(s) => s,
        _ => panic!(),
    };
    assert_eq!(s, SessionStatus::Working);

    // 3. Claude wants to run a tool that needs permission.
    let s = match apply_event(Some(s), &perm_req()) {
        FsmAction::Update(s) => s,
        _ => panic!(),
    };
    assert_eq!(s, SessionStatus::NeedsAttention);

    // 4. User approves; claude runs the tool.
    let s = match apply_event(Some(s), &pre_tool()) {
        FsmAction::Update(s) => s,
        _ => panic!(),
    };
    assert_eq!(s, SessionStatus::ExecutingTool);

    // 5. Tool completes — back to generating response.
    let s = match apply_event(Some(s), &post_tool()) {
        FsmAction::Update(s) => s,
        _ => panic!(),
    };
    assert_eq!(s, SessionStatus::Working);

    // 6. Turn ends.
    let s = match apply_event(Some(s), &stop()) {
        FsmAction::Update(s) => s,
        _ => panic!(),
    };
    assert_eq!(s, SessionStatus::Idle);

    // 7. User logs out.
    assert_eq!(apply_event(Some(s), &sess_end()), FsmAction::Delete);
}
