pub mod entry;
pub mod preset;
#[cfg(test)]
mod tests;
pub mod vocabulary;

use daruda_store::accounts::AccountRecipeId;
use serde::{Deserialize, Serialize};

pub use entry::{AgentEntry, PresetOverrides};
pub use preset::{
    ACP_REGISTRY_URL, ACP_REGISTRY_VERSION, AgentPreset, PresetLaunchability,
    preset as agent_preset, presets as agent_presets,
};
pub use vocabulary::{AgentVocabularySeed, seed_for_command as agent_vocabulary_seed};

/// A selectable ACP agent: an id, a display name, and how its ACP adapter is
/// launched.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(from = "AgentDefinitionRepr", into = "AgentDefinitionRepr")]
pub struct AgentDefinition {
    pub id: String,
    pub name: String,
    pub launch: AgentLaunch,
    /// Session mode to request when a fresh session with this agent connects.
    ///
    /// A free-form id rather than an enum because modes are agent-advertised:
    /// this catalog holds Cline, Codex and a dozen other adapters whose
    /// vocabularies daruda cannot enumerate, and even one agent's list varies
    /// per model. An id the agent doesn't advertise is skipped at connect,
    /// falling through to the adapter's own default.
    pub default_mode: Option<String>,
    /// Model id to request when a fresh session with this agent connects.
    ///
    /// A free-form id rather than an enum because model ids are
    /// runtime/account/plan dependent: Claude resolves its list from the SDK
    /// plus a settings allowlist, Codex fetches its list from its backend,
    /// and neither is knowable ahead of time from this catalog alone. An id
    /// the agent doesn't advertise is simply skipped at connect.
    pub default_model: Option<String>,
    /// Fold rules a fresh chat pane under this agent starts on. This entry is
    /// the default a pane returns to; `None` means the built-in matrix, which
    /// an empty list also resolves to.
    pub fold_mode: Option<Vec<String>>,
    /// Trailing-step window a fresh chat pane under this agent starts on.
    /// `None` means [`TAIL_WINDOW_DEFAULT`].
    pub tail_window: Option<u8>,
    /// Visible row kinds a fresh chat pane under this agent starts on. `None`
    /// means the unfiltered set; unlike `fold_mode`, an empty list is a real
    /// value naming an empty visible set, so the two cannot be collapsed.
    pub display_filter: Option<Vec<String>>,
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

/// Package-name fragments that identify an adapter's auth domain inside a
/// launch command. Matched as substrings so a version pin
/// (`codex-acp@1.1.0`), an env prefix, or an `npx -y` wrapper all still
/// resolve; agents sharing an adapter therefore share credentials.
const CLAUDE_ADAPTER_MARKERS: &[&str] = &["claude-agent-acp", "claude-code-acp"];
const CODEX_ADAPTER_MARKERS: &[&str] = &["codex-acp"];

/// Leading character that marks a [`AgentLaunch::Raw`] command as a JSON
/// stdio config rather than a shell command line — the same discrimination
/// `daruda_acp`'s adapter parser makes.
const JSON_STDIO_PREFIX: char = '{';

/// Whether `command` is a JSON stdio config. Such a command carries its
/// program, args and env as structured fields, so the shell-string edits in
/// [`AgentLaunch::wrap_with_env`] and [`AgentLaunch::login_command`] would
/// corrupt it — both are gated on this, and it also bars the launch from
/// carrying a managed account at all.
fn is_json_stdio_str(command: &str) -> bool {
    command.trim_start().starts_with(JSON_STDIO_PREFIX)
}

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

    /// Like [`Self::wrap`], but applies `env` for the spawned process.
    ///
    /// `Ssh`/`Docker` fold both halves into the remote shell via
    /// `unset`/`export`. `Raw` emits **only** the `env.inject` `KEY=value`
    /// prefix: the string must stay parseable by
    /// `daruda_acp::node::command_needs_node`, which reads the launcher token
    /// after the assignments, and an `/usr/bin/env -u …` prefix here would
    /// hide it and skip Node.js provisioning. `env.strip` is applied instead
    /// at final launch assembly, in
    /// `daruda_acp::launch_env::prepare_adapter_command`, once the runtime is
    /// resolved.
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

    /// Whether this launch is a JSON stdio config (`{"command": ...}`)
    /// rather than a shell command line — the same discrimination
    /// `daruda_acp`'s adapter parser makes. Only `Raw` can carry one;
    /// `Ssh`/`Docker`'s `adapter_command` is always a shell command (it gets
    /// wrapped in `sh -c '...'`), so this is always `false` for them.
    pub fn is_json_stdio(&self) -> bool {
        match self {
            AgentLaunch::Raw(command) => is_json_stdio_str(command),
            AgentLaunch::Ssh { .. } | AgentLaunch::Docker { .. } => false,
        }
    }

