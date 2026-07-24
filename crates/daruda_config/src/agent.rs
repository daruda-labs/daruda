use serde::{Deserialize, Serialize};

/// Permission mode the agent chat session starts in. Mirrors Claude Code's
/// permission modes; applied on connect via ACP session/set_mode when the
/// adapter advertises it. Variants and ids track the adapter's advertised
/// `availableModes` set. `BypassPermissions` (everything runs without asking)
/// is the default here; when the adapter no longer advertises it or refuses
/// the switch, the connect falls back to [`Self::CONNECT_FALLBACK`] (`Auto`) —
/// see `daruda_acp`'s set_mode application.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum DefaultPermissionMode {
    /// A model classifier approves/denies each permission prompt.
    #[serde(rename = "auto")]
    Auto,
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
    /// Never prompts; denies anything not already pre-approved.
    #[serde(rename = "dontAsk")]
    DontAsk,
    /// Everything runs without asking, no safety checks. Intended for isolated
    /// containers/VMs; refused under root/sudo.
    #[default]
    #[serde(rename = "bypassPermissions")]
    BypassPermissions,
}

impl DefaultPermissionMode {
    /// All variants in declaration order (matches the adapter's advertised
    /// order) — used to enumerate options without hardcoding strings at call
    /// sites.
    pub const ALL: [DefaultPermissionMode; 6] = [
        Self::Auto,
        Self::Default,
        Self::AcceptEdits,
        Self::Plan,
        Self::DontAsk,
        Self::BypassPermissions,
    ];

    /// The ACP/Claude-Code mode id string (matches advertised SessionMode ids).
    pub fn mode_id(self) -> &'static str {
        match self {
            DefaultPermissionMode::Auto => "auto",
            DefaultPermissionMode::Default => "default",
            DefaultPermissionMode::AcceptEdits => "acceptEdits",
            DefaultPermissionMode::Plan => "plan",
            DefaultPermissionMode::DontAsk => "dontAsk",
            DefaultPermissionMode::BypassPermissions => "bypassPermissions",
        }
    }

    /// Look up a variant by its ACP mode id string. Returns `None` if `id`
    /// does not match any known variant.
    pub fn from_mode_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.mode_id() == id)
    }

    /// The mode a connect falls back to when the configured mode is not
    /// advertised by the adapter or its `set_mode` is rejected. `Auto` (a model
    /// classifier that approves/denies each prompt) is a safe, always-current
    /// choice — it is the adapter's own default and the least likely mode to be
    /// removed.
    pub const CONNECT_FALLBACK: DefaultPermissionMode = DefaultPermissionMode::Auto;

    /// Priority-ordered mode ids to try on connect: the configured mode first,
    /// then [`Self::CONNECT_FALLBACK`]. `daruda_acp` applies the first one the
    /// adapter both advertises and accepts, so a session lands on the fallback
    /// only when the preferred mode is unavailable. The fallback is omitted when
    /// it equals the configured mode (no redundant candidate).
    pub fn connect_mode_priority(self) -> Vec<&'static str> {
        let mut priority = vec![self.mode_id()];
        if self != Self::CONNECT_FALLBACK {
            priority.push(Self::CONNECT_FALLBACK.mode_id());
        }
        priority
    }
}

/// A selectable ACP agent: an id, a display name, and how its ACP adapter is
/// launched.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(from = "AgentDefinitionRepr", into = "AgentDefinitionRepr")]
pub struct AgentDefinition {
    pub id: String,
    pub name: String,
    pub launch: AgentLaunch,
}

