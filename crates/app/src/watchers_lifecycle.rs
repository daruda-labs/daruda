//! App-wide watcher pumps spawned once at launch.
//!
//! Covers Claude status, attention demotion, status animation, deferred
//! Telegram flushes, and panels reload. Config reload lives with
//! `SettingsStore` because workspaces already subscribe to that global.

use crate::ui::StatusPulseClock;
use crate::ui::theme;
use crate::window_registry::WindowRegistry;
use crate::{hooks, panels_watcher, watcher_pumps};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use gpui::App;

/// App-global watcher handles parked for process lifetime; dropping one would
/// stop its OS subscription.
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

/// Re-check held Telegram pings and silent phone-triggered turns against their
/// quiet windows; idle work is just cheap empty/pane checks, and the 15s cadence
/// bounds notification delay.
fn spawn_deferred_telegram_flush(cx: &mut App) {
    watcher_pumps::spawn_periodic_pump(
        std::time::Duration::from_secs(15),
        |cx: &mut App| {
            WindowRegistry::for_each_workspace(cx, |ws, _window, cx| {
                ws.flush_deferred_telegram(cx);
                ws.flush_telegram_first_response_fallbacks(cx);
            });
        },
        cx,
    );
}

/// Drive the shared status-badge clock at ~4 fps. One gated pulse dirties only
/// windows with animation, avoiding per-badge animation redraws (CLAUDE.md
/// Pitfall #10).
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
                    // Cached dock entities must be dirtied explicitly or their
                    // animated badges freeze (Pitfall #10).
                    ws.notify_right_dock(cx);
                    ws.notify_left_dock(cx);
                }
                // AgentChat panes are separately cached; pulse them through
                // settle so their final running frame cannot freeze.
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
