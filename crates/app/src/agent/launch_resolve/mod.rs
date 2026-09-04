//! Resolving how an ACP agent is launched: the adapter command line, the
//! working directory it is rooted at, the environment the spawned process
//! must not inherit, and the session-host correction a stale lane owes.
//!
//! GPUI-free by construction — every function here is pure, so both the
//! connect pump and the flow-submission path (both inside the app) can
//! share one resolution.

#[cfg(test)]
mod tests;

use crate::agent::account::PreparedAccount;
use crate::lane::{Lane, session_host};
use daruda_config::{
    AccountEnv, AgentDefinition, AgentLaunch, SessionHostEntry, SessionHostTombstone,
};
use daruda_store::project::{LaneSessionHost, PaneCwd};
use std::path::{Path, PathBuf};

/// How one catalog agent launches: the adapter command shape plus the
/// environment its definition declares for the adapter process.
///
/// The two travel together because every launch site needs both, and looking
/// them up separately would let a stale id hand back one agent's command with
/// another's environment.
///
/// `env` is flattened to a plain list here: the definition's "states none"
/// and "states an empty one" both spawn the same process, and that
/// distinction only has to survive persistence, not launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentLaunchSpec {
    pub(crate) launch: AgentLaunch,
    pub(crate) env: Vec<(String, String)>,
}

impl AgentLaunchSpec {
    pub(crate) fn of(definition: &AgentDefinition) -> Self {
        Self {
            launch: definition.launch.clone(),
            env: definition.env.clone().unwrap_or_default(),
        }
    }
}

/// Why [`resolve_session_command`] could not build a connect command —
/// distinct reasons because they need distinct messages (`connect_agent_chat`
/// maps each to its own status-line string; see that match).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectCommandError {
    /// The legacy `Raw` + `{{cwd}}` escape hatch has no usable remote path
    /// to substitute (`pane_cwd` isn't `Remote`, or its path is blank).
    NoRemotePath,
    /// `launch` is a JSON stdio config and the resolved host is remote —
    /// `session_host::wrap` can only fold a shell command line into
    /// `ssh`/`docker`, and raw JSON isn't one. See
    /// `AgentLaunch::is_json_stdio`'s doc.
    JsonStdioRemote,
    /// `launch` is a JSON stdio config and something declared an environment
    /// for it. Every transport applies `env` as *shell* text — a `KEY='v' `
    /// assignment prefix locally, `export KEY='v'; ` remotely — so folding it
    /// in front of raw JSON produces
    /// `CODEX_CONFIG='…' {"command":"/usr/bin/foo"}`, which both downstream
    /// discriminators (`AcpAgent::from_str`, `command_needs_node`) then read
    /// as a shell command line whose program is named `{command:/usr/bin/foo}`.
    ///
    /// Refused rather than merged: a JSON stdio config carries its own `env`
    /// field, so the environment has a lossless place to go that no
    /// shell-string edit can reach. See [`json_stdio_refusal`].
    JsonStdioEnv,
}