/// How an ACP agent adapter is launched. `Raw` runs a bash-style command (or
/// JSON stdio config) exactly as given — daruda_acp parses it and provisions
/// Node.js only for npx/node-family commands; this is what every registry
/// preset, `claude_default()`, and every pre-migration config use. `Ssh` /
/// `Docker` reach an adapter that runs on another host / inside a running
/// container, and need a remote working directory assembled into the launch
/// command at connect time via [`AgentLaunch::wrap`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentLaunch {
    /// Runs `command` exactly as given — no wrapping. May contain the
    /// [`CWD_TOKEN`] placeholder (e.g. to `cd` into a remote path over `ssh`
    /// before launching the adapter); substituted at connect time by
    /// [`AgentLaunch::wrap`], and its presence marks the agent as remote —
    /// see [`AgentLaunch::needs_remote_cwd`]. This is the legacy
    /// hand-written escape hatch; new remote setups should use `Ssh` /
    /// `Docker` instead.
    Raw(String),
    /// `adapter_command` runs on `host` over SSH; [`AgentLaunch::wrap`]
    /// assembles `ssh <host> sh -c 'cd "<remote path>" && <adapter_command>'`.
    Ssh {
        adapter_command: String,
        host: String,
    },
    /// `adapter_command` runs inside the already-running container
    /// `container`; [`AgentLaunch::wrap`] assembles
    /// `docker exec -i <container> sh -c 'cd "<remote path>" && <adapter_command>'`.
    Docker {
        adapter_command: String,
        container: String,
    },
}

/// Placeholder token in an [`AgentLaunch::Raw`] command substituted with the
/// working directory to launch the adapter in.
pub const CWD_TOKEN: &str = "{{cwd}}";

/// `remote_path`, once validated non-blank by [`AgentLaunch::wrap`], borrowed
/// back out — or `Err(())` when it is `None` or blank (whitespace-only).
fn require_remote_path(remote_path: Option<&str>) -> Result<&str, ()> {
    match remote_path {
        Some(path) if !path.trim().is_empty() => Ok(path),
        _ => Err(()),
    }
}

impl AgentLaunch {
    /// Whether this launch needs a remote working directory substituted in
    /// before [`Self::wrap`] can succeed. Always `true` for `Ssh` / `Docker`;
    /// for `Raw`, `true` iff the command contains the literal [`CWD_TOKEN`]
    /// — the only place token-sniffing still happens, preserved as the
    /// legacy `Raw` escape hatch.
    pub fn needs_remote_cwd(&self) -> bool {
        match self {
            AgentLaunch::Raw(command) => command.contains(CWD_TOKEN),
            AgentLaunch::Ssh { .. } | AgentLaunch::Docker { .. } => true,
        }
    }

    /// Build the final shell command to launch this adapter, substituting
    /// `remote_path` in where needed.
    ///
    /// - `Raw`: if the command has no [`CWD_TOKEN`], returned unchanged and
    ///   `remote_path` is ignored. If it does, `remote_path` must be `Some`
    ///   and non-blank (whitespace-only counts as blank), else `Err(())`;
    ///   otherwise the token is replaced with `remote_path` verbatim.
    /// - `Ssh` / `Docker`: `remote_path` must be `Some` and non-blank, else
    ///   `Err(())`; otherwise the adapter command is wrapped to `cd` into
    ///   `remote_path` before running.
    ///
    /// `Err(())` rather than a richer error type: the only failure is "no
    /// usable remote path was supplied", a precondition the caller already
    /// knows the human-readable reason for (e.g. "lane has no remote cwd
    /// yet") — there is nothing more to report back.
    #[allow(clippy::result_unit_err)]
    pub fn wrap(&self, remote_path: Option<&str>) -> Result<String, ()> {
        match self {
            AgentLaunch::Raw(command) => {
                if command.contains(CWD_TOKEN) {
                    let path = require_remote_path(remote_path)?;
                    Ok(command.replace(CWD_TOKEN, path))
                } else {
                    Ok(command.clone())
                }
            }
            AgentLaunch::Ssh {
                adapter_command,
                host,
            } => {
                let path = require_remote_path(remote_path)?;
                Ok(format!(
                    "ssh {host} sh -c 'cd \"{path}\" && {adapter_command}'"
                ))
            }
            AgentLaunch::Docker {
                adapter_command,
                container,
            } => {
                let path = require_remote_path(remote_path)?;
                Ok(format!(
                    "docker exec -i {container} sh -c 'cd \"{path}\" && {adapter_command}'"
                ))
            }
        }
    }

