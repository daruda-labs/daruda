//! Implementation of `daruda --hook <eventType>`.
//!
//! Lifecycle of one hook invocation:
//! 1. Read JSON payload from stdin (Claude Code closes the stream once
//!    the JSON is written, so a blocking `read_to_string` returns
//!    quickly).
//! 2. Parse to [`HookEvent`]. Unknown event types or malformed JSON
//!    are silently ignored — the goal is to never block Claude Code
//!    on a daruda bug.
//! 3. Acquire an exclusive advisory lock on the per-session lock file
//!    (`<status_dir>/<session_id>.lock`). This serialises concurrent
//!    `daruda --hook` processes for the same session so a fast burst
//!    of hooks (`PreToolUse` immediately followed by `PostToolUse`,
//!    say) does not lose updates via read-modify-write race.
//! 4. Read the prior persisted state for this `session_id` (if any).
//! 5. Run the FSM ([`fsm::apply_event`]).
//! 6. Atomically write or delete the status file.
//! 7. Release the lock; exit 0 regardless of internal failures
//!    (per official-doc non-blocking convention).

use std::fs::OpenOptions;
use std::io::{self, Read};
use std::path::Path;

use fs4::fs_std::FileExt;

use daruda_claude::hooks::events::HookEvent;
use daruda_claude::hooks::fsm::{self, FsmAction};
use daruda_claude::hooks::status_file::{
    self, Source, StatusFile, default_dir, delete, lock_path_for, path_for, read, write_atomic,
};

/// Run the hook handler. Always returns the exit code that `main`
/// should pass to `std::process::exit` — currently always 0.
pub fn run(event_type: &str) -> i32 {
    if let Err(err) = run_inner(event_type) {
        eprintln!("daruda --hook {event_type}: {err}");
    }
    0
}

fn run_inner(_event_type: &str) -> Result<(), Box<dyn std::error::Error>> {
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
            // skip — daruda only subscribes to 9 of 28 events anyway.
            return Ok(());
        }
    };

    let dir = default_dir()?;
    std::fs::create_dir_all(&dir)?;
    let session_id = &event.common().session_id;
    let path = path_for(&dir, session_id);

    // Acquire a per-session advisory lock for the read-modify-write
    // window. Lock file is dropped (and lock released) at end of scope.
    let _lock = SessionLock::acquire(&dir, session_id)?;

    // Prior state — None if first event for this session.
    let prev_state = read(&path)?.map(|f| f.status);

    match fsm::apply_event(prev_state, &event) {
        FsmAction::Update(new_state) => {
            let common = event.common();
            let (tool_name, tool_input) = tool_fields(&event);
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
                notification: notification_type(&event),
                timestamp: chrono::Utc::now(),
                source: Source::Hook,
            };
            write_atomic(&path, &file)?;
        }
        FsmAction::Delete => {
            delete(&path)?;
            // SessionEnd: the lock file is no longer needed. We can't
            // delete it here because the current handler still holds
            // the lock — it'd race with another `daruda --hook`
            // happening to come in for this same session_id during
            // shutdown. The lock file dropping logic lives in
            // `cold_restore::run` (TTL sweep on the next daruda
            // startup) which now also targets `*.lock` files older
            // than the configured TTL.
        }
    }

    Ok(())
}

/// Per-session exclusive advisory lock. Holds an open file handle on
/// `<dir>/<session_id>.lock`; the lock is released when the handle is
/// dropped (RAII).
struct SessionLock {
    file: std::fs::File,
}

impl SessionLock {
    fn acquire(dir: &Path, session_id: &str) -> std::io::Result<Self> {
        let path = lock_path_for(dir, session_id);
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)?;
        FileExt::lock_exclusive(&file)?;
        Ok(Self { file })
    }
}

impl Drop for SessionLock {
    fn drop(&mut self) {
        // Best-effort unlock; release happens automatically on close
        // anyway, so an error here is informational.
        let _ = FileExt::unlock(&self.file);
    }
}

/// The notification subtype, present only on `Notification` events.
/// Recorded into the status file so the app-side ingest can gate a
/// desktop push without re-reading the hook payload.
fn notification_type(event: &HookEvent) -> Option<daruda_claude::hooks::events::NotificationType> {
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