/// Pure core of the `command`/cwd resolution inside
/// [`Workspace::connect_agent_chat`]. `owning_lane` is the pane's owning lane
/// (`None` when it could not be found — falls back to `Local`, never
/// promotes to remote; see the design's B′ note on restored remote panes).
///
/// `resolved_host` is `owning_lane`'s host, already resolved against the
/// live registry catalog/tombstones by the caller (`owning_lane.map(|lane|
/// lane.effective_session_host(...))`) — passed in rather than recomputed
/// here so the one connect only walks the registry chase once, and so this
/// function stays pure over plain values instead of also taking the
/// catalog/tombstone slices.
///
/// Returns the resolved [`PaneCwd`] unconditionally (even on `Err`) — the
/// caller writes it back into the pane's own `cwd` so a pane whose lane's
/// host changed doesn't keep reporting the host it was created with (see
/// that call site's comment).
///
/// `pane_cwd` is consulted only for the legacy `Raw` + `{{cwd}}`-token escape
/// hatch (`AgentLaunch::CWD_TOKEN`), which predates the lane session-host
/// axis entirely and is untouched by it — see
/// [`session_host::adapter_command`]'s doc. Every other launch shape
/// resolves **both** the command and the cwd from the lane, not the
/// pane's own (possibly stale) persisted `cwd` — a restored remote pane
/// whose owning lane's host later changes follows the lane on its next
/// connect, not a frozen snapshot.
fn resolve_session_command(
    launch: &AgentLaunch,
    owning_lane: Option<&Lane>,
    pane_cwd: &PaneCwd,
    env: &AccountEnv,
    resolved_host: Option<&LaneSessionHost>,
) -> (Result<String, ConnectCommandError>, PaneCwd) {
    let is_raw_token = matches!(launch, AgentLaunch::Raw(command) if command.contains(daruda_config::agent::CWD_TOKEN));
    if is_raw_token {
        let (remote_path, resolved_cwd) = match pane_cwd {
            PaneCwd::Remote(remote_path) => (
                Some(remote_path.as_str()),
                PaneCwd::Remote(remote_path.clone()),
            ),
            PaneCwd::Local(path) => (None, PaneCwd::Local(path.clone())),
        };
        let wrapped = match json_stdio_refusal(launch, env, false) {
            Some(err) => Err(err),
            None => launch
                .wrap_with_env(remote_path, env)
                .map_err(|()| ConnectCommandError::NoRemotePath),
        };
        return (wrapped, resolved_cwd);
    }

    let host = resolved_host.cloned().unwrap_or(LaneSessionHost::Local);
    let resolved_cwd =
        match host.session_path() {
            Some(remote_path) => PaneCwd::Remote(remote_path.to_string()),
            // `Local`: the lane's own current path when it's known, else the
            // pane's own last-persisted local value (owning lane not found —
            // there is no better answer, see the module doc on that gate).
            None => PaneCwd::Local(owning_lane.map(|lane| lane.path.clone()).unwrap_or_else(
                || {
                    pane_cwd
                        .as_local()
                        .map(Path::to_path_buf)
                        .unwrap_or_default()
                },
            )),
        };
    if let Some(err) = json_stdio_refusal(launch, env, host.is_remote()) {
        return (Err(err), resolved_cwd);
    }
    let adapter = session_host::adapter_command(launch);
    (
        Ok(session_host::wrap_with_env(&host, adapter, env)),
        resolved_cwd,
    )
}

/// Whether a JSON stdio `launch` has to be refused before assembly, and why.
///
/// A JSON stdio config carries its command, args and env as structured
/// fields, while every assembly path here edits a *shell string*. There are
/// exactly two edits, and each corrupts it:
///
/// - **`is_remote`** — the remote arms splice the command into
///   `ssh … sh -c 'cd "…" && <command>'`. Raw JSON is not a command line the
///   remote shell can run.
/// - **A non-empty `env.inject`** — every arm prepends the environment as
///   shell text (`KEY='v' ` locally, `export KEY='v'; ` remotely), so the
///   assembled string stops being JSON. `AcpAgent::from_str` and
///   `daruda_acp::node::command_needs_node` both discriminate on
///   `trim_start().starts_with('{')`, so it silently takes the shell path and
///   daruda tries to exec a program literally named
///   `{command:/usr/bin/foo,args:[acp]}`.
///
/// `env.strip` needs no arm: it is never applied here (the local prefix omits
/// it by design, and `daruda_acp::launch_env::prefix_with_env_unsets` is
/// reached only with a managed account, which
/// [`AgentLaunch::account_recipe`] already declines for a JSON stdio config).
///
/// Refusing rather than merging into the JSON's own `env` map is deliberate.
/// Merging would need a second owner of the JSON shape outside `daruda_acp`
/// — `daruda_config`'s assembler is GPUI-free and deliberately parses no JSON
/// — or a `LaunchSpec` that carries the environment separately all the way
/// down. Neither buys the user anything a one-line config edit does not: the
/// JSON config already has an `env` field, which is both lossless and the
/// place the value belongs. The message says so.
fn json_stdio_refusal(
    launch: &AgentLaunch,
    env: &AccountEnv,
    is_remote: bool,
) -> Option<ConnectCommandError> {
    if !launch.is_json_stdio() {
        return None;
    }
    if is_remote {
        return Some(ConnectCommandError::JsonStdioRemote);
    }
    (!env.inject.is_empty()).then_some(ConnectCommandError::JsonStdioEnv)
}