    /// Like [`Self::wrap`], but injects `env.inject` and removes `env.strip`
    /// for the spawned process. `Raw` uses a `KEY=value` prefix (preserved by
    /// the managed Node path); `Ssh`/`Docker` fold it into the remote shell
    /// via export/unset.
    #[allow(clippy::result_unit_err)]
    pub fn wrap_with_env(
        &self,
        remote_path: Option<&str>,
        env: &crate::account_env::AccountEnv,
    ) -> Result<String, ()> {
        let base = self.wrap(remote_path)?;
        match self {
            AgentLaunch::Raw(_) => {
                // Single-quoted so a value containing a space (an account
                // config dir under `default_data_dir()`, which on macOS
                // sits under `~/Library/Application Support`) survives the
                // downstream quote-aware re-tokenization in
                // `daruda_acp::node::split_env_prefixed_tokens` /
                // `AcpAgent::from_str`'s `shell_words::split` as one value
                // instead of splitting into two tokens. No inner escaping is
                // needed: a filesystem config-dir path never contains a
                // single quote.
                let mut prefix = String::new();
                for (k, v) in &env.inject {
                    prefix.push_str(&format!("{k}='{v}' "));
                }
                Ok(format!("{prefix}{base}"))
            }
            AgentLaunch::Ssh { .. } | AgentLaunch::Docker { .. } => {
                // `base` is `ssh host sh -c 'cd "<p>" && <cmd>'`. Inject
                // exports inside the inner shell by rewriting the trailing
                // `&& <cmd>`.
                let mut env_script = String::new();
                for s in &env.strip {
                    env_script.push_str(&format!("unset {s}; "));
                }
                for (k, v) in &env.inject {
                    env_script.push_str(&format!("export {k}=\"{v}\"; "));
                }
                // The inner command starts after the last `&& `.
                match base.rfind("&& ") {
                    Some(idx) => {
                        let (head, tail) = base.split_at(idx + 3);
                        Ok(format!("{head}{env_script}{tail}"))
                    }
                    None => Ok(base),
                }
            }
        }
    }

    /// The interactive login command for this agent, or `None` for remote
    /// launches (SSH/Docker) where a local desktop browser can't complete OAuth.
    /// The subscription (`--claudeai`) flow; matches orca's add-account command.
    pub fn login_command(&self) -> Option<String> {
        match self {
            AgentLaunch::Raw(command) => Some(format!("{command} --cli auth login --claudeai")),
            AgentLaunch::Ssh { .. } | AgentLaunch::Docker { .. } => None,
        }
    }
}

/// Private wire representation for [`AgentDefinition`]. `id`/`name` stay flat
/// TOML keys; the launch variant is either the flat `command` key
/// (`AgentLaunch::Raw`, kept for backward-compatible configs) or exactly one
/// of the `[agents.ssh]` / `[agents.docker]` sub-tables — a plain (not
/// untagged) struct since the three launch fields live at different keys
/// rather than sharing one slot.
#[derive(Serialize, Deserialize)]
struct AgentDefinitionRepr {
    id: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ssh: Option<SshLaunchRepr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    docker: Option<DockerLaunchRepr>,
}

#[derive(Serialize, Deserialize)]
struct SshLaunchRepr {
    adapter_command: String,
    host: String,
}

#[derive(Serialize, Deserialize)]
struct DockerLaunchRepr {
    adapter_command: String,
    container: String,
}

impl From<AgentDefinition> for AgentDefinitionRepr {
    fn from(v: AgentDefinition) -> Self {
        let (command, ssh, docker) = match v.launch {
            AgentLaunch::Raw(command) => (Some(command), None, None),
            AgentLaunch::Ssh {
                adapter_command,
                host,
            } => (
                None,
                Some(SshLaunchRepr {
                    adapter_command,
                    host,
                }),
                None,
            ),
            AgentLaunch::Docker {
                adapter_command,
                container,
            } => (
                None,
                None,
                Some(DockerLaunchRepr {
                    adapter_command,
                    container,
                }),
            ),
        };
        Self {
            id: v.id,
            name: v.name,
            command,
            ssh,
            docker,
        }
    }
}

