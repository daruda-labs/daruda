//! Bootstrap entry points — run before GPUI takes over.
//!
//! Order: [`route_hook_subcommand`] → [`init_observability`] →
//! [`new_application`]. Hook routing comes first so non-GUI
//! `daruda --hook` invocations exit without instantiating
//! `Application`. Observability must precede any code that can emit
//! an `ErrorReport` or panic so the first report carries the right
//! version and panics survive a dead `LogWriter`.

use crate::windows::open_empty_workspace_window;
use daruda_core::process_env;
use gpui::{Application, QuitMode};

/// Returns `Some(exit_code)` when invoked as `daruda --hook
/// <eventType>`. Callers in `main()` should exit with that code
/// immediately — the hook handler always exits 0 (see
/// `daruda_agent::hooks::handler::run`) but we keep it generic in case future hook
/// types want to signal failure.
pub(crate) fn route_hook_subcommand() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("--hook") {
        let event_type = args.next().unwrap_or_default();
        return Some(daruda_agent::hooks::handler::run(&event_type));
    }
    None
}

/// Returns `Some(exit_code)` when invoked as `daruda --mcp`.
///
/// Routed here next to `--hook` so a non-GUI invocation exits without
/// instantiating `Application`: a Metal context and a Dock presence per agent
/// session is not free, and an agent may spawn several.
pub(crate) fn route_mcp_subcommand() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some(crate::control::mcp::shim::SUBCOMMAND) {
        return Some(crate::control::mcp::shim::run());
    }
    None
}

/// Returns `Some(exit_code)` when invoked as `daruda --env …`.
///
/// Routed with the other non-GUI subcommands: this one stands in for
/// `env(1)` where the host has none, and an ACP launch puts it between the
/// app and the adapter — a window per adapter would be absurd.
pub(crate) fn route_env_subcommand() -> Option<i32> {
    crate::env_strip::route()
}

/// Returns `Some(exit_code)` when invoked as `daruda --await-exit <pid>`.
///
/// The half of a portable-install update that cannot run inside the process
/// being replaced: wait for it to go, then start the new executable. Two
/// instances must never overlap — they contend for the control socket and the
/// flow locks — so this waits rather than racing.
pub(crate) fn route_await_exit_subcommand() -> Option<i32> {
    Some(
        match crate::update::parse_await_exit(std::env::args().skip(1))? {
            Ok(pid) => crate::update::await_exit_and_start(pid),
            // Same code `--env` reports for a line it cannot read.
            Err(()) => 2,
        },
    )
}

/// Observability bootstrap. Order matters — see module docs.
pub(crate) fn init_observability() {
    daruda_store::observability::system_info::set_app_version(env!("CARGO_PKG_VERSION"));
    let logs_cfg = daruda_config::Config::load().logs;
    let log_policy = daruda_store::observability::log_writer::LogPolicy {
        retention: logs_cfg.retention_duration(),
        max_file_size: logs_cfg.max_file_size_bytes(),
    };
    // Diagnostic sinks: the ACP wire tap and the Telegram trace. Debug builds
    // default both to files beside the NDJSON logs; either build can opt in
    // by exporting the variable by hand.
    let log_dir = cfg!(debug_assertions)
        .then(daruda_store::observability::log_writer::log_dir)
        .flatten();
    daruda_acp::wire_log::configure_from_env(log_dir.as_ref().map(|d| d.join("acp-wire.log")));
    crate::telegram::trace::configure_from_env(
        log_dir.map(|d| d.join(crate::telegram::trace::TRACE_FILE_NAME)),
    );
    // Both are fixed in-process now, so no child — an adapter, a terminal
    // shell, a `cargo test` an agent runs — may inherit a sink this process
    // owns: an inherited wire tap rotates the live capture away.
    // Removed BEFORE `LogWriter::init`, which spawns the log-writer worker
    // thread — afterwards the process is multi-threaded and `remove_var`
    // would be unsound.
    for key in process_env::PROCESS_LOCAL_SINKS {
        // SAFETY: reached before `LogWriter::init` (below) spawns any thread and
        // after `shell_env` on the main thread, so the process is still
        // single-threaded — no other thread can read the environment concurrently.
        unsafe {
            std::env::remove_var(key.name());
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
        // On macOS `QuitMode::Default` resolves to `Explicit`, so the app
        // outlives its last window and this is the way back in; the other
        // platforms quit instead and never reach here.
        open_empty_workspace_window(cx).unwrap();
    });
    app
}