/// `host` (as [`Lane::effective_session_host`] resolved it) with its
/// `registry_id` corrected to the id the link currently resolves to —
/// [`session_host::resolved_registry_id`]'s value folded back onto the
/// host's own field. `effective_session_host` deliberately leaves a
/// tombstone-redirected id untouched (see its doc); this is the write-back
/// half `connect_agent_chat` persists so a future connect resolves directly
/// against the catalog instead of re-chasing the tombstone every time. A
/// no-op for `Local` or an unlinked/orphaned host.
fn sync_registry_id(
    host: LaneSessionHost,
    catalog: &[SessionHostEntry],
    tombstones: &[SessionHostTombstone],
) -> LaneSessionHost {
    let Some(fresh_id) = session_host::resolved_registry_id(&host, catalog, tombstones) else {
        return host;
    };
    match host {
        LaneSessionHost::Ssh {
            target,
            session_path,
            ..
        } => LaneSessionHost::Ssh {
            target,
            session_path,
            registry_id: Some(fresh_id),
        },
        LaneSessionHost::Docker {
            container,
            session_path,
            ..
        } => LaneSessionHost::Docker {
            container,
            session_path,
            registry_id: Some(fresh_id),
        },
        LaneSessionHost::Local => LaneSessionHost::Local,
    }
}

/// Pure core of `connect_agent_chat`'s session-host write-back: `Some(host)`
/// to persist onto the lane, `None` when there's nothing to sync.
///
/// `None` in two cases: `cached` is `None` — the lane never explicitly
/// answered the host question, so its resolution falls back to the legacy
/// `remote_cwd` pair, which must stay unanswered (see
/// `Lane::set_session_host`'s doc) rather than being silently promoted to an
/// explicit answer by this sync; or `resolved` is `None` — no owning lane
/// was found for this connect at all. Otherwise compares `cached` against
/// `resolved` corrected by [`sync_registry_id`] (folding in a tombstone
/// redirect's fresh id alongside the catalog's current `target`/`container`)
/// and returns the corrected value only when it actually differs.
fn session_host_write_back(
    cached: Option<&LaneSessionHost>,
    resolved: Option<&LaneSessionHost>,
    catalog: &[SessionHostEntry],
    tombstones: &[SessionHostTombstone],
) -> Option<LaneSessionHost> {
    let cached = cached?;
    let resolved = resolved?;
    let corrected = sync_registry_id(resolved.clone(), catalog, tombstones);
    (&corrected != cached).then_some(corrected)
}

/// The account recipe for `launch` on this connect, given `is_remote` (a
/// lane-verified locality this call site already has — see the doc on
/// `connect_agent_chat`'s `is_remote`). Handles the one case
/// [`AgentLaunch::account_recipe`] can't: a deprecated `Ssh`/`Docker` launch
/// this lane resolves to `Local` derives its recipe from `adapter_command`
/// directly, since `account_recipe` itself has no lane to verify locality
/// against and must stay conservative for its other three callers (see that
/// method's doc).
pub(crate) fn account_recipe_for_connect(
    launch: &AgentLaunch,
    is_remote: bool,
) -> Option<daruda_store::accounts::AccountRecipeId> {
    match launch {
        AgentLaunch::Ssh {
            adapter_command, ..
        }
        | AgentLaunch::Docker {
            adapter_command, ..
        } if !is_remote => daruda_config::account_recipe_for_local_command(adapter_command),
        _ => launch.account_recipe(is_remote),
    }
}

/// The `PathBuf` `connect_agent_session` needs for `cwd`, from a resolved
/// [`PaneCwd`]. For `Remote` this just re-wraps the opaque remote-path
/// string — the ACP wire protocol's `cwd` field is a plain path string
/// regardless of which machine it names, so this isn't a "real" local
/// `PathBuf`, only the shape the protocol call needs.
fn connect_wire_path(cwd: &PaneCwd) -> PathBuf {
    match cwd {
        PaneCwd::Local(path) => path.clone(),
        PaneCwd::Remote(remote_path) => PathBuf::from(remote_path),
    }
}

