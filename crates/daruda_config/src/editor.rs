//! Preferred external editor for "open externally" actions (agent-chat diff
//! header, file viewer, etc.) — a small built-in catalog plus the config
//! section selecting one.
//!
//! Unlike the ACP agent catalog ([`crate::agent`]), this is deliberately not
//! user-extensible yet: [`EditorConfig::preferred`] holds one of
//! [`PRESETS`]'s [`ExternalEditorPreset::name`]s, or an empty string meaning
//! "use the OS default handler" (the pre-existing behavior, and
//! `EditorConfig`'s default).

use serde::{Deserialize, Serialize};

/// A built-in external-editor option — display metadata plus enough
/// per-platform launch data for the app to build the actual open command.
/// macOS: `macos_bundle_ids` (when non-empty) is a multi-edition fallback
/// list tried via `open -b <id>`, since a JetBrains IDE's `.app` display name
/// varies by edition (Community vs Ultimate) but its bundle id doesn't;
/// otherwise `macos_app_name` is used via `open -a "<name>"`. Linux:
/// `linux_cli_candidates` are tried in order as direct CLI commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalEditorPreset {
    /// The internal key used in `config.toml` (`editor.preferred = "<name>"`)
    /// and as the Settings dropdown's option value.
    pub name: &'static str,
    /// The human-readable name shown in the Settings UI. Not localized — an
    /// editor's own name, like an agent's or a theme preset's, is a proper
    /// noun.
    pub display_name: &'static str,
    pub macos_app_name: Option<&'static str>,
    pub macos_bundle_ids: &'static [&'static str],
    pub linux_cli_candidates: &'static [&'static str],
}

/// All built-in editor presets, in the order they appear in the Settings
/// dropdown. Deliberately small (8) rather than exhaustive — an editor
/// missing here has no escape hatch yet (unlike [`crate::agent::AgentEntry`],
/// which allows a raw custom command); widen this list before that becomes a
/// blocker.
pub const PRESETS: &[ExternalEditorPreset] = &[
    ExternalEditorPreset {
        name: "vscode",
        display_name: "VS Code",
        macos_app_name: Some("Visual Studio Code"),
        macos_bundle_ids: &[],
        linux_cli_candidates: &["code"],
    },
    ExternalEditorPreset {
        name: "cursor",
        display_name: "Cursor",
        macos_app_name: Some("Cursor"),
        macos_bundle_ids: &[],
        linux_cli_candidates: &["cursor"],
    },
    ExternalEditorPreset {
        name: "zed",
        display_name: "Zed",
        macos_app_name: Some("Zed"),
        macos_bundle_ids: &[],
        linux_cli_candidates: &["zed"],
    },
    ExternalEditorPreset {
        name: "intellij",
        display_name: "IntelliJ IDEA",
        macos_app_name: None,
        macos_bundle_ids: &["com.jetbrains.intellij", "com.jetbrains.intellij.ce"],
        linux_cli_candidates: &["idea"],
    },
    ExternalEditorPreset {
        name: "webstorm",
        display_name: "WebStorm",
        macos_app_name: Some("WebStorm"),
        macos_bundle_ids: &[],
        linux_cli_candidates: &["webstorm"],
    },
    ExternalEditorPreset {
        name: "pycharm",
        display_name: "PyCharm",
        macos_app_name: None,
        macos_bundle_ids: &["com.jetbrains.pycharm", "com.jetbrains.pycharm.ce"],
        linux_cli_candidates: &["pycharm"],
    },
    ExternalEditorPreset {
        name: "xcode",
        display_name: "Xcode",
        macos_app_name: Some("Xcode"),
        macos_bundle_ids: &[],
        linux_cli_candidates: &[],
    },
    ExternalEditorPreset {
        name: "sublime",
        display_name: "Sublime Text",
        macos_app_name: Some("Sublime Text"),
        macos_bundle_ids: &[],
        linux_cli_candidates: &["subl"],
    },
];

/// The preset named `name`, or `None` for an empty/unrecognized name — the
/// caller's cue to fall back to the OS default handler.
pub fn preset(name: &str) -> Option<&'static ExternalEditorPreset> {
    PRESETS.iter().find(|p| p.name == name)
}

/// Preferred external editor, config-wide (see module docs).
///
/// Sample `config.toml`:
///
/// ```toml
/// [editor]
/// preferred = "vscode"   # empty (default) = OS default handler
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct EditorConfig {
    pub preferred: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_preferred_is_empty() {
        assert_eq!(EditorConfig::default().preferred, "");
    }

    #[test]
    fn empty_name_has_no_preset() {
        assert!(preset("").is_none());
    }

    #[test]
    fn unknown_name_has_no_preset() {
        assert!(preset("not-a-real-editor").is_none());
    }

    #[test]
    fn every_preset_name_resolves_to_itself() {
        for p in PRESETS {
            assert_eq!(preset(p.name).map(|found| found.name), Some(p.name));
        }
    }

    #[test]
    fn preset_names_are_unique() {
        let mut names: Vec<_> = PRESETS.iter().map(|p| p.name).collect();
        names.sort_unstable();
        let mut deduped = names.clone();
        deduped.dedup();
        assert_eq!(names.len(), deduped.len(), "preset names must be unique");
    }

    #[test]
    fn every_preset_has_at_least_one_macos_launch_path() {
        for p in PRESETS {
            assert!(
                p.macos_app_name.is_some() || !p.macos_bundle_ids.is_empty(),
                "{} has neither a macOS app name nor bundle ids",
                p.name
            );
        }
    }
}
