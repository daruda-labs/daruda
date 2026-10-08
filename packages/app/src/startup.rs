//! Route non-GUI commands before preparing the desktop process.

mod desktop;
mod failure;
mod inspection;

pub(crate) fn run() {
    // Helpers must inherit the original environment and never create a GUI,
    // initialize desktop ownership, or contend with the user's running app.
    if let Some(code) = run_helper() {
        std::process::exit(code);
    }
    crate::shell_env::hydrate_path_from_login_shell();
    crate::bootstrap::init_observability();
    if let Some(code) = run_cleanup() {
        if code != 0 {
            std::process::exit(code);
        }
        return;
    }
    desktop::run();
}

fn run_helper() -> Option<i32> {
    crate::bootstrap::route_hook_subcommand()
        .or_else(crate::bootstrap::route_mcp_subcommand)
        .or_else(crate::bootstrap::route_await_exit_subcommand)
        .or_else(crate::bootstrap::route_env_subcommand)
}

fn run_cleanup() -> Option<i32> {
    if !std::env::args_os().any(|arg| arg == "--unregister-desktop") {
        return None;
    }
    Some(match crate::platform::notifications::unregister_desktop() {
        Ok(()) => 0,
        Err(error) => {
            crate::platform::report_error(
                "desktop.shortcuts",
                "Desktop shortcut cleanup failed",
                error.as_ref(),
            );
            1
        }
    })
}