/// The ambient auth-override vars to unset for this pane's adapter, taken
/// from the resolved account's recipe. Empty for the System account, which
/// is defined by running under whatever the user's own environment says.
fn account_strip_env(prepared: Option<&PreparedAccount>) -> Vec<String> {
    prepared
        .map(|account| {
            account
                .env
                .strip
                .iter()
                .map(|name| (*name).to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The environment one adapter process launches with: what the agent's own
/// definition declares, with the account's environment layered over it.
///
/// **The account wins a key collision** — its vars scope credentials, and an
/// agent definition must not be able to redirect them. That covers the vars
/// the account strips as well as the ones it injects: a stripped var is one
/// the account is scoping too.
///
/// Deduplicated here rather than left to the shell's last-assignment-wins,
/// because the remote path emits `export` statements and the local path a
/// command prefix — a rule that lived in either shell would be invisible and
/// transport-dependent. Surviving agent entries keep their declared order and
/// precede the account's, so the assembled string is deterministic.
///
/// `None` is the System account, which by definition adds nothing: the result
/// is then the agent's declaration alone, and for an agent declaring nothing,
/// [`AccountEnv::ambient`].
fn launch_env(agent_env: &[(String, String)], account: Option<&AccountEnv>) -> AccountEnv {
    let account = account.cloned().unwrap_or_else(AccountEnv::ambient);
    let mut inject: Vec<(String, String)> = agent_env
        .iter()
        .filter(|(key, _)| {
            !account.inject.iter().any(|(scoped, _)| scoped == key)
                && !account.strip.iter().any(|scoped| scoped == key)
        })
        .cloned()
        .collect();
    inject.extend(account.inject);
    AccountEnv {
        inject,
        strip: account.strip,
    }
}

/// Everything one ACP launch needs, resolved together.
///
/// `spec` is a `Result` because two launch shapes cannot produce a command
/// at all (see [`ConnectCommandError`]); the other three fields are always
/// meaningful, so the caller can still show the resolved directory while
/// reporting the command failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedLaunch {
    pub(crate) spec: Result<daruda_acp::LaunchSpec, ConnectCommandError>,
    /// The working directory the launch actually resolved to, which may
    /// differ from the caller's cached one when the lane's host moved.
    pub(crate) resolved_cwd: PaneCwd,
    /// The `PathBuf` shape the ACP `cwd` field takes — for a remote lane
    /// this just wraps the opaque remote-path string, not a real local path.
    pub(crate) wire_cwd: PathBuf,
    /// `Some` when the caller's cached host is stale and owes a write-back.
    pub(crate) host_write_back: Option<LaneSessionHost>,
}

/// Resolve one launch end to end. Pure: the caller has already prepared the
/// account directory (that step needs I/O and a UI error path), and passes
/// the result in.
#[allow(clippy::too_many_arguments)] // A resolution funnel: bundling these
// into a struct would only move the same nine values to the call site.
pub(crate) fn resolve_launch(
    launch: &AgentLaunch,
    agent_env: &[(String, String)],
    owning_lane: Option<&Lane>,
    pane_cwd: &PaneCwd,
    prepared: Option<&PreparedAccount>,
    cached_host: Option<&LaneSessionHost>,
    resolved_host: Option<&LaneSessionHost>,
    catalog: &[SessionHostEntry],
    tombstones: &[SessionHostTombstone],
) -> ResolvedLaunch {
    // The one site that knows the agent/account precedence rule — both the
    // managed-account and System-account paths run through it.
    let env = launch_env(agent_env, prepared.map(|p| &p.env));
    let (command, resolved_cwd) =
        resolve_session_command(launch, owning_lane, pane_cwd, &env, resolved_host);
    let wire_cwd = connect_wire_path(&resolved_cwd);
    let host_write_back = session_host_write_back(cached_host, resolved_host, catalog, tombstones);
    let spec = command.map(|command| daruda_acp::LaunchSpec {
        command,
        strip_env: account_strip_env(prepared),
    });
    ResolvedLaunch {
        spec,
        resolved_cwd,
        wire_cwd,
        host_write_back,
    }
}
