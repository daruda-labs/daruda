//! Runtime watchers — long-lived background pumps spawned once at
//! launch. Each pump owns a channel-receiver thread plus a GPUI-side
//! fanout that dispatches into every open Workspace window.
//!
//! Long-lived pumps live here:
//!
//! - **`claude-status`** — `~/.daruda/status/<session>.json`
//!   filesystem watch → `Workspace::apply_claude_status_event`.
//!   Cold restore happens inside `Workspace::new_with_project`;
//!   this pump only delivers subsequent live changes.
//! - **`needs-attention-demote`** — 30 s periodic tick that calls
//!   `cx.notify()` on every open Workspace so stale
//!   `NeedsAttention` demotion (see
//!   `ClaudeStatusStore::aggregate_for_cwd`) surfaces without the
//!   user having to click. 30 s is conservative — the stale
//!   threshold defaults to 60 s, so worst-case lag is ~30 s.
//! - **`status-pulse`** — app-global animation clock for status badges
//!   and agent-chat busy rows.
//! - **`deferred-telegram-flush`** — 15 s periodic tick that delivers
//!   Telegram pings held while the user was present once presence drops.
//! - **`panels-reload`** — filesystem watch on
//!   `~/.config/daruda/projects/<…>/panels.json` →
//!   `Workspace::apply_panels_reload`. Self-write suppression in
//!   `apply_panels_reload` (compares serialized JSON, skips notify
//!   when disk matches the in-memory state) prevents daruda's own
//!   atomic-rename saves from looping.
//!
//! Config-reload lives in `crate::settings_store::spawn_file_watch`,
//! not here. Workspaces pick up edits via their
//! `cx.observe_global::<SettingsStore>` subscription; the app-wide
//! theme / keybinding / appearance side effects fan out from
//! `globals::register_settings_observer`.

use crate::ui::StatusPulseClock;
use crate::ui::theme;
use crate::window_registry::WindowRegistry;
use crate::{hooks, panels_watcher, watcher_pumps};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use gpui::App;

/// App-global watchers (status, panels, …) never stop, so their RAII
/// [`crate::dir_watch::DirWatcher`] handles are parked here for the process
/// lifetime instead of being dropped (dropping would end the watch). Populated
/// once at launch from `spawn_all`; never cleared.
static APP_WATCHERS: std::sync::OnceLock<std::sync::Mutex<Vec<crate::dir_watch::DirWatcher>>> =
    std::sync::OnceLock::new();

/// Park an app-global watcher handle for the process lifetime.
fn retain_app_watcher(watcher: crate::dir_watch::DirWatcher) {
    if let Ok(mut v) = APP_WATCHERS
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
    {
        v.push(watcher);
    }
}

pub(crate) fn spawn_all(cx: &mut App) {
    spawn_claude_status(cx);
    spawn_needs_attention_demote(cx);
    spawn_status_pulse(cx);
    spawn_deferred_telegram_flush(cx);
    spawn_panels_reload(cx);
}

fn spawn_claude_status(cx: &mut App) {
    match daruda_claude::hooks::status_file::default_dir() {
        Err(e) => {
            LogWriter::log(
                ErrorReport::new("Claude status watcher disabled — could not resolve status dir")
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .at(file!(), line!())
                    .dedup("claude.status.dir")
                    .build(),
            );
        }
        Ok(status_dir) => {
            let (status_rx, watcher) = hooks::watcher::spawn_status_watcher(status_dir);
            retain_app_watcher(watcher);
            watcher_pumps::spawn_event_fanout_pump(
                status_rx,
                std::time::Duration::from_millis(100),
                |event, ws, _window, cx| {
                    ws.apply_claude_status_event(event, cx);
                },
                cx,
            );
        }
    }
}

fn spawn_needs_attention_demote(cx: &mut App) {
    watcher_pumps::spawn_periodic_pump(
        std::time::Duration::from_secs(30),
        |cx: &mut App| {
            WindowRegistry::for_each_workspace(cx, |_ws, _window, cx| {
                cx.notify();
            });
        },
        cx,
    );
}

/// Deliver presence-deferred Telegram pings shortly after the user leaves the
/// window. Cheap when idle: `flush_deferred_telegram` returns immediately while
/// each workspace's deferred map is empty. 15s cadence bounds the delay from
/// "user stepped away" to "phone buzzes".
fn spawn_deferred_telegram_flush(cx: &mut App) {
    watcher_pumps::spawn_periodic_pump(
        std::time::Duration::from_secs(15),
        |cx: &mut App| {
            WindowRegistry::for_each_workspace(cx, |ws, _window, cx| {
                ws.flush_deferred_telegram(cx);
            });
        },
        cx,
    );
}

/// Drive the shared status-badge animation clock. Advances one
/// `StatusPulseClock` tick every `STATUS_INDICATOR_TICK_MS` (~4 fps) and
/// repaints every window that has an animating Claude session —
/// backgrounded windows included, so a session in another window keeps
/// pulsing. Windows with no animating session stay at zero redraws.
/// Replaces per-badge `with_animation` (which repainted the whole
/// window ~60×/s). See `ui::agent_status_badge` and root `CLAUDE.md`
/// Pitfall #10.
fn spawn_status_pulse(cx: &mut App) {
    watcher_pumps::spawn_periodic_pump(
        std::time::Duration::from_millis(theme::STATUS_INDICATOR_TICK_MS),
        |cx: &mut App| {
            if cx.try_global::<StatusPulseClock>().is_none() {
                cx.set_global(StatusPulseClock::default());
            }
            {
                let clock = cx.global_mut::<StatusPulseClock>();
                clock.tick = clock.tick.wrapping_add(1);
            }
            WindowRegistry::for_each_workspace(cx, |ws, _window, cx| {
                if ws.has_animating_claude_status(cx) {
                    cx.notify();
                    // Right and left docks are `.cached()`; animated
                    // status badges live in both (Tasks tab in the right
                    // dock, per-lane AgentStatusBadge in the left dock),
                    // so the pulse must dirty both dock entities or they
                    // freeze (Pitfall #10).
                    ws.notify_right_dock(cx);
                    ws.notify_left_dock(cx);
                }
                // AgentChat panes are separately-cached main-area entities, so
                // their in-flight rollup dot / working row needs its own dirty —
                // plus one trailing frame when a pane settles, or its last
                // "running" frame freezes (see `pulse_agent_chats`).
                ws.pulse_agent_chats(cx);
            });
        },
        cx,
    );
}

fn spawn_panels_reload(cx: &mut App) {
    let (panels_reload_rx, watcher) = panels_watcher::spawn_panels_watcher();
    retain_app_watcher(watcher);
    watcher_pumps::spawn_drain_burst_pump(
        panels_reload_rx,
        std::time::Duration::from_millis(250),
        |cx: &mut App| {
            WindowRegistry::for_each_workspace(cx, |ws, _window, cx| {
                ws.apply_panels_reload(cx);
            });
        },
        cx,
    );
}