impl From<AgentDefinitionRepr> for AgentDefinition {
    /// Reads whichever of `ssh` / `docker` / `command` is set. Priority order
    /// `ssh` > `docker` > `command` is a deliberate but arbitrary tie-break
    /// for a hand-edited config that sets more than one at once — this isn't
    /// expected in practice since daruda itself only ever writes one.
    /// Defaults to `Raw("")` if none are set at all.
    fn from(v: AgentDefinitionRepr) -> Self {
        let launch = if let Some(ssh) = v.ssh {
            AgentLaunch::Ssh {
                adapter_command: ssh.adapter_command,
                host: ssh.host,
            }
        } else if let Some(docker) = v.docker {
            AgentLaunch::Docker {
                adapter_command: docker.adapter_command,
                container: docker.container,
            }
        } else {
            AgentLaunch::Raw(v.command.unwrap_or_default())
        };
        Self {
            id: v.id,
            name: v.name,
            launch,
        }
    }
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
            launch: AgentLaunch::Raw(self.command.to_string()),
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
///
/// Every command pins `@latest` (npx dist-tag / uvx version alias) rather than
/// a specific version — same policy as [`AgentDefinition::claude_default`].
/// Pinning a snapshot version here meant every upstream release (new model,
/// bugfix, protocol feature) needed a manual PR to bump it — see the Codex
/// GPT-5.6 rollout, where the pinned `codex-acp` build predated the model by a
/// week and it silently never showed up. `@latest` trades that manual-bump tax
/// for exposure to a bad upstream release reaching every user simultaneously
/// with no automatic rollback — accepted here because `claude_default` already
/// carries that same risk for daruda's primary agent.
pub const ACP_REGISTRY_AGENT_PRESETS: &[AgentPreset] = &[
    AgentPreset {
        id: "agoragentic-acp",
        name: "Agoragentic",
        command: "npx -y agoragentic-mcp@latest --acp",
    },
    AgentPreset {
        id: "auggie",
        name: "Auggie CLI",
        command: "AUGMENT_DISABLE_AUTO_UPDATE=1 npx -y @augmentcode/auggie@latest --acp",
    },
    AgentPreset {
        id: "autohand",
        name: "Autohand Code",
        command: "npx -y @autohandai/autohand-acp@latest",
    },
    AgentPreset {
        id: "claude-acp",
        name: "Claude Agent",
        command: "npx -y @agentclientprotocol/claude-agent-acp@latest",
    },
    AgentPreset {
        id: "cline",
        name: "Cline",
        command: "npx -y cline@latest --acp",
    },
    AgentPreset {
        id: "codebuddy-code",
        name: "Codebuddy Code",
        command: "npx -y @tencent-ai/codebuddy-code@latest --acp",
    },
    AgentPreset {
        id: "codex-acp",
        name: "Codex",
        command: "npx -y @agentclientprotocol/codex-acp@latest",
    },
    AgentPreset {
        id: "deepagents",
        name: "DeepAgents",
        command: "npx -y deepagents-acp@latest",
    },
    AgentPreset {
        id: "dimcode",
        name: "DimCode",
        command: "npx -y dimcode@latest acp",
    },
    AgentPreset {
        id: "dirac",
        name: "Dirac",
        command: "npx -y dirac-cli@latest --acp",
    },
    AgentPreset {
        id: "factory-droid",
        name: "Factory Droid",
        command: "DROID_DISABLE_AUTO_UPDATE=true FACTORY_DROID_AUTO_UPDATE_ENABLED=false npx -y droid@latest exec --output-format acp-daemon",
    },
    AgentPreset {
        id: "fast-agent",
        name: "fast-agent",
        command: "uvx fast-agent-acp@latest -x",
    },
    AgentPreset {
        id: "gemini",
        name: "Gemini CLI",
        command: "npx -y @google/gemini-cli@latest --acp",
    },
    AgentPreset {
        id: "github-copilot-cli",
        name: "GitHub Copilot",
        command: "npx -y @github/copilot@latest --acp",
    },
    AgentPreset {
        id: "glm-acp-agent",
        name: "GLM Agent",
        command: "npx -y glm-acp-agent@latest",
    },
    AgentPreset {
        id: "grok-build",
        name: "Grok Build",
        command: "npx -y @xai-official/grok@latest agent stdio",
    },
    AgentPreset {
        id: "kilo",
        name: "Kilo",
        command: "npx -y @kilocode/cli@latest acp",
    },
    AgentPreset {
        id: "minion-code",
        name: "Minion Code",
        command: "uvx minion-code@latest acp",
    },
    AgentPreset {
        id: "nova",
        name: "Nova",
        command: "npx -y @compass-ai/nova@latest acp",
    },
    AgentPreset {
        id: "pi-acp",
        name: "pi ACP",
        command: "npx -y pi-acp@latest",
    },
    AgentPreset {
        id: "qoder",
        name: "Qoder CLI",
        command: "npx -y @qoder-ai/qodercli@latest --acp",
    },
    AgentPreset {
        id: "qwen-code",
        name: "Qwen Code",
        command: "npx -y @qwen-code/qwen-code@latest --acp --experimental-skills",
    },
    AgentPreset {
        id: "sigit",
        name: "siGit Code",
        command: "npx -y @smbcloud/sigit@latest",
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
            launch: AgentLaunch::Raw(
                "npx -y @agentclientprotocol/claude-agent-acp@latest".to_string(),
            ),
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
        assert_eq!(DefaultPermissionMode::Auto.mode_id(), "auto");
        assert_eq!(DefaultPermissionMode::Default.mode_id(), "default");
        assert_eq!(DefaultPermissionMode::AcceptEdits.mode_id(), "acceptEdits");
        assert_eq!(DefaultPermissionMode::Plan.mode_id(), "plan");
        assert_eq!(DefaultPermissionMode::DontAsk.mode_id(), "dontAsk");
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
    fn connect_mode_priority_appends_auto_fallback() {
        // The configured mode is tried first, then the auto fallback.
        assert_eq!(
            DefaultPermissionMode::BypassPermissions.connect_mode_priority(),
            vec!["bypassPermissions", "auto"]
        );
        assert_eq!(
            DefaultPermissionMode::DontAsk.connect_mode_priority(),
            vec!["dontAsk", "auto"]
        );
        // When the configured mode already is the fallback, it appears once.
        assert_eq!(
            DefaultPermissionMode::Auto.connect_mode_priority(),
            vec!["auto"]
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
            (DefaultPermissionMode::Auto, "auto"),
            (DefaultPermissionMode::Default, "default"),
            (DefaultPermissionMode::AcceptEdits, "acceptEdits"),
            (DefaultPermissionMode::Plan, "plan"),
            (DefaultPermissionMode::DontAsk, "dontAsk"),
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
            d.launch,
            AgentLaunch::Raw("npx -y @agentclientprotocol/claude-agent-acp@latest".to_string())
        );
    }

    #[test]
    fn codex_default_has_expected_fields() {
        let d = AgentDefinition::codex_default();
        assert_eq!(d.id, "codex-acp");
        assert_eq!(d.name, "Codex");
        assert_eq!(
            d.launch,
            AgentLaunch::Raw("npx -y @agentclientprotocol/codex-acp@latest".to_string())
        );
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
            let AgentLaunch::Raw(command) = &p.launch else {
                return false;
            };
            p.id == "factory-droid" && command.starts_with("DROID_DISABLE_AUTO_UPDATE=true ")
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
            launch: AgentLaunch::Raw("codex acp".to_string()),
        };
        let toml_str = toml::to_string(&d).expect("serialize");
        let back: AgentDefinition = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(back, d);
    }

    #[test]
    fn migration_command_toml_deserializes_to_raw_unchanged() {
        // Every pre-migration config line, including a hand-written {{cwd}}
        // token, must land in `Raw` byte-for-byte.
        let toml_str = "id = \"legacy\"\nname = \"Legacy\"\ncommand = \"ssh vm-work \\\"cd {{cwd}} && run\\\"\"\n";
        let d: AgentDefinition = toml::from_str(toml_str).expect("deserialize");
        assert_eq!(d.id, "legacy");
        assert_eq!(d.name, "Legacy");
        assert_eq!(
            d.launch,
            AgentLaunch::Raw("ssh vm-work \"cd {{cwd}} && run\"".to_string())
        );

        // And it serializes back to the exact same flat `command` shape.
        let toml_str = toml::to_string(&d).expect("serialize");
        assert!(toml_str.contains("command = "));
        assert!(!toml_str.contains("[ssh]"));
        assert!(!toml_str.contains("[docker]"));
    }

    #[test]
    fn ssh_launch_toml_round_trips() {
        let d = AgentDefinition {
            id: "remote-agent".to_string(),
            name: "Remote Agent".to_string(),
            launch: AgentLaunch::Ssh {
                adapter_command: "npx -y some-acp".to_string(),
                host: "vm-work".to_string(),
            },
        };
        let toml_str = toml::to_string(&d).expect("serialize");
        assert!(toml_str.contains("[ssh]"));
        assert!(toml_str.contains("host = \"vm-work\""));
        // No flat `command` key — only `adapter_command` inside `[ssh]`.
        assert!(!toml_str.contains("\ncommand = "));
        let back: AgentDefinition = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(back, d);
    }

    #[test]
    fn docker_launch_toml_round_trips() {
        let d = AgentDefinition {
            id: "docker-agent".to_string(),
            name: "Docker Agent".to_string(),
            launch: AgentLaunch::Docker {
                adapter_command: "npx -y some-acp".to_string(),
                container: "ubuntu-dev".to_string(),
            },
        };
        let toml_str = toml::to_string(&d).expect("serialize");
        assert!(toml_str.contains("[docker]"));
        assert!(toml_str.contains("container = \"ubuntu-dev\""));
        // No flat `command` key — only `adapter_command` inside `[docker]`.
        assert!(!toml_str.contains("\ncommand = "));
        let back: AgentDefinition = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(back, d);
    }

    #[test]
    fn repr_priority_prefers_ssh_over_docker_over_command_when_hand_edited() {
        // Defensive priority order for a hand-edited config that sets more
        // than one launch shape at once: ssh > docker > command.
        let both: AgentDefinition = toml::from_str(
            "id = \"x\"\nname = \"X\"\ncommand = \"legacy\"\n\
             [docker]\nadapter_command = \"d\"\ncontainer = \"c\"\n\
             [ssh]\nadapter_command = \"s\"\nhost = \"h\"\n",
        )
        .expect("deserialize");
        assert_eq!(
            both.launch,
            AgentLaunch::Ssh {
                adapter_command: "s".to_string(),
                host: "h".to_string(),
            }
        );

        let docker_and_command: AgentDefinition = toml::from_str(
            "id = \"x\"\nname = \"X\"\ncommand = \"legacy\"\n\
             [docker]\nadapter_command = \"d\"\ncontainer = \"c\"\n",
        )
        .expect("deserialize");
        assert_eq!(
            docker_and_command.launch,
            AgentLaunch::Docker {
                adapter_command: "d".to_string(),
                container: "c".to_string(),
            }
        );
    }

    #[test]
    fn needs_remote_cwd_raw_detects_token() {
        assert!(
            AgentLaunch::Raw(
                "ssh vm-work \"cd {{cwd}} && npx -y @agentclientprotocol/claude-agent-acp@latest\""
                    .to_string()
            )
            .needs_remote_cwd()
        );
    }

    #[test]
    fn needs_remote_cwd_raw_false_without_token() {
        assert!(
            !AgentLaunch::Raw("npx -y @agentclientprotocol/claude-agent-acp@latest".to_string())
                .needs_remote_cwd()
        );
    }

    #[test]
    fn needs_remote_cwd_ssh_and_docker_are_always_true() {
        assert!(
            AgentLaunch::Ssh {
                adapter_command: "run".to_string(),
                host: "h".to_string(),
            }
            .needs_remote_cwd()
        );
        assert!(
            AgentLaunch::Docker {
                adapter_command: "run".to_string(),
                container: "c".to_string(),
            }
            .needs_remote_cwd()
        );
    }

    #[test]
    fn wrap_raw_without_token_ignores_remote_path() {
        let launch = AgentLaunch::Raw("npx -y some-acp".to_string());
        assert_eq!(launch.wrap(None), Ok("npx -y some-acp".to_string()));
        assert_eq!(
            launch.wrap(Some("/tmp/anything")),
            Ok("npx -y some-acp".to_string())
        );
    }

    #[test]
    fn wrap_raw_with_token_substitutes_remote_path() {
        let launch = AgentLaunch::Raw("ssh vm-work \"cd {{cwd}} && run\"".to_string());
        assert_eq!(
            launch.wrap(Some("/home/user/project")),
            Ok("ssh vm-work \"cd /home/user/project && run\"".to_string())
        );
    }

    #[test]
    fn wrap_raw_with_token_errs_on_missing_or_blank_remote_path() {
        let launch = AgentLaunch::Raw("cd {{cwd}} && run".to_string());
        assert_eq!(launch.wrap(None), Err(()));
        assert_eq!(launch.wrap(Some("")), Err(()));
        assert_eq!(launch.wrap(Some("   ")), Err(()));
    }

    #[test]
    fn wrap_ssh_builds_exact_command() {
        let launch = AgentLaunch::Ssh {
            adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest".to_string(),
            host: "vm-work".to_string(),
        };
        assert_eq!(
            launch.wrap(Some("/home/user/project")),
            Ok(
                "ssh vm-work sh -c 'cd \"/home/user/project\" && npx -y @agentclientprotocol/claude-agent-acp@latest'"
                    .to_string()
            )
        );
    }

    #[test]
    fn wrap_ssh_errs_on_missing_or_blank_remote_path() {
        let launch = AgentLaunch::Ssh {
            adapter_command: "run".to_string(),
            host: "vm-work".to_string(),
        };
        assert_eq!(launch.wrap(None), Err(()));
        assert_eq!(launch.wrap(Some("  ")), Err(()));
    }

    #[test]
    fn wrap_docker_builds_exact_command_with_dash_i_never_dash_it() {
        let launch = AgentLaunch::Docker {
            adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest".to_string(),
            container: "ubuntu-dev".to_string(),
        };
        let wrapped = launch.wrap(Some("/home/user/project")).unwrap();
        assert_eq!(
            wrapped,
            "docker exec -i ubuntu-dev sh -c 'cd \"/home/user/project\" && npx -y @agentclientprotocol/claude-agent-acp@latest'"
        );
        assert!(wrapped.contains(" -i "));
        assert!(!wrapped.contains(" -it "));
    }

    #[test]
    fn wrap_docker_errs_on_missing_or_blank_remote_path() {
        let launch = AgentLaunch::Docker {
            adapter_command: "run".to_string(),
            container: "ubuntu-dev".to_string(),
        };
        assert_eq!(launch.wrap(None), Err(()));
        assert_eq!(launch.wrap(Some("")), Err(()));
    }

    #[test]
    fn wrap_with_env_prefixes_raw_command() {
        // The value carries a space — like a real account config dir under
        // `default_data_dir()`, which on macOS lands under `~/Library/
        // Application Support` — so the assertion also guards that the
        // emitted prefix is single-quoted, not just concatenated.
        use crate::account_env::AccountEnv;
        let launch = AgentLaunch::Raw("npx -y some-acp".to_string());
        let env = AccountEnv {
            inject: vec![(
                "CLAUDE_CONFIG_DIR".into(),
                "/Users/x/Library/Application Support/daruda/acc/alice".into(),
            )],
            strip: vec!["ANTHROPIC_API_KEY"],
        };
        let cmd = launch.wrap_with_env(None, &env).unwrap();
        assert!(cmd.starts_with(
            "CLAUDE_CONFIG_DIR='/Users/x/Library/Application Support/daruda/acc/alice' "
        ));
        assert!(cmd.contains("npx -y some-acp"));
    }

    #[test]
    fn wrap_with_env_ssh_exports_and_unsets() {
        use crate::account_env::AccountEnv;
        let launch = AgentLaunch::Ssh {
            adapter_command: "npx -y some-acp".to_string(),
            host: "vm".to_string(),
        };
        let env = AccountEnv {
            inject: vec![("CLAUDE_CONFIG_DIR".into(), "/remote/acc".into())],
            strip: vec!["ANTHROPIC_API_KEY"],
        };
        let cmd = launch.wrap_with_env(Some("/work"), &env).unwrap();
        assert!(cmd.contains("export CLAUDE_CONFIG_DIR=\"/remote/acc\""));
        assert!(cmd.contains("unset ANTHROPIC_API_KEY"));
    }

    #[test]
    fn login_command_appends_cli_login_for_raw_only() {
        let raw = AgentLaunch::Raw("npx -y @agentclientprotocol/claude-agent-acp@latest".into());
        assert_eq!(
            raw.login_command().as_deref(),
            Some("npx -y @agentclientprotocol/claude-agent-acp@latest --cli auth login --claudeai")
        );
        let ssh = AgentLaunch::Ssh {
            adapter_command: "x".into(),
            host: "h".into(),
        };
        assert_eq!(ssh.login_command(), None);
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
