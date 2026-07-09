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

/// A selectable ACP agent: an id, a display name, and the command that launches
/// its ACP adapter. The command is a bash-style string or a JSON stdio config —
/// daruda_acp parses it and provisions Node.js only for npx/node-family commands.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentDefinition {
    pub id: String,
    pub name: String,
    pub command: String,
}

/// A built-in ACP registry preset that can be inserted into the user catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub command: &'static str,
}

impl AgentPreset {
    pub fn definition(self) -> AgentDefinition {
        AgentDefinition {
            id: self.id.to_string(),
            name: self.name.to_string(),
            command: self.command.to_string(),
        }
    }
}

/// ACP registry snapshot used to seed the Settings preset list.
pub const ACP_REGISTRY_URL: &str =
    "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";
pub const ACP_REGISTRY_VERSION: &str = "1.0.0";

/// Presets generated from registry entries with directly runnable `npx`/`uvx`
/// distributions. Registry entries that only publish per-platform binary
/// archives need a downloader/extractor before they can be exposed here.
pub const ACP_REGISTRY_AGENT_PRESETS: &[AgentPreset] = &[
    AgentPreset {
        id: "agoragentic-acp",
        name: "Agoragentic",
        command: "npx -y agoragentic-mcp@1.3.0 --acp",
    },
    AgentPreset {
        id: "auggie",
        name: "Auggie CLI",
        command: "AUGMENT_DISABLE_AUTO_UPDATE=1 npx -y @augmentcode/auggie@0.32.0 --acp",
    },
    AgentPreset {
        id: "autohand",
        name: "Autohand Code",
        command: "npx -y @autohandai/autohand-acp@0.2.1",
    },
    AgentPreset {
        id: "claude-acp",
        name: "Claude Agent",
        command: "npx -y @agentclientprotocol/claude-agent-acp@0.57.0",
    },
    AgentPreset {
        id: "cline",
        name: "Cline",
        command: "npx -y cline@3.0.38 --acp",
    },
    AgentPreset {
        id: "codebuddy-code",
        name: "Codebuddy Code",
        command: "npx -y @tencent-ai/codebuddy-code@2.106.7 --acp",
    },
    AgentPreset {
        id: "codex-acp",
        name: "Codex",
        command: "npx -y @agentclientprotocol/codex-acp@1.1.0",
    },
    AgentPreset {
        id: "deepagents",
        name: "DeepAgents",
        command: "npx -y deepagents-acp@0.1.7",
    },
    AgentPreset {
        id: "dimcode",
        name: "DimCode",
        command: "npx -y dimcode@0.2.22 acp",
    },
    AgentPreset {
        id: "dirac",
        name: "Dirac",
        command: "npx -y dirac-cli@0.4.13 --acp",
    },
    AgentPreset {
        id: "factory-droid",
        name: "Factory Droid",
        command: "DROID_DISABLE_AUTO_UPDATE=true FACTORY_DROID_AUTO_UPDATE_ENABLED=false npx -y droid@0.167.0 exec --output-format acp-daemon",
    },
    AgentPreset {
        id: "fast-agent",
        name: "fast-agent",
        command: "uvx fast-agent-acp==0.9.2 -x",
    },
    AgentPreset {
        id: "gemini",
        name: "Gemini CLI",
        command: "npx -y @google/gemini-cli@0.49.0 --acp",
    },
    AgentPreset {
        id: "github-copilot-cli",
        name: "GitHub Copilot",
        command: "npx -y @github/copilot@1.0.69 --acp",
    },
    AgentPreset {
        id: "glm-acp-agent",
        name: "GLM Agent",
        command: "npx -y glm-acp-agent@1.1.4",
    },
    AgentPreset {
        id: "grok-build",
        name: "Grok Build",
        command: "npx -y @xai-official/grok@0.2.92 agent stdio",
    },
    AgentPreset {
        id: "kilo",
        name: "Kilo",
        command: "npx -y @kilocode/cli@7.4.1 acp",
    },
    AgentPreset {
        id: "minion-code",
        name: "Minion Code",
        command: "uvx minion-code@0.1.44 acp",
    },
    AgentPreset {
        id: "nova",
        name: "Nova",
        command: "npx -y @compass-ai/nova@1.1.25 acp",
    },
    AgentPreset {
        id: "pi-acp",
        name: "pi ACP",
        command: "npx -y pi-acp@0.0.31",
    },
    AgentPreset {
        id: "qoder",
        name: "Qoder CLI",
        command: "npx -y @qoder-ai/qodercli@0.2.14 --acp",
    },
    AgentPreset {
        id: "qwen-code",
        name: "Qwen Code",
        command: "npx -y @qwen-code/qwen-code@0.19.7 --acp --experimental-skills",
    },
    AgentPreset {
        id: "sigit",
        name: "siGit Code",
        command: "npx -y @smbcloud/sigit@1.4.0",
    },
];

impl AgentDefinition {
    /// The built-in Claude Code agent, used when config declares no `[[agents]]`.
    /// The command mirrors daruda_acp's default adapter launch; the "@latest"
    /// tag keeps us on the newest adapter (which advertises model/effort/mode as
    /// session config options). Keep this policy comment here — daruda_config is
    /// the single source now that the default command lives in config.
    pub fn claude_default() -> Self {
        Self {
            id: "claude".to_string(),
            name: "Claude Code".to_string(),
            command: "npx -y @agentclientprotocol/claude-agent-acp@latest".to_string(),
        }
    }

