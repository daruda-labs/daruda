//! Implementation of `daruda --hook <eventType> [--harness <name>]`.
//!
//! Lifecycle of one hook invocation:
//! 1. Read the terminal's [`HookRoute`] from the environment. Without one
//!    the agent runs outside a daruda terminal and the hook does nothing.
//! 2. Read JSON payload from stdin (the agent closes the stream once
//!    the JSON is written, so a blocking `read_to_string` returns
//!    quickly).
//! 3. Parse to [`HookEvent`]. Unknown event types or malformed JSON
//!    are silently ignored — the goal is to never block the agent
//!    on a daruda bug.
//! 4. Acquire an exclusive advisory lock on the per-terminal lock file
//!    (`<status_dir>/<terminal_id>.lock`). This serialises concurrent
//!    `daruda --hook` processes for the same terminal so a fast burst
//!    of hooks (`PreToolUse` immediately followed by `PostToolUse`,
//!    say) does not lose updates via read-modify-write race.
//! 5. Read the prior persisted state for this `session_id` (if any).
//! 6. Run the FSM ([`fsm::apply_harness_event`]).
//! 7. Atomically write the terminal's latest event, retaining SessionEnd
//!    until its successor or startup so watcher coalescing loses no run.
//! 8. Release the lock; exit 0 regardless of internal failures
//!    (per official-doc non-blocking convention).

use std::io::{self, Read};

use super::events::HookEvent;
use super::fsm::{self, FsmAction};
use super::route::{Harness, HookRoute};
use super::status_file::lock::SessionLock;
use super::status_file::{self, StatusFile, path_for, read, write_atomic};

/// Run the hook handler. Always returns the exit code that `main`
/// should pass to `std::process::exit` — currently always 0.
pub fn run(event_type: &str, harness: Harness) -> i32 {
    if let Err(err) = run_inner(harness) {
        daruda_store::observability::log_writer::LogWriter::log(
            daruda_store::observability::error_report::ErrorReport::new("Agent hook failed")
                .message(err.to_string())
                .with_context("event", event_type)
                .build(),
        );
    }
    0
}

fn run_inner(harness: Harness) -> Result<(), Box<dyn std::error::Error>> {
    let Some(route) = HookRoute::from_env() else {
        return Ok(());
    };
    let mut payload = String::new();
    io::stdin().read_to_string(&mut payload)?;

    let trimmed = payload.trim();
    if trimmed.is_empty() {
        // Manual `daruda --hook X` invocation with no piped input —
        // no-op rather than error.
        return Ok(());
    }

    let event: HookEvent = match serde_json::from_str(trimmed) {
        Ok(e) => e,
        Err(_) => {
            // Unknown / future event type, or schema drift. Silent
            // skip — daruda only subscribes to a subset of Claude Code's
            // hook events (see [`super::installer::CLAUDE_EVENTS`]).
            return Ok(());
        }
    };

    handle_event(
        &route,
        harness,
        &event,
        super::agent_process::find(harness, &event.common().session_id),
    )
}

fn handle_event(
    route: &HookRoute,
    harness: Harness,
    event: &HookEvent,
    agent_pid: Option<u32>,
) -> Result<(), Box<dyn std::error::Error>> {
    if harness == Harness::Claude && event.common().agent_id.is_some() {
        return Ok(());
    }
    let dir = &route.status_dir;
    std::fs::create_dir_all(dir)?;
    let key = route.terminal.as_str();
    let path = path_for(dir, key);
    let _lock = SessionLock::acquire(dir, key)?;
    let previous = read(&path)?;
    // An old session's queued hook must not overwrite or end its successor.
    if previous
        .as_ref()
        .is_some_and(|file| file.session_id != event.common().session_id)
        && !matches!(event, HookEvent::SessionStart { .. })
    {
        return Ok(());
    }
    if previous
        .as_ref()
        .is_some_and(|file| file.last_event == "SessionEnd")
        && !matches!(event, HookEvent::SessionStart { .. })
    {
        return Ok(());
    }
    let prev_state = previous.map(|file| file.status);
    let new_state = match fsm::apply_harness_event(harness, prev_state, event) {
        FsmAction::Update(status) => status,
        FsmAction::Delete => crate::SessionStatus::Idle,
    };

    let common = event.common();
    let (tool_name, tool_input) = tool_fields(event);
    let file = StatusFile {
        schema_version: status_file::SCHEMA_VERSION,
        session_id: common.session_id.clone(),
        cwd: common.cwd.clone(),
        transcript_path: common.transcript_path.clone(),
        status: new_state,
        last_event: event.name().to_string(),
        tool_name,
        tool_input,
        permission_mode: common.permission_mode,
        notification: notification_type(event),
        timestamp: chrono::Utc::now(),
        terminal_id: Some(route.terminal.clone()),
        harness: Some(harness),
        agent_pid,
    };
    // Retain the end marker until a successor or startup replaces it:
    // a short session can end before the watcher reads its first event.
    let history_result = super::history::observe_locked(dir, &file);
    write_atomic(&path, &file)?;
    history_result?;

    Ok(())
}

/// The notification subtype, present only on `Notification` events.
/// Recorded into the status file so the app-side ingest can gate a
/// desktop push without re-reading the hook payload.
fn notification_type(event: &HookEvent) -> Option<super::events::NotificationType> {
    match event {
        HookEvent::Notification {
            notification_type, ..
        } => Some(*notification_type),
        _ => None,
    }
}

/// Pull `tool_name` + `tool_input` out of the four event variants that
/// carry them. Other variants return `(None, None)`.
fn tool_fields(event: &HookEvent) -> (Option<String>, Option<serde_json::Value>) {
    match event {
        HookEvent::PreToolUse {
            tool_name,
            tool_input,
            ..
        }
        | HookEvent::PostToolUse {
            tool_name,
            tool_input,
            ..
        }
        | HookEvent::PostToolUseFailure {
            tool_name,
            tool_input,
            ..
        }
        | HookEvent::PermissionRequest {
            tool_name,
            tool_input,
            ..
        } => (Some(tool_name.clone()), tool_input.clone()),
        _ => (None, None),
    }
}

#[cfg(test)]
#[path = "handler/tests.rs"]
mod tests;
