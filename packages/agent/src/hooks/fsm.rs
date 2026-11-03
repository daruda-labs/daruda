//! Hook event → [`SessionStatus`] transition.
//!
//! Pure function.

use crate::SessionStatus;
use crate::hooks::events::HookEvent;
use crate::hooks::route::Harness;

/// The codex tool that blocks on the user's answer to a planning question.
const CODEX_USER_INPUT_TOOL: &str = "request_user_input";

/// Outcome of applying a hook event to a session's prior state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FsmAction {
    /// Replace the persisted state with this new value.
    Update(SessionStatus),
    /// Session ended — consumers remove live state. The handler retains
    /// an end marker so a short-lived session can still be attached to a run.
    Delete,
}

/// Apply a hook event from `harness` to the prior state. codex has no
/// `PermissionRequest`; its question tool is what waits on the user.
pub fn apply_harness_event(
    harness: Harness,
    prev: Option<SessionStatus>,
    event: &HookEvent,
) -> FsmAction {
    match (harness, event) {
        (Harness::Codex, HookEvent::PreToolUse { tool_name, .. })
            if tool_name == CODEX_USER_INPUT_TOOL =>
        {
            FsmAction::Update(SessionStatus::NeedsAttention)
        }
        _ => apply_event(prev, event),
    }
}

/// Apply a hook event to the prior state.
///
/// `prev` is `None` when this is the first event ever seen for the
/// session; the FSM's behaviour is independent of the prior state for
/// every event except notifications of unhandled subtypes.
pub fn apply_event(prev: Option<SessionStatus>, event: &HookEvent) -> FsmAction {
    use HookEvent as E;

    let new_state = match event {
        // SessionStart sets a fresh Idle (the session is up and ready).
        // The Connecting state is reserved for "no events yet" or
        // "TTL stale reset" in the consumer; the FSM never returns it
        // because every hook event is concrete activity.
        E::SessionStart { .. } => SessionStatus::Idle,

        // SessionEnd removes the live session rather than transitioning it.
        E::SessionEnd { .. } => return FsmAction::Delete,

        // User submitted a prompt — Claude starts generating a response.
        E::UserPromptSubmit { .. } => SessionStatus::Working,

        // Tool is actively executing.
        E::PreToolUse { .. } => SessionStatus::ExecutingTool,

        // Tool finished (success or failure) — Claude resumes responding.
        E::PostToolUse { .. } | E::PostToolUseFailure { .. } => SessionStatus::Working,

        // Explicit user-attention signal.
        E::PermissionRequest { .. } => SessionStatus::NeedsAttention,

        // Notifications never move the indicator. Blocking subtypes
        // (permission / idle / elicitation) surface as a transient
        // desktop push fired app-side on ingest — not a persistent
        // `NeedsAttention` latch, which would keep blinking long after
        // the user has seen it. Informational subtypes are inert. Either
        // way the persisted status stays whatever the turn FSM last set.
        // The only event that still latches `NeedsAttention` is the
        // explicit `PermissionRequest` above.
        E::Notification { .. } => prev.unwrap_or(SessionStatus::Idle),

        // Turn ended — clean finish or API-error failure both return the
        // session to Idle (the turn is over either way).
        E::Stop { .. } | E::StopFailure { .. } => SessionStatus::Idle,

        // An interrupted turn is over; it neither completed nor failed.
        E::Interrupt { .. } => SessionStatus::Idle,
    };

    FsmAction::Update(new_state)
}