    /// The built-in Codex ACP agent preset exposed by the Settings UI.
    pub fn codex_default() -> Self {
        Self::registry_preset("codex-acp").expect("codex-acp registry preset exists")
    }

    pub fn registry_presets() -> Vec<Self> {
        ACP_REGISTRY_AGENT_PRESETS
            .iter()
            .copied()
            .map(AgentPreset::definition)
            .collect()
    }

    pub fn registry_preset(id: &str) -> Option<Self> {
        ACP_REGISTRY_AGENT_PRESETS
            .iter()
            .copied()
            .find(|preset| preset.id == id)
            .map(AgentPreset::definition)
    }
}

/// Default agent catalog: a single Claude entry. Used as the serde field default
/// (missing `[[agents]]`) AND to normalize an explicitly-empty catalog.
pub(crate) fn default_agents() -> Vec<AgentDefinition> {
    vec![AgentDefinition::claude_default()]
}

/// Agent chat configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
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
    /// Maximum number of visible rows before the bottom input scrolls
    /// internally. The input auto-grows from 1 row up to this limit,
    /// then clips and scrolls. Clamped to
    /// [`INPUT_MAX_ROWS_MIN`]..=[`INPUT_MAX_ROWS_MAX`] at load time.
    pub input_max_rows: u8,
}

/// Minimum allowed value for `AgentConfig::input_max_rows`.
pub const INPUT_MAX_ROWS_MIN: u8 = 2;
/// Maximum allowed value for `AgentConfig::input_max_rows`.
pub const INPUT_MAX_ROWS_MAX: u8 = 20;
/// Default maximum rows for the bottom input before it scrolls.
pub const INPUT_MAX_ROWS_DEFAULT: u8 = 8;

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            default_permission_mode: DefaultPermissionMode::default(),
            use_modifier_to_send: false,
            input_max_rows: INPUT_MAX_ROWS_DEFAULT,
        }
    }
}

impl AgentConfig {
    /// Clamp `input_max_rows` to its valid range.
    pub fn clamp(&mut self) {
        self.input_max_rows = self
            .input_max_rows
            .clamp(INPUT_MAX_ROWS_MIN, INPUT_MAX_ROWS_MAX);
    }
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

    #[test]
    fn claude_default_has_expected_fields() {
        let d = AgentDefinition::claude_default();
        assert_eq!(d.id, "claude");
        assert_eq!(d.name, "Claude Code");
        assert_eq!(
            d.command,
            "npx -y @agentclientprotocol/claude-agent-acp@latest"
        );
    }

    #[test]
    fn codex_default_has_expected_fields() {
        let d = AgentDefinition::codex_default();
        assert_eq!(d.id, "codex-acp");
        assert_eq!(d.name, "Codex");
        assert_eq!(d.command, "npx -y @agentclientprotocol/codex-acp@1.1.0");
    }

    #[test]
    fn registry_presets_include_directly_runnable_agents() {
        let presets = AgentDefinition::registry_presets();
        assert_eq!(ACP_REGISTRY_VERSION, "1.0.0");
        assert_eq!(ACP_REGISTRY_AGENT_PRESETS.len(), 23);
        assert_eq!(presets.len(), ACP_REGISTRY_AGENT_PRESETS.len());
        assert!(presets.iter().any(|p| p.id == "claude-acp"));
        assert!(presets.iter().any(|p| p.id == "codex-acp"));
        assert!(presets.iter().any(|p| {
            p.id == "factory-droid" && p.command.starts_with("DROID_DISABLE_AUTO_UPDATE=true ")
        }));
    }

    #[test]
    fn default_agents_is_single_claude() {
        assert_eq!(default_agents(), vec![AgentDefinition::claude_default()]);
    }

    #[test]
    fn agent_definition_field_round_trip() {
        let d = AgentDefinition {
            id: "codex".to_string(),
            name: "Codex".to_string(),
            command: "codex acp".to_string(),
        };
        let toml_str = toml::to_string(&d).expect("serialize");
        let back: AgentDefinition = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(back, d);
    }

    #[test]
    fn input_max_rows_defaults_and_clamps() {
        assert_eq!(
            AgentConfig::default().input_max_rows,
            INPUT_MAX_ROWS_DEFAULT
        );

        // Round-trip a non-default value.
        let toml_str = "input_max_rows = 5";
        let cfg: AgentConfig = toml::from_str(toml_str).expect("deserialize");
        assert_eq!(cfg.input_max_rows, 5);

        // Values below the minimum are clamped up.
        let mut too_low: AgentConfig = toml::from_str("input_max_rows = 0").expect("deserialize");
        too_low.clamp();
        assert_eq!(too_low.input_max_rows, INPUT_MAX_ROWS_MIN);

        // Values above the maximum are clamped down.
        let mut too_high: AgentConfig =
            toml::from_str("input_max_rows = 255").expect("deserialize");
        too_high.clamp();
        assert_eq!(too_high.input_max_rows, INPUT_MAX_ROWS_MAX);

        // Omitting the key keeps the default.
        let omitted: AgentConfig =
            toml::from_str("use_modifier_to_send = false").expect("deserialize");
        assert_eq!(omitted.input_max_rows, INPUT_MAX_ROWS_DEFAULT);
    }
}
