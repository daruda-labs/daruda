use serde::{Deserialize, Serialize};

/// Permission mode the agent chat session starts in. Mirrors Claude Code's
/// permission modes; applied on connect via ACP session/set_mode when the
/// adapter advertises it. `BypassPermissions` runs every tool call without a
/// prompt (intended for isolated environments) — it is the default here.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum DefaultPermissionMode {
    /// Only reads run without asking; edits/commands prompt each time.
    #[serde(rename = "default")]
    Default,
    /// Reads + file edits + common filesystem commands in the working dir run
    /// without asking; other commands prompt.
    #[serde(rename = "acceptEdits")]
    AcceptEdits,
    /// Reads only; Claude analyzes/proposes but does not edit.
    #[serde(rename = "plan")]
    Plan,
    /// Everything runs without asking, no safety checks. Intended for isolated
    /// containers/VMs; refused under root/sudo.
    #[default]
    #[serde(rename = "bypassPermissions")]
    BypassPermissions,
}

impl DefaultPermissionMode {
    /// All variants in declaration order — used to enumerate options without
    /// hardcoding strings at call sites.
    pub const ALL: [DefaultPermissionMode; 4] = [
        Self::Default,
        Self::AcceptEdits,
        Self::Plan,
        Self::BypassPermissions,
    ];

    /// The ACP/Claude-Code mode id string (matches advertised SessionMode ids).
    pub fn mode_id(self) -> &'static str {
        match self {
            DefaultPermissionMode::Default => "default",
            DefaultPermissionMode::AcceptEdits => "acceptEdits",
            DefaultPermissionMode::Plan => "plan",
            DefaultPermissionMode::BypassPermissions => "bypassPermissions",
        }
    }

    /// Look up a variant by its ACP mode id string. Returns `None` if `id`
    /// does not match any known variant.
    pub fn from_mode_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.mode_id() == id)
    }
}

/// Agent chat configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentConfig {
    /// Permission mode applied when an agent chat session connects.
    pub default_permission_mode: DefaultPermissionMode,
    /// How the agent chat input submits a message. When `false` (the
    /// default), plain Enter sends and Shift+Enter inserts a newline —
    /// matching Zed's agent panel default. When `true`, Enter inserts a
    /// newline and Cmd+Enter (Ctrl+Enter on Linux/Windows) sends. Only
    /// affects the bottom input while an agent chat pane is focused; the
    /// terminal input always uses Cmd+Enter to send.
    pub use_modifier_to_send: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_id_strings_are_exact() {
        assert_eq!(DefaultPermissionMode::Default.mode_id(), "default");
        assert_eq!(DefaultPermissionMode::AcceptEdits.mode_id(), "acceptEdits");
        assert_eq!(DefaultPermissionMode::Plan.mode_id(), "plan");
        assert_eq!(
            DefaultPermissionMode::BypassPermissions.mode_id(),
            "bypassPermissions"
        );
    }

    #[test]
    fn default_is_bypass_permissions() {
        assert_eq!(
            DefaultPermissionMode::default(),
            DefaultPermissionMode::BypassPermissions
        );
        assert_eq!(
            AgentConfig::default().default_permission_mode.mode_id(),
            "bypassPermissions"
        );
    }

    #[test]
    fn from_mode_id_round_trips_all_variants() {
        for m in DefaultPermissionMode::ALL {
            assert_eq!(
                DefaultPermissionMode::from_mode_id(m.mode_id()),
                Some(m),
                "from_mode_id({}) should return the original variant",
                m.mode_id()
            );
        }
    }

    #[test]
    fn from_mode_id_returns_none_for_unknown_id() {
        assert_eq!(DefaultPermissionMode::from_mode_id("bogus"), None);
        assert_eq!(DefaultPermissionMode::from_mode_id(""), None);
        assert_eq!(
            DefaultPermissionMode::from_mode_id("BypassPermissions"),
            None
        );
    }

    #[test]
    fn toml_round_trip_all_variants() {
        // Verify serde(rename_all = "camelCase") produces the right TOML keys.
        let cases = [
            (DefaultPermissionMode::Default, "default"),
            (DefaultPermissionMode::AcceptEdits, "acceptEdits"),
            (DefaultPermissionMode::Plan, "plan"),
            (
                DefaultPermissionMode::BypassPermissions,
                "bypassPermissions",
            ),
        ];
        for (variant, expected_str) in cases {
            let cfg = AgentConfig {
                default_permission_mode: variant,
                ..AgentConfig::default()
            };
            let toml_str = toml::to_string(&cfg).expect("serialize");
            assert!(
                toml_str.contains(expected_str),
                "expected TOML to contain \"{expected_str}\", got: {toml_str}"
            );
            let back: AgentConfig = toml::from_str(&toml_str).expect("deserialize");
            assert_eq!(back.default_permission_mode, variant);
        }
    }

    #[test]
    fn missing_agent_section_deserializes_to_default() {
        let cfg: AgentConfig = toml::from_str("").expect("empty TOML should produce defaults");
        assert_eq!(
            cfg.default_permission_mode,
            DefaultPermissionMode::BypassPermissions
        );
    }

    #[test]
    fn use_modifier_to_send_defaults_false_and_round_trips() {
        // Default matches Zed's agent panel: Enter sends, Shift+Enter newline.
        assert!(!AgentConfig::default().use_modifier_to_send);

        let cfg = AgentConfig {
            use_modifier_to_send: true,
            ..AgentConfig::default()
        };
        let toml_str = toml::to_string(&cfg).expect("serialize");
        let back: AgentConfig = toml::from_str(&toml_str).expect("deserialize");
        assert!(back.use_modifier_to_send);

        // A config that omits the key keeps the default.
        let omitted: AgentConfig =
            toml::from_str("default_permission_mode = \"plan\"").expect("deserialize");
        assert!(!omitted.use_modifier_to_send);
    }
}