    /// The auth domain a managed account for this agent belongs to, keyed by
    /// adapter rather than agent id so two catalog entries running the same
    /// adapter share one set of credentials. `None` for a remote launch (no
    /// local browser to complete OAuth in), for a JSON stdio config (see
    /// [`Self::is_json_stdio`]), and for an unrecognized adapter.
    ///
    /// `is_remote`: whether the *caller's* context — the lane a pane is
    /// attached to, not this launch's own shape — is remote. A `Raw`
    /// command carries no host of its own once the session-host axis lives
    /// on the lane, so `Ssh`/`Docker`/the legacy `{{cwd}}` token remain the
    /// only self-describing remote shapes; `is_remote` is how a lane-aware
    /// caller closes the rest: a managed account's config dir is a local
    /// path, and injecting one into a command that actually runs elsewhere
    /// would point the remote adapter at a directory that doesn't exist
    /// there. A caller with no lane in scope (the catalog-wide login-command
    /// scan) passes `false` — there is no lane to ask, and unlike Ssh/Docker,
    /// a bare `Raw` command cannot self-report as remote-only.
    ///
    /// Deliberately unconditional for `Ssh`/`Docker` here (via
    /// `needs_remote_cwd()`), even when `is_remote` is `false`: a caller
    /// without a real lane to check against (this fn's other three callers)
    /// cannot tell "verified local" from "don't know," so this stays
    /// conservative. A caller that *did* verify locality through a lane —
    /// `effective_session_host(..).is_remote() == false` — should call
    /// [`account_recipe_for_local_command`] on
    /// `lane::session_host::adapter_command(launch)` instead, which is what
    /// lets a deprecated `Ssh`/`Docker` launch that now resolves `Local`
    /// still seed a managed account like any other local one.
    pub fn account_recipe(&self, is_remote: bool) -> Option<AccountRecipeId> {
        let AgentLaunch::Raw(command) = self else {
            return None;
        };
        if is_remote || self.needs_remote_cwd() {
            return None;
        }
        account_recipe_for_local_command(command)
    }

    /// The interactive login command for this agent, or `None` for remote
    /// launches (SSH/Docker) where a local desktop browser can't complete
    /// OAuth. `login_args` comes from the auth domain's `AccountRecipe`,
    /// which owns the exact flag text; this only joins the two.
    pub fn login_command(&self, login_args: &str) -> Option<String> {
        match self {
            AgentLaunch::Raw(command) => Some(format!("{command} {login_args}")),
            AgentLaunch::Ssh { .. } | AgentLaunch::Docker { .. } => None,
        }
    }
}

/// The auth domain a *known-local* `command` derives to — `None` for a JSON
/// stdio config or an unrecognized adapter. The command-matching half of
/// [`AgentLaunch::account_recipe`], factored out so a caller holding a real
/// lane can apply it to `lane::session_host::adapter_command(launch)`
/// directly: the bare adapter command regardless of which `AgentLaunch`
/// variant carries it, once that caller has already confirmed via
/// `effective_session_host` that this specific launch is running locally
/// right now. `account_recipe` itself stays conservative for callers with no
/// lane to check against — see its doc.
pub fn account_recipe_for_local_command(command: &str) -> Option<AccountRecipeId> {
    if is_json_stdio_str(command) {
        return None;
    }
    if CLAUDE_ADAPTER_MARKERS.iter().any(|m| command.contains(m)) {
        Some(AccountRecipeId::Claude)
    } else if CODEX_ADAPTER_MARKERS.iter().any(|m| command.contains(m)) {
        Some(AccountRecipeId::Codex)
    } else {
        None
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
    default_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fold_mode: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tail_window: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display_filter: Option<Vec<String>>,
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
            default_mode: v.default_mode,
            default_model: v.default_model,
            fold_mode: v.fold_mode,
            tail_window: v.tail_window,
            display_filter: v.display_filter,
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
            default_mode: v.default_mode,
            default_model: v.default_model,
            fold_mode: v.fold_mode,
            tail_window: v.tail_window,
            display_filter: v.display_filter,
        }
    }
}

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
            default_mode: None,
            default_model: None,
            fold_mode: None,
            tail_window: None,
            display_filter: None,
        }
    }

    /// The built-in Codex ACP agent preset exposed by the Settings UI.
    pub fn codex_default() -> Self {
        Self::registry_preset("codex-acp").expect("codex-acp preset is Runnable")
    }

    /// The catalog entry for preset `id`, or `None` when no preset carries that
    /// id or the one that does needs a manual install first. There is
    /// deliberately no plural counterpart: a runnable-only list is what kept the
    /// other 15 presets out of the Settings dropdown, which now walks
    /// [`preset::presets`] and says why a preset cannot be added.
    pub fn registry_preset(id: &str) -> Option<Self> {
        preset::preset(id).and_then(AgentPreset::definition)
    }
}

/// Default agent catalog: a single Claude entry. Used as the serde field default
/// (missing `[[agents]]`) AND to normalize an explicitly-empty catalog.
///
/// Deliberately [`AgentEntry::Custom`], not a reference to the `claude-acp`
/// preset it shares a command with: `claude` is daruda's own stable id and every
/// AgentChat pane persists it, so the default must never resolve under the
/// preset's id instead.
pub(crate) fn default_agents() -> Vec<AgentEntry> {
    vec![AgentEntry::Custom(AgentDefinition::claude_default())]
}

