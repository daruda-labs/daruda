//! Runtime watchers — long-lived background pumps spawned once at
//! launch. Each pump owns a channel-receiver thread plus a GPUI-side
//! fanout that dispatches into every open Workspace window.
//!
//! Three pumps live here:
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

pub(crate) fn spawn_all(cx: &mut App) {
    spawn_claude_status(cx);
    spawn_needs_attention_demote(cx);
    spawn_status_pulse(cx);
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
            let status_rx = hooks::watcher::spawn_status_watcher(status_dir);
            watcher_pumps::spawn_event_fanout_pump(
                "claude-status",
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
        "needs-attention-demote",
        std::time::Duration::from_secs(30),
        |cx: &mut App| {
            WindowRegistry::for_each_workspace(cx, |_ws, _window, cx| {
                cx.notify();
            });
        },
        cx,
    );
}

/// Drive the shared status-badge animation clock. Advances one
/// `StatusPulseClock` tick every `STATUS_INDICATOR_TICK_MS` (~6 fps) and
/// repaints only active windows that have an animating Claude session —
/// idle/backgrounded windows stay at zero redraws. Replaces per-badge
/// `with_animation` (which repainted the whole window ~60×/s). See
/// `ui::agent_status_badge` and root `CLAUDE.md` Pitfall #10.
fn spawn_status_pulse(cx: &mut App) {
    watcher_pumps::spawn_periodic_pump(
        "status-pulse",
        std::time::Duration::from_millis(theme::STATUS_INDICATOR_TICK_MS),
        |cx: &mut App| {
            if cx.try_global::<StatusPulseClock>().is_none() {
                cx.set_global(StatusPulseClock::default());
            }
            {
                let clock = cx.global_mut::<StatusPulseClock>();
                clock.tick = clock.tick.wrapping_add(1);
            }
            WindowRegistry::for_each_workspace(cx, |ws, window, cx| {
                if window.is_window_active() && ws.has_animating_claude_status() {
                    cx.notify();
                }
            });
        },
        cx,
    );
}

fn spawn_panels_reload(cx: &mut App) {
    let panels_reload_rx = panels_watcher::spawn_panels_watcher();
    watcher_pumps::spawn_drain_burst_pump(
        "panels-reload",
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
