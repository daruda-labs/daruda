//! Blocking wrappers around `claude plugin install / uninstall`.
//!
//! Synchronous on purpose — UI callers shell out from
//! `cx.background_executor().spawn` so the main thread stays free.
//! Keeping the module GPUI-free lets it be unit-tested without a
//! GPUI fixture and reused from CLI helpers later.
//!
//! Spec lives behind `claude plugin --help`. Daruda only invokes:
//! - `claude plugin install <id>  --scope <s>`
//! - `claude plugin uninstall <id> --scope <s> --yes`
//!
//! The `--yes` flag is mandatory for uninstall when stdin is not a
//! TTY (background_executor never has one). Scope defaults to `user`
//! to mirror the CLI itself.

use std::process::{Command, Output};

/// Which CLI subcommand to invoke. Encoded as an enum so callers can
/// share the same plumbing for both verbs (toast text differs, but
/// the spawn / output handling is identical).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginAction {
    Install,
    Uninstall,
}

impl PluginAction {
    /// Subcommand verb passed straight to `claude plugin <verb>`.
    pub fn verb(self) -> &'static str {
        match self {
            PluginAction::Install => "install",
            PluginAction::Uninstall => "uninstall",
        }
    }
}

/// Installation scope passed via `--scope`. Mirrors the CLI's three
/// values; `user` is the default both here and in the CLI itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PluginScope {
    #[default]
    User,
    Project,
    Local,
}

impl PluginScope {
    pub fn flag(self) -> &'static str {
        match self {
            PluginScope::User => "user",
            PluginScope::Project => "project",
            PluginScope::Local => "local",
        }
    }
}

/// Errors surfaced from the spawn-and-wait path.
#[derive(Debug)]
pub enum PluginOpError {
    /// `claude` couldn't be launched — typically "not on PATH".
    Spawn(std::io::Error),
    /// The CLI ran but exited non-zero.
    Exit { code: Option<i32>, stderr: String },
    /// CLI output wasn't valid UTF-8 — extremely unlikely in practice
    /// but kept for parity with `lane::git`'s error shape.
    Utf8,
}

impl std::fmt::Display for PluginOpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginOpError::Spawn(e) => write!(f, "spawn claude: {e}"),
            PluginOpError::Exit {
                code: Some(c),
                stderr,
            } => write!(f, "claude exited {c}: {}", stderr.trim()),
            PluginOpError::Exit { code: None, stderr } => {
                write!(f, "claude terminated by signal: {}", stderr.trim())
            }
            PluginOpError::Utf8 => write!(f, "claude output not valid UTF-8"),
        }
    }
}

impl std::error::Error for PluginOpError {}

/// Run `claude plugin <action> <plugin_id> --scope <scope> [--yes]`
/// and return stdout on success.
///
/// `plugin_id` is the fully-qualified `<plugin>@<marketplace>` form
/// daruda already uses everywhere else. Caller is responsible for
/// deciding the scope.
pub fn run_plugin_action(
    action: PluginAction,
    plugin_id: &str,
    scope: PluginScope,
) -> Result<String, PluginOpError> {
    let mut cmd = Command::new("claude");
    cmd.arg("plugin")
        .arg(action.verb())
        .arg(plugin_id)
        .arg("--scope")
        .arg(scope.flag());
    // Uninstall prompts for confirmation when stdin isn't a TTY; the
    // `--yes` flag is what the CLI itself recommends in that case.
    if matches!(action, PluginAction::Uninstall) {
        cmd.arg("--yes");
    }
    let Output {
        status,
        stdout,
        stderr,
    } = cmd.output().map_err(PluginOpError::Spawn)?;

    if !status.success() {
        return Err(PluginOpError::Exit {
            code: status.code(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        });
    }
    String::from_utf8(stdout).map_err(|_| PluginOpError::Utf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_verbs_match_cli() {
        assert_eq!(PluginAction::Install.verb(), "install");
        assert_eq!(PluginAction::Uninstall.verb(), "uninstall");
    }

    #[test]
    fn scope_flags_match_cli() {
        assert_eq!(PluginScope::User.flag(), "user");
        assert_eq!(PluginScope::Project.flag(), "project");
        assert_eq!(PluginScope::Local.flag(), "local");
    }

    #[test]
    fn scope_default_is_user() {
        assert_eq!(PluginScope::default(), PluginScope::User);
    }

    #[test]
    fn error_display_formats_exit_code() {
        let err = PluginOpError::Exit {
            code: Some(1),
            stderr: "boom\n".into(),
        };
        assert!(format!("{err}").contains("claude exited 1: boom"));
    }
}
