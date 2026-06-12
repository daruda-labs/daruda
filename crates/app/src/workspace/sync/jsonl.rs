//! JSONL event pump — bridges [`crate::hooks::jsonl_watcher`] into
//! the GPUI Workspace.
//!
//! The watcher runs whenever `claude_status_enabled` is true,
//! regardless of hook installation status. On each `JsonlEvent`, we
//! synthesize a [`StatusFile`] with `source = Source::Jsonl` and feed
//! it through the same store as hook events. The store's race policy
//! The hook-wins race policy ensures hook entries win over JSONL on
//! ties, so when both channels are live the user sees the authoritative path.
//!
//! Mirrors the shape of [`super::pty_pump`] — drain every queued
//! event per 100 ms tick so a burst from a multi-session lane
//! doesn't lag visible state.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use daruda_claude::hooks::status_file::{Source, StatusFile};
use gpui::{Context, Task};

use crate::hooks::jsonl_watcher::{self, JsonlEvent, JsonlWatcherHandle};
use crate::workspace::Workspace;

/// 100 ms — same as `pty_pump`. Short enough that JSONL-only updates
/// (Claude Code spawned outside daruda's PTY ancestry) feel responsive,
/// long enough that no-events ticks aren't expensive.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Spawn the long-lived task that pulls `JsonlEvent`s from `events`
/// and applies them to the Workspace via
/// [`Workspace::apply_claude_jsonl_event`]. The returned `Task<()>`
/// is held by Workspace as `_jsonl_pump`; dropping it closes the
/// receiver, which lets the watcher thread (held alive by
/// `_jsonl_shutdown`) exit when its shutdown sender drops.
pub(in crate::workspace) fn spawn(
    events: Receiver<JsonlEvent>,
    cx: &mut Context<Workspace>,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        'outer: loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            loop {
                let ev = match events.try_recv() {
                    Ok(e) => e,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break 'outer,
                };
                if this
                    .update(cx, |ws, cx| ws.apply_claude_jsonl_event(ev, cx))
                    .is_err()
                {
                    break 'outer;
                }
            }
        }
    })
}

impl Workspace {
    /// Convert one `JsonlEvent` into a `StatusFile(source = Jsonl)`
    /// and feed it to the store, applying the hook-wins race policy.
    /// Notifies only when something changed.
    pub fn apply_claude_jsonl_event(&mut self, event: JsonlEvent, cx: &mut Context<Self>) {
        let JsonlEvent {
            session_id,
            cwd,
            jsonl_path,
            status,
            timestamp,
        } = event;

        let file = StatusFile {
            schema_version: daruda_claude::hooks::status_file::SCHEMA_VERSION,
            session_id,
            cwd,
            transcript_path: Some(jsonl_path),
            status,
            last_event: jsonl_pseudo_event_label(),
            tool_name: None,
            tool_input: None,
            permission_mode: None,
            notification: None,
            timestamp,
            source: Source::Jsonl,
        };
        #[cfg(debug_assertions)]
        let dbg_probe = self.probe_lane_status(&file.session_id);
        #[cfg(debug_assertions)]
        let dbg_fields = (
            file.session_id.clone(),
            file.cwd.clone(),
            file.last_event.clone(),
            file.source,
        );
        if self.claude.claude_status.update(file) {
            #[cfg(debug_assertions)]
            {
                let (sid, cwd, event, source) = dbg_fields;
                self.log_lane_status_change(dbg_probe, &sid, &cwd, &event, source);
            }
            cx.notify();
            self.notify_right_dock(cx);
            self.notify_left_dock(cx);
        }
    }
}

/// Pseudo `last_event` label for JSONL-source entries. Hook entries
/// carry the literal Claude hook name (`PreToolUse`, `Stop`, …); on
/// the fallback path we don't know the underlying event so we mark
/// it explicitly so debug logs can tell the channel apart.
fn jsonl_pseudo_event_label() -> String {
    "jsonl-fallback".to_string()
}

impl Workspace {
    /// (Re)evaluate whether the JSONL watcher should be running for
    /// this Workspace, and (re)spawn it with the current lane set.
    ///
    /// Engaged when **both** are true:
    /// - `claude_status_enabled` (config opt-in)
    /// - resolvable `dirs::home_dir()` (sanity check)
    ///
    /// The JSONL watcher runs regardless of hook installation status.
    /// When hooks also deliver data, the store's race policy
    /// (`should_replace`) ensures hook data takes precedence over JSONL.
    ///
    /// Otherwise the watcher is dropped (which closes its shutdown
    /// channel and lets the FSEvents subscription unregister).
    ///
    /// Call from any site that changes one of those inputs:
    /// initial construction, lane create / remove, and
    /// `apply_config` when `claude_status.enable` flips.
    pub fn refresh_jsonl_watcher(&mut self, cx: &mut Context<Self>) {
        // Drop any existing shutdown sender + pump first so the old
        // watcher thread exits before the new one starts subscribing
        // to the same FSEvents paths. Otherwise both would emit
        // duplicate events during the overlap window.
        self.claude._jsonl_watcher_shutdown = None;
        self.claude._jsonl_event_pump = None;

        let should_run = self.claude.claude_status_enabled;
        if !should_run {
            return;
        }
        let Some(home) = dirs::home_dir() else {
            return;
        };
        let pairs: Vec<(PathBuf, PathBuf)> = self
            .active_lanes()
            .iter()
            .map(|wt| {
                (
                    wt.path.clone(),
                    jsonl_watcher::project_dir_for(&home, &wt.path),
                )
            })
            .collect();
        if pairs.is_empty() {
            return;
        }

        let JsonlWatcherHandle {
            shutdown_tx,
            events,
        } = jsonl_watcher::spawn(pairs);
        let pump = spawn(events, cx);
        self.claude._jsonl_watcher_shutdown = Some(shutdown_tx);
        self.claude._jsonl_event_pump = Some(pump);
    }
}