/// Agent chat configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentConfig {
    /// Legacy global permission mode from pre per-agent-mode configs.
    /// Deserialized only so [`crate::Config::clamp`] can migrate it into the
    /// agent catalog; never written back.
    #[doc(hidden)]
    #[serde(default, rename = "default_permission_mode", skip_serializing)]
    pub legacy_default_permission_mode: Option<String>,
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
    /// Fixed content-column width used when an AgentChat pane is toggled into
    /// reading-width mode. Clamped to
    /// [`READING_WIDTH_MIN`]..=[`READING_WIDTH_MAX`] at load time.
    pub reading_width: f32,
    /// Session config options to hide from the input-dock chip row, matched by
    /// the option's advertised `description` (exact string). Presentation-only:
    /// the option stays in the session state and the agent can still change it.
    ///
    /// Exists because the protocol carries no structured "unusable" signal: an
    /// adapter that knows why an option can't be used folds the reason into the
    /// description text, and one that doesn't advertises the same description
    /// as the working state. Matching adapter-authored text is inherently
    /// loose: when an adapter rewords, the entry stops matching and the chip
    /// simply shows again.
    ///
    /// Defaults to [`FAST_MODE_PLAIN_DESCRIPTION`]; set
    /// `hidden_config_option_descriptions = []` to show every advertised option.
    pub hidden_config_option_descriptions: Vec<String>,
}

/// The Claude adapter's Fast mode description in its reason-less form — the
/// text it advertises both while the toggle works and while the CLI declines
/// it without a stated reason. Hidden by default because the toggle does not
/// stick through the ACP adapter path (the CLI reverts it on its next
/// `fast_mode_state` report), leaving a chip that silently flips back. The
/// reason-carrying variant ("… — not available on the free plan" etc.) is a
/// different string, so an adapter that can explain itself keeps its chip.
pub const FAST_MODE_PLAIN_DESCRIPTION: &str = "Faster responses on supported models";

/// Default for [`AgentConfig::hidden_config_option_descriptions`].
fn default_hidden_config_option_descriptions() -> Vec<String> {
    vec![FAST_MODE_PLAIN_DESCRIPTION.to_string()]
}

/// Minimum allowed value for `AgentConfig::input_max_rows`.
pub const INPUT_MAX_ROWS_MIN: u8 = 2;
/// Maximum allowed value for `AgentConfig::input_max_rows`.
pub const INPUT_MAX_ROWS_MAX: u8 = 20;
/// Default maximum rows for the bottom input before it scrolls.
pub const INPUT_MAX_ROWS_DEFAULT: u8 = 8;
/// Minimum readable-content column width for AgentChat.
pub const READING_WIDTH_MIN: f32 = 360.0;
/// Maximum readable-content column width for AgentChat.
pub const READING_WIDTH_MAX: f32 = 2400.0;
/// Default readable-content column width for AgentChat.
pub const READING_WIDTH_DEFAULT: f32 = 700.0;

/// Sentinel meaning every work step is visible.
pub const TAIL_WINDOW_ALL: u8 = 0;
/// Tail-window sizes offered by the AgentChat chip.
pub const TAIL_WINDOW_CHOICES: [u8; 4] = [1, 3, 5, 10];
pub const TAIL_WINDOW_DEFAULT: u8 = TAIL_WINDOW_ALL;

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            legacy_default_permission_mode: None,
            use_modifier_to_send: false,
            input_max_rows: INPUT_MAX_ROWS_DEFAULT,
            reading_width: READING_WIDTH_DEFAULT,
            hidden_config_option_descriptions: default_hidden_config_option_descriptions(),
        }
    }
}

impl AgentConfig {
    pub(crate) fn take_legacy_default_permission_mode(&mut self) -> Option<String> {
        self.legacy_default_permission_mode
            .take()
            .map(|mode| mode.trim().to_string())
            .filter(|mode| !mode.is_empty())
    }

    /// Clamp numeric fields to their valid ranges.
    pub fn clamp(&mut self) {
        self.input_max_rows = self
            .input_max_rows
            .clamp(INPUT_MAX_ROWS_MIN, INPUT_MAX_ROWS_MAX);
        self.reading_width = self
            .reading_width
            .clamp(READING_WIDTH_MIN, READING_WIDTH_MAX);
    }
}

/// Mode candidate to try when a fresh session connects: the agent's own
/// `default_mode`, when its catalog entry sets one. Empty when it doesn't —
/// the adapter's own default mode applies.
///
/// `daruda_acp` applies the first candidate the adapter both advertises and
/// accepts, so an agent whose vocabulary doesn't include this candidate falls
/// through to its own default. That is what makes a per-agent override safe
/// to state as free-form text.
pub fn connect_mode_priority(agent_default_mode: Option<&str>) -> Vec<String> {
    agent_default_mode
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| vec![id.to_string()])
        .unwrap_or_default()
}
