use serde::{Deserialize, Serialize};

/// Shell process + lifecycle configuration.
///
/// `program` controls which executable spawns inside each new pane;
/// `close_pane_on_exit` controls what happens when that process ends.
/// Both fields are settable at the user-global layer and (when wrapped
/// in `ProjectConfig`) overridable per project.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ShellConfig {
    /// Shell executable path. `None` falls back to the `$SHELL`
    /// environment variable, then `/bin/zsh`. Project-overridable so
    /// a worktree can pin e.g. `/usr/local/bin/zsh` (Homebrew) or a
    /// nix-shell wrapper script independently of the user default.
    pub program: Option<String>,

    /// When `true`, the pane (and its containing tab, if it's the
    /// only pane) auto-closes as soon as the shell process exits.
    /// When `false`, the pane stays open showing the final output so
    /// the user can review it.
    pub close_pane_on_exit: bool,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            program: None,
            close_pane_on_exit: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_closes_on_exit() {
        assert!(ShellConfig::default().close_pane_on_exit);
    }

    #[test]
    fn parses_disabled() {
        let toml = "close_pane_on_exit = false";
        let cfg: ShellConfig = toml::from_str(toml).unwrap();
        assert!(!cfg.close_pane_on_exit);
    }

    #[test]
    fn parses_empty_to_default() {
        let cfg: ShellConfig = toml::from_str("").unwrap();
        assert!(cfg.close_pane_on_exit);
        assert!(cfg.program.is_none());
    }

    #[test]
    fn parses_program_override() {
        let toml = "program = \"/usr/local/bin/zsh\"";
        let cfg: ShellConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.program.as_deref(), Some("/usr/local/bin/zsh"));
        // Other fields fall back to defaults.
        assert!(cfg.close_pane_on_exit);
    }
}
