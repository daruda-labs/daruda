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
    /// a lane can pin e.g. `/usr/local/bin/zsh` (Homebrew) or a
    /// nix-shell wrapper script independently of the user default.
    pub program: Option<String>,

    /// When `true`, the pane (and its containing tab, if it's the
    /// only pane) auto-closes as soon as the shell process exits.
    /// When `false`, the pane stays open showing the final output so
    /// the user can review it.
    pub close_pane_on_exit: bool,

    /// macOS-native cursor/edit shortcuts in the terminal. When `true`,
    /// `Cmd+←/→` jump to line start/end, `Opt+←/→` move by word, and
    /// `Cmd/Opt+Delete` perform the matching readline kills — mirroring
    /// iTerm2's "Natural Text Editing" preset. When `false`, those keys
    /// keep their default behaviour (`Cmd+Arrow` does nothing, `Opt+Arrow`
    /// sends the xterm CSI sequence).
    pub natural_text_editing: bool,
    /// Retain a bounded text viewport across app restarts. Processes restart.
    pub restore_output: bool,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            program: None,
            close_pane_on_exit: true,
            natural_text_editing: true,
            restore_output: false,
        }
    }
}
