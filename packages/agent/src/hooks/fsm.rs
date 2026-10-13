//! Hook event → [`SessionStatus`] transition.
//!
//! Pure function.

use crate::SessionStatus;
use crate::hooks::events::HookEvent;

/// Outcome of applying a hook event to a session's prior state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FsmAction {
    /// Replace the persisted state with this new value.
    Update(SessionStatus),
    /// Session ended — caller should remove the status file entirely.
    Delete,
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

        // SessionEnd removes the file rather than transitioning state.
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
    };

    FsmAction::Update(new_state)
}
