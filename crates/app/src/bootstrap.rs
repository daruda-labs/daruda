//! Bootstrap entry points — run before GPUI takes over.
//!
//! Order: [`route_hook_subcommand`] → [`init_observability`] →
//! [`new_application`]. Hook routing comes first so non-GUI
//! `daruda --hook` invocations exit without instantiating
//! `Application`. Observability must precede any code that can emit
//! an `ErrorReport` or panic so the first report carries the right
//! version and panics survive a dead `LogWriter`.

use crate::hooks;
use crate::windows::{build_window_options, open_welcome_window};
use gpui::{Application, QuitMode};

/// Returns `Some(exit_code)` when invoked as `daruda --hook
/// <eventType>`. Callers in `main()` should exit with that code
/// immediately — the hook handler always exits 0 (see
/// `hooks::handler::run`) but we keep it generic in case future hook
/// types want to signal failure.
pub(crate) fn route_hook_subcommand() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("--hook") {
        let event_type = args.next().unwrap_or_default();
        return Some(hooks::handler::run(&event_type));
    }
    None
}

/// Observability bootstrap. Order matters — see module docs.
pub(crate) fn init_observability() {
    daruda_store::observability::system_info::set_app_version(env!("CARGO_PKG_VERSION"));
    let logs_cfg = daruda_config::Config::load().logs;
    let log_policy = daruda_store::observability::log_writer::LogPolicy {
        retention: logs_cfg.retention_duration(),
        max_file_size: logs_cfg.max_file_size_bytes(),
    };
    // Dev-build ACP wire tap: point `daruda_acp`'s wire logger at a file next to
    // the NDJSON logs so tool-call / subagent JSON-RPC traffic is captured
    // automatically in debug builds. Release builds never set this (the tap
    // stays off); either build can still opt in by exporting the var by hand.
    // Set BEFORE `LogWriter::init`, which spawns the log-writer worker thread —
    // afterwards the process is multi-threaded and `set_var` would be unsound.
    #[cfg(debug_assertions)]
    if std::env::var_os("DARUDA_ACP_WIRE_LOG").is_none()
        && let Some(dir) = daruda_store::observability::log_writer::log_dir()
    {
        // SAFETY: reached before `LogWriter::init` (below) spawns any thread and
        // after `shell_env` on the main thread, so the process is still
        // single-threaded — no other thread can read the environment concurrently.
        unsafe {
            std::env::set_var("DARUDA_ACP_WIRE_LOG", dir.join("acp-wire.log"));
        }
    }
    daruda_store::observability::log_writer::LogWriter::init(log_policy);
    std::panic::set_hook(Box::new(|info| {
        let report = daruda_store::observability::error_report::ErrorReport::from_panic(info);
        eprintln!("[daruda panic] {}", report.message);
        let _ = daruda_store::observability::log_writer::write_panic_log(&report);
    }));
}

/// Build the `gpui::Application` with daruda's asset bundle,
/// macOS-style quit mode, and the Dock-click reopen hook.
///
/// `on_reopen` is the macOS
/// `applicationShouldHandleReopen:hasVisibleWindows:` callback —
/// fires when the user clicks the Dock icon with no windows visible.
pub(crate) fn new_application() -> Application {
    let app = Application::with_platform(gpui_platform::current_platform(false))
        .with_assets(crate::assets::DarudaAssets)
        .with_quit_mode(QuitMode::Default);
    app.on_reopen(|cx| {
        if !cx.windows().is_empty() {
            return;
        }
        // Fall back to `Config::load()` if the reopen fires before
        // `globals::init_all` registered `SettingsStore`.
        let config = if cx.has_global::<crate::settings_store::SettingsStore>() {
            crate::settings_store::SettingsStore::global(cx).user_arc()
        } else {
            std::sync::Arc::new(daruda_config::Config::load())
        };
        let opts = build_window_options(&config);
        open_welcome_window(config, opts, cx);
    });
    app
}
