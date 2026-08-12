//! Resolving how an ACP agent is launched: the adapter command line, the
//! working directory it is rooted at, the environment the spawned process
//! must not inherit, and the session-host correction a stale lane owes.
//!
//! GPUI-free by construction — every function here is pure, so both the
//! connect pump and the flow-submission path (both inside the app) can
//! share one resolution.

use crate::agent::account::PreparedAccount;
use crate::lane::{Lane, session_host};
use daruda_config::{AccountEnv, AgentLaunch, SessionHostEntry, SessionHostTombstone};
use daruda_store::project::{LaneSessionHost, PaneCwd};
use std::path::{Path, PathBuf};

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
/// connect, not a frozen snapshot (the "의도된 행동 변화" the design calls
/// out for `edit_remote_cwd_hint`).
fn resolve_session_command(
    launch: &AgentLaunch,
    owning_lane: Option<&Lane>,
    pane_cwd: &PaneCwd,
    env: Option<&AccountEnv>,
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
        let wrapped = match env {
            Some(env) => launch.wrap_with_env(remote_path, env),
            None => launch.wrap(remote_path),
        }
        .map_err(|()| ConnectCommandError::NoRemotePath);
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
    // A JSON stdio config carries its command/args/env as structured
    // fields; `session_host::wrap` only knows how to splice a shell command
    // line into `ssh`/`docker`, so folding raw JSON in there would hand the
    // remote shell something it can't run. Local is unaffected — `wrap`
    // returns a JSON `Local` command unchanged, and `daruda_acp` parses it
    // exactly as it always has.
    if host.is_remote() && launch.is_json_stdio() {
        return (Err(ConnectCommandError::JsonStdioRemote), resolved_cwd);
    }
    let adapter = session_host::adapter_command(launch);
    let command = match env {
        Some(env) => session_host::wrap_with_env(&host, adapter, env),
        None => session_host::wrap(&host, adapter),
    };
    (Ok(command), resolved_cwd)
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
// into a struct would only move the same eight values to the call site.
pub(crate) fn resolve_launch(
    launch: &AgentLaunch,
    owning_lane: Option<&Lane>,
    pane_cwd: &PaneCwd,
    prepared: Option<&PreparedAccount>,
    cached_host: Option<&LaneSessionHost>,
    resolved_host: Option<&LaneSessionHost>,
    catalog: &[SessionHostEntry],
    tombstones: &[SessionHostTombstone],
) -> ResolvedLaunch {
    let (command, resolved_cwd) = resolve_session_command(
        launch,
        owning_lane,
        pane_cwd,
        prepared.map(|p| &p.env),
        resolved_host,
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::project::{LaneKind, LaneStatus};

    #[test]
    fn connect_wire_path_unwraps_a_remote_path_verbatim() {
        // `PaneCwd::Remote` carries a `String`, `Local` a `PathBuf`.
        let cwd = PaneCwd::Remote("/srv/work".to_string());
        assert_eq!(connect_wire_path(&cwd), PathBuf::from("/srv/work"));
    }

    /// A minimal lane at `path` for `resolve_session_command` tests — only
    /// `path` / `session_host` / `remote_cwd` matter to that function.
    fn lane_at(
        path: &str,
        session_host: Option<LaneSessionHost>,
        remote_cwd: Option<&str>,
    ) -> Lane {
        Lane {
            id: 1,
            kind: LaneKind::Default,
            path: PathBuf::from(path),
            name: None,
            tab_order: 0,
            is_unread: false,
            last_activity: 0,
            status: LaneStatus::Idle,
            is_main: false,
            base_ref: None,
            description: None,
            remote_cwd: remote_cwd.map(str::to_string),
            session_host,
            availability: crate::lane::availability::LaneAvailability::Present,
        }
    }

    fn raw(command: &str) -> AgentLaunch {
        AgentLaunch::Raw(command.to_string())
    }

    /// Mirrors the caller's own `resolved_host` computation
    /// (`connect_agent_chat`'s `owning_lane.map(|lane|
    /// lane.effective_session_host(...))`), so tests can pass
    /// `resolve_session_command` the same pre-resolved value it receives in
    /// production instead of catalog/tombstone slices.
    fn resolved(launch: &AgentLaunch, lane: Option<&Lane>) -> Option<LaneSessionHost> {
        lane.map(|lane| lane.effective_session_host(launch, &[], &[]))
    }

    /// The rev2 design's B′ regression: a restored `PaneCwd::Remote` pane
    /// (a stale path with no host of its own) whose launch is a
    /// host-agnostic `Raw` command must still attach to its lane's `Ssh`
    /// host — not silently fall back to local because the launch no longer
    /// names a host itself.
    #[test]
    fn restored_remote_pane_follows_the_lanes_ssh_host_not_a_local_fallback() {
        let lane = lane_at(
            "/local/checkout",
            Some(LaneSessionHost::Ssh {
                target: "vm-work".into(),
                session_path: "/srv/app".into(),
                registry_id: None,
            }),
            None,
        );
        let stale_cwd = PaneCwd::Remote("stale-unrelated-path".into());
        let launch = raw("npx acp-adapter");
        let (command, connect_cwd) = resolve_session_command(
            &launch,
            Some(&lane),
            &stale_cwd,
            None,
            resolved(&launch, Some(&lane)).as_ref(),
        );
        assert_eq!(
            command,
            Ok("ssh vm-work sh -c 'cd \"/srv/app\" && npx acp-adapter'".to_string())
        );
        assert_eq!(
            connect_cwd,
            PaneCwd::Remote("/srv/app".to_string()),
            "connect cwd follows the lane's current session path, not the pane's stale one"
        );
    }

    /// The other half of B′: the lane answering `Local` (explicitly retiring
    /// a prior remote setup) must also override a stale `PaneCwd::Remote`.
    #[test]
    fn restored_remote_pane_follows_the_lane_back_to_local() {
        let lane = lane_at("/local/checkout", Some(LaneSessionHost::Local), None);
        let stale_cwd = PaneCwd::Remote("stale-unrelated-path".into());
        let launch = raw("npx acp-adapter");
        let (command, connect_cwd) = resolve_session_command(
            &launch,
            Some(&lane),
            &stale_cwd,
            None,
            resolved(&launch, Some(&lane)).as_ref(),
        );
        assert_eq!(command, Ok("npx acp-adapter".to_string()));
        assert_eq!(
            connect_cwd,
            PaneCwd::Local(PathBuf::from("/local/checkout"))
        );
    }

    /// Owning lane not found (pane outlived it) — falls back to local,
    /// never promoted to remote off a launch that names no host of its own.
    #[test]
    fn missing_owning_lane_falls_back_to_local_never_promotes_to_remote() {
        let local_cwd = PaneCwd::Local(PathBuf::from("/pane/own/local/path"));
        let (command, connect_cwd) =
            resolve_session_command(&raw("npx acp-adapter"), None, &local_cwd, None, None);
        assert_eq!(command, Ok("npx acp-adapter".to_string()));
        assert_eq!(
            connect_cwd,
            PaneCwd::Local(PathBuf::from("/pane/own/local/path"))
        );
    }

    /// The legacy combo (lane never answered `session_host`, agent-side
    /// `Ssh`/`Docker` + `remote_cwd`) must keep assembling remotely, and the
    /// assembled string must stay byte-identical to what `AgentLaunch::wrap`
    /// alone produced before this axis moved to the lane.
    #[test]
    fn legacy_combo_still_assembles_remotely_byte_identical() {
        let lane = lane_at("/local/checkout", None, Some("/srv/legacy"));
        let launch = AgentLaunch::Ssh {
            adapter_command: "npx acp-adapter".into(),
            host: "old-box".into(),
        };
        let cwd = PaneCwd::Remote("/srv/legacy".into());
        let (command, connect_cwd) = resolve_session_command(
            &launch,
            Some(&lane),
            &cwd,
            None,
            resolved(&launch, Some(&lane)).as_ref(),
        );
        assert_eq!(
            command,
            launch
                .wrap(Some("/srv/legacy"))
                .map_err(|()| ConnectCommandError::NoRemotePath),
            "must match AgentLaunch::wrap's own assembly for the same inputs"
        );
        assert_eq!(connect_cwd, PaneCwd::Remote("/srv/legacy".to_string()));
    }

    /// The legacy `Raw` + `{{cwd}}` token escape hatch predates the lane
    /// session-host axis and must be untouched by it: even when the lane has
    /// its own `session_host` set to something else, the token path keeps
    /// resolving purely from the pane's own persisted `cwd`, exactly as
    /// `AgentLaunch::wrap` already did.
    #[test]
    fn raw_token_escape_hatch_ignores_the_lanes_session_host() {
        let lane = lane_at(
            "/local/checkout",
            Some(LaneSessionHost::Ssh {
                target: "other-host".into(),
                session_path: "/other/path".into(),
                registry_id: None,
            }),
            None,
        );
        let launch = raw("ssh vm-work \"cd {{cwd}} && run\"");
        let cwd = PaneCwd::Remote("/home/user/project".into());
        let (command, connect_cwd) = resolve_session_command(
            &launch,
            Some(&lane),
            &cwd,
            None,
            resolved(&launch, Some(&lane)).as_ref(),
        );
        assert_eq!(
            command,
            Ok("ssh vm-work \"cd /home/user/project && run\"".to_string())
        );
        assert_eq!(
            connect_cwd,
            PaneCwd::Remote("/home/user/project".to_string())
        );
    }

    fn json_stdio_launch() -> AgentLaunch {
        AgentLaunch::Raw(r#"{"command":"/opt/adapters/claude-agent-acp","args":[]}"#.to_string())
    }

    /// A JSON stdio config can't be folded into `ssh`/`docker` — `wrap` only
    /// knows how to splice a shell command line in there, and raw JSON isn't
    /// one. Must fail with the dedicated reason, not silently produce a
    /// broken command.
    #[test]
    fn json_stdio_launch_on_a_remote_lane_is_rejected() {
        let lane = lane_at(
            "/local/checkout",
            Some(LaneSessionHost::Ssh {
                target: "vm-work".into(),
                session_path: "/srv/app".into(),
                registry_id: None,
            }),
            None,
        );
        let cwd = PaneCwd::Local(PathBuf::from("/local/checkout"));
        let launch = json_stdio_launch();
        let (command, resolved_cwd) = resolve_session_command(
            &launch,
            Some(&lane),
            &cwd,
            None,
            resolved(&launch, Some(&lane)).as_ref(),
        );
        assert_eq!(command, Err(ConnectCommandError::JsonStdioRemote));
        // The resolved cwd is still reported (used to sync the pane even on
        // a rejected connect), even though the command itself failed.
        assert_eq!(resolved_cwd, PaneCwd::Remote("/srv/app".to_string()));
    }

    /// The same JSON stdio config on a lane that resolves `Local` is
    /// unaffected — `wrap` passes a `Local` command through unchanged, and
    /// `daruda_acp` parses it exactly as it always has.
    #[test]
    fn json_stdio_launch_on_a_local_lane_is_unaffected() {
        let lane = lane_at("/local/checkout", Some(LaneSessionHost::Local), None);
        let cwd = PaneCwd::Local(PathBuf::from("/local/checkout"));
        let launch = json_stdio_launch();
        let (command, resolved_cwd) = resolve_session_command(
            &launch,
            Some(&lane),
            &cwd,
            None,
            resolved(&launch, Some(&lane)).as_ref(),
        );
        assert_eq!(
            command,
            Ok(r#"{"command":"/opt/adapters/claude-agent-acp","args":[]}"#.to_string())
        );
        assert_eq!(
            resolved_cwd,
            PaneCwd::Local(PathBuf::from("/local/checkout"))
        );
    }

    /// Finding: a deprecated `Ssh`-typed launch that a lane now resolves to
    /// `Local` (the user picked Local in the Session Host dialog, retiring
    /// the launch's own embedded host) must still be eligible for a managed
    /// account, exactly like a `Raw` launch running the same command would.
    #[test]
    fn account_recipe_for_connect_recognizes_a_locally_resolved_ssh_launch() {
        let launch = AgentLaunch::Ssh {
            adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest".into(),
            host: "old-box".into(),
        };
        assert_eq!(
            account_recipe_for_connect(&launch, false),
            Some(daruda_store::accounts::AccountRecipeId::Claude)
        );
        // Still excluded once it's actually remote.
        assert_eq!(account_recipe_for_connect(&launch, true), None);
    }

    /// Same for `Docker`.
    #[test]
    fn account_recipe_for_connect_recognizes_a_locally_resolved_docker_launch() {
        let launch = AgentLaunch::Docker {
            adapter_command: "npx -y @agentclientprotocol/codex-acp@latest".into(),
            container: "dev-1".into(),
        };
        assert_eq!(
            account_recipe_for_connect(&launch, false),
            Some(daruda_store::accounts::AccountRecipeId::Codex)
        );
    }

    /// A locally-resolved `Ssh`/`Docker` launch whose command is a JSON
    /// stdio config is still excluded — the shape that bars a managed
    /// account for `Raw` bars it here too.
    #[test]
    fn account_recipe_for_connect_still_excludes_json_stdio() {
        let launch = AgentLaunch::Ssh {
            adapter_command: r#"{"command":"x"}"#.into(),
            host: "old-box".into(),
        };
        assert_eq!(account_recipe_for_connect(&launch, false), None);
    }

    /// A plain `Raw` launch is unaffected by the new match arm — behaves
    /// exactly as `account_recipe` itself already does.
    #[test]
    fn account_recipe_for_connect_matches_account_recipe_for_raw_launches() {
        let launch = raw("npx -y @agentclientprotocol/claude-agent-acp@latest");
        assert_eq!(
            account_recipe_for_connect(&launch, false),
            launch.account_recipe(false)
        );
        assert_eq!(
            account_recipe_for_connect(&launch, true),
            launch.account_recipe(true)
        );
    }

    #[test]
    fn env_is_folded_in_for_the_lane_resolved_path() {
        let lane = lane_at(
            "/local/checkout",
            Some(LaneSessionHost::Ssh {
                target: "vm-work".into(),
                session_path: "/srv/app".into(),
                registry_id: None,
            }),
            None,
        );
        let env = AccountEnv {
            inject: vec![("CLAUDE_CONFIG_DIR".into(), "/remote/acc".into())],
            strip: vec!["ANTHROPIC_API_KEY"],
        };
        let cwd = PaneCwd::Local(PathBuf::from("/unused"));
        let launch = raw("npx acp-adapter");
        let (command, _) = resolve_session_command(
            &launch,
            Some(&lane),
            &cwd,
            Some(&env),
            resolved(&launch, Some(&lane)).as_ref(),
        );
        let command = command.unwrap();
        assert!(command.contains("export CLAUDE_CONFIG_DIR=\"/remote/acc\""));
        assert!(command.contains("unset ANTHROPIC_API_KEY"));
    }

    #[test]
    fn account_strip_env_carries_the_recipes_auth_overrides() {
        use crate::agent::account::PreparedAccount;
        use daruda_store::accounts::AccountRecipeId;
        use std::path::PathBuf;

        let recipe = daruda_agent::accounts::recipe_for(AccountRecipeId::Claude);
        let config_dir = PathBuf::from("/data/acc/alice");
        let prepared = PreparedAccount {
            recipe: AccountRecipeId::Claude,
            env: daruda_config::account_env(
                recipe.config_dir_env(),
                &config_dir,
                recipe.strip_env(),
            ),
            config_dir,
        };
        let strip = account_strip_env(Some(&prepared));
        assert_eq!(strip, recipe.strip_env());
        assert!(strip.iter().any(|n| n == "ANTHROPIC_API_KEY"));

        // Codex strips nothing, and the System account (no managed account)
        // runs under the user's own environment by definition.
        let codex = daruda_agent::accounts::recipe_for(AccountRecipeId::Codex);
        assert!(codex.strip_env().is_empty());
        assert!(account_strip_env(None).is_empty());
    }

    // Task 3: connect-path write-back — a registry `target`/`container` edit,
    // or a tombstone redirect, syncs back onto the lane's cached
    // `session_host` so a future connect (and any UI reading it) sees the
    // fresh value instead of re-deriving it every time.

    fn write_back_tombstone(
        old_id: daruda_store::project::SessionHostId,
        redirected_to: Option<daruda_store::project::SessionHostId>,
    ) -> daruda_config::SessionHostTombstone {
        daruda_config::SessionHostTombstone {
            old_id,
            kind: daruda_config::SessionHostKind::Ssh {
                target: "old-box".into(),
            },
            value: "old-box".into(),
            removed_at: 0,
            redirected_to,
        }
    }

    /// The Step 1 scenario: a catalog `target` edit surfaces through
    /// `effective_session_host` already (Task 2) — this asserts the
    /// write-back actually persists it onto the lane's cache.
    #[test]
    fn catalog_target_change_writes_back_the_lanes_cached_target() {
        let id = daruda_store::project::SessionHostId::new();
        let cached = LaneSessionHost::Ssh {
            target: "old-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(id),
        };
        let catalog = vec![daruda_config::SessionHostEntry {
            id,
            label: "Build box".into(),
            kind: daruda_config::SessionHostKind::Ssh {
                target: "new-target".into(),
            },
        }];
        // Task 2's resolver already folded the catalog's current target
        // in — this is what `connect_agent_chat` would pass as `resolved`.
        let resolved = LaneSessionHost::Ssh {
            target: "new-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(id),
        };
        let written = session_host_write_back(Some(&cached), Some(&resolved), &catalog, &[])
            .expect("target changed — must write back");
        assert_eq!(
            written,
            LaneSessionHost::Ssh {
                target: "new-target".into(),
                session_path: "/srv/app".into(),
                registry_id: Some(id),
            }
        );
    }

    /// A tombstone redirect also corrects the cached `registry_id` to the
    /// live id — not just `target` — so the next connect resolves directly
    /// against the catalog instead of re-chasing the tombstone.
    #[test]
    fn tombstone_redirect_writes_back_both_target_and_registry_id() {
        let old_id = daruda_store::project::SessionHostId::new();
        let new_id = daruda_store::project::SessionHostId::new();
        let cached = LaneSessionHost::Ssh {
            target: "old-box".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(old_id),
        };
        let catalog = vec![daruda_config::SessionHostEntry {
            id: new_id,
            label: "Renamed box".into(),
            kind: daruda_config::SessionHostKind::Ssh {
                target: "merged-target".into(),
            },
        }];
        let tombstones = vec![write_back_tombstone(old_id, Some(new_id))];
        // `effective_session_host` (Task 2) resolves the target through the
        // redirect but leaves `registry_id` at the stale cached id — exactly
        // what `connect_agent_chat` would pass here.
        let resolved = LaneSessionHost::Ssh {
            target: "merged-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(old_id),
        };
        let written =
            session_host_write_back(Some(&cached), Some(&resolved), &catalog, &tombstones)
                .expect("redirect resolved a new value — must write back");
        assert_eq!(
            written,
            LaneSessionHost::Ssh {
                target: "merged-target".into(),
                session_path: "/srv/app".into(),
                registry_id: Some(new_id),
            }
        );
    }

    /// No drift → no write, so a connect that changes nothing doesn't
    /// spuriously dirty + persist the workspace on every single connect.
    #[test]
    fn unchanged_value_writes_back_nothing() {
        let id = daruda_store::project::SessionHostId::new();
        let host = LaneSessionHost::Ssh {
            target: "vm-work".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(id),
        };
        let catalog = vec![daruda_config::SessionHostEntry {
            id,
            label: "Build box".into(),
            kind: daruda_config::SessionHostKind::Ssh {
                target: "vm-work".into(),
            },
        }];
        assert_eq!(
            session_host_write_back(Some(&host), Some(&host), &catalog, &[]),
            None
        );
    }

    /// An unanswered lane (`session_host: None`) must never be silently
    /// promoted to an explicit answer by this sync — that's a distinct,
    /// user-driven action (`Lane::set_session_host`'s doc), not something a
    /// background resolve should do on its own.
    #[test]
    fn unanswered_lane_is_never_written_to() {
        let resolved = LaneSessionHost::Ssh {
            target: "vm-work".into(),
            session_path: "/srv/app".into(),
            registry_id: None,
        };
        assert_eq!(
            session_host_write_back(None, Some(&resolved), &[], &[]),
            None
        );
    }

    /// No owning lane at all (pane with no resolvable lane) — nothing to
    /// write back to.
    #[test]
    fn missing_owning_lane_writes_back_nothing() {
        let cached = LaneSessionHost::Local;
        assert_eq!(session_host_write_back(Some(&cached), None, &[], &[]), None);
    }

    /// `sync_registry_id` itself: a `Local` host has no registry link to
    /// correct — passed through unchanged.
    #[test]
    fn sync_registry_id_is_a_noop_for_local() {
        assert_eq!(
            sync_registry_id(LaneSessionHost::Local, &[], &[]),
            LaneSessionHost::Local
        );
    }

    /// An orphaned link (deleted, no redirect) keeps its stale id — there is
    /// nothing live to correct it to.
    #[test]
    fn sync_registry_id_keeps_an_orphaned_id() {
        let id = daruda_store::project::SessionHostId::new();
        let host = LaneSessionHost::Docker {
            container: "dev-1".into(),
            session_path: "/workspace".into(),
            registry_id: Some(id),
        };
        assert_eq!(sync_registry_id(host.clone(), &[], &[]), host);
    }

    /// The single entry point answers all four questions at once: what command
    /// to run, where it is rooted, what path the wire protocol gets, and
    /// whether the lane owes a session-host correction.
    #[test]
    fn resolve_launch_bundles_command_cwd_and_strip_env() {
        use daruda_store::accounts::AccountRecipeId;

        let launch = raw("npx acp-adapter");
        let local = PaneCwd::Local(PathBuf::from("/repo"));

        // Same construction the moved `account_strip_env` test uses, so the
        // strip list below is the recipe's real one rather than a stub.
        let recipe = daruda_agent::accounts::recipe_for(AccountRecipeId::Claude);
        let config_dir = PathBuf::from("/data/acc/alice");
        let prepared = PreparedAccount {
            recipe: AccountRecipeId::Claude,
            env: daruda_config::account_env(
                recipe.config_dir_env(),
                &config_dir,
                recipe.strip_env(),
            ),
            config_dir,
        };

        let resolved = resolve_launch(&launch, None, &local, Some(&prepared), None, None, &[], &[]);

        let spec = resolved.spec.expect("a local raw launch always resolves");
        // `wrap_with_env` prefixes a `Local` command with an inject-only
        // `KEY='value' ` segment (see `wrap_with_env_local_matches_the_shipped_assembly`);
        // the bare adapter command still trails it verbatim.
        assert_eq!(
            spec.command,
            "CLAUDE_CONFIG_DIR='/data/acc/alice' npx acp-adapter"
        );
        assert!(spec.strip_env.iter().any(|n| n == "ANTHROPIC_API_KEY"));
        assert_eq!(resolved.wire_cwd, PathBuf::from("/repo"));
        assert_eq!(resolved.host_write_back, None);
    }

    /// `cached_host` and `resolved_host` are adjacent same-typed parameters
    /// consumed by two different callees inside `resolve_launch`
    /// (`resolve_session_command` takes only `resolved_host`;
    /// `session_host_write_back` takes both, in `(cached, resolved)` order).
    /// A `None, None` fixture can't tell a correct wiring from a swapped one
    /// — both are vacuous. Here `cached` and `resolved` differ, so a swap
    /// would flip every assertion below: the command would wrap for `Local`
    /// (the cached host) instead of the freshly-resolved `Ssh`, and the
    /// write-back would report no drift instead of the real one.
    #[test]
    fn resolve_launch_uses_resolved_host_for_command_and_cached_for_write_back_baseline() {
        let cached = LaneSessionHost::Local;
        let resolved_host = LaneSessionHost::Ssh {
            target: "vm".into(),
            session_path: "/srv/app".into(),
            registry_id: None,
        };
        let launch = raw("npx acp-adapter");
        let local = PaneCwd::Local(PathBuf::from("/repo"));

        let r = resolve_launch(
            &launch,
            None,
            &local,
            None,
            Some(&cached),
            Some(&resolved_host),
            &[],
            &[],
        );

        // Command assembly follows `resolved_host` (the freshly-resolved
        // `Ssh`), not `cached` (`Local`) — byte-identical to
        // `session_host::wrap` for the same `(host, adapter_command)` pair.
        let spec = r.spec.expect("an ssh-resolved host still assembles");
        assert_eq!(
            spec.command,
            "ssh vm sh -c 'cd \"/srv/app\" && npx acp-adapter'"
        );
        assert_eq!(r.resolved_cwd, PaneCwd::Remote("/srv/app".to_string()));
        assert_eq!(r.wire_cwd, PathBuf::from("/srv/app"));
        // `cached` (`Local`) differs from the corrected `resolved_host`, so
        // the write-back fires with the resolved value.
        assert_eq!(r.host_write_back, Some(resolved_host));
    }
}
