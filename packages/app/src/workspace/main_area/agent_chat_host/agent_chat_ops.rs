//! `impl Workspace` ops for the Agent chat pane, split by concern into the
//! child modules below (pane construction, notifications, mode/config, links,
//! accessors); this file keeps their pure helpers. They need `Workspace`
//! state (default permission mode, per-event `syntax_theme`, `report_error`);
//! per-event folding and render-listener ops live on [`AgentChatView`].
//!
//! The ACP connection + event pump live in the sibling
//! [`super::agent_chat_connect_ops`]; the prompt-queue send/edit/cancel
//! routing lives in [`super::agent_chat_queue_ops`]. Telegram relay lives in
//! [`super::telegram_ops`]; `agent_chat_ops/notify.rs` only tees into it from
//! `maybe_notify_agent_event` and `fire_activity_completion`.
//!
//! [`AgentChatView`]: daruda_chat::view::AgentChatView

mod links;
mod mode_config;
mod notify;
mod pane_access;
mod pane_build;
#[cfg(feature = "screenshot")]
mod shots;

#[cfg(feature = "screenshot")]
pub(crate) use shots::ThoughtsShot;

use daruda_config::AgentLaunch;
use daruda_store::project::{LaneSessionHost, PaneCwd};
use std::path::PathBuf;

use daruda_chat::view::AgentIdentity;

/// The catalog's default agent id — the first entry, or the built-in Claude id
/// if the catalog is somehow empty (the config layer guarantees non-empty, so
/// the fallback is purely defensive).
pub(super) fn catalog_default_id(agents: &[daruda_config::AgentDefinition]) -> String {
    agents
        .first()
        .map(|a| a.id.clone())
        .unwrap_or_else(|| daruda_config::AgentDefinition::claude_default().id)
}

pub(super) fn agent_name_for(agents: &[daruda_config::AgentDefinition], agent_id: &str) -> String {
    agents
        .iter()
        .find(|a| a.id == agent_id)
        .map(|a| a.name.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| agent_id.to_string())
}

/// Pure core of [`Workspace::resolve_restored_agent`] — decide the effective
/// agent for a restored pane and whether its persisted session id survives.
/// Factored out of the workspace so it is unit-testable without gpui.
///
/// [`Workspace::resolve_restored_agent`]: crate::workspace::Workspace::resolve_restored_agent
fn resolve_restored_agent(
    agents: &[daruda_config::AgentDefinition],
    persisted_agent_id: Option<String>,
) -> (String, bool) {
    let owner =
        persisted_agent_id.unwrap_or_else(|| daruda_config::AgentDefinition::claude_default().id);
    if agents.iter().any(|a| a.id == owner) {
        (owner, true)
    } else {
        (catalog_default_id(agents), false)
    }
}

/// Pure core of the agent-id choice for a freshly opened pane: keep `last`
/// when still in the catalog, else fall back to `agents[0]`. Factored out for
/// unit-testability without gpui.
fn resolve_open_agent_id(agents: &[daruda_config::AgentDefinition], last: Option<&str>) -> String {
    last.filter(|id| agents.iter().any(|a| a.id == *id))
        .map(str::to_owned)
        .unwrap_or_else(|| catalog_default_id(agents))
}

/// Cwd outcome fed into [`Workspace::build_agent_chat_pane`]'s status
/// derivation. One enum (not a `cwd` + separate error-reason pair that could
/// disagree) so an unattachable pane carries its own reason as its own variant.
///
/// [`Workspace::build_agent_chat_pane`]: crate::workspace::Workspace::build_agent_chat_pane
enum PaneCwdOutcome {
    /// Normal resolution: `Some` parks the pane `Idle` with that cwd; `None`
    /// parks it in the generic "no working directory" error (cwd-less restore).
    Ready(Option<PaneCwd>),
    /// Cwd unresolvable — parks the pane in `Error` with the given
    /// (already-localized) reason, cwd `None`.
    Blocked(String),
}

/// Why a fresh pane has nowhere to attach. Two variants because they are
/// fixed in different places, and the blocked pane has to say which.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PaneCwdBlocked {
    /// A legacy `Raw` + `{{cwd}}` launch with no remote path to substitute.
    NoRemotePath,
    /// The lane's session host holds a value the launch quoting cannot carry.
    UnusableSessionHost(crate::lane::session_host::SessionHostError),
}

/// Pure core of [`Workspace::resolve_new_pane_cwd`]: decide `Local` vs.
/// `Remote` cwd for a fresh pane, mirroring what its first connect will
/// resolve (`resolve_session_command` in `crate::agent::launch_resolve`) so a
/// freshly opened pane's `PaneCwd` — the value the rest of the app reads via
/// `PaneCwd::as_local`/`into_local` — already agrees with where the session
/// actually attaches.
///
/// Two shapes have nowhere to attach and block the pane rather than guess:
/// the legacy `Raw` + `{{cwd}}` escape hatch with no non-blank `remote_cwd`
/// to substitute, and a lane whose resolved host the launch quoting cannot
/// carry (see `session_host::effective_session_host`). Every other launch
/// resolves through `session_host` to a remote path or the lane's own local
/// one.
///
/// [`Workspace::resolve_new_pane_cwd`]: crate::workspace::Workspace::resolve_new_pane_cwd
fn resolve_new_pane_cwd_core(
    launch: &AgentLaunch,
    local_cwd: Option<PathBuf>,
    remote_cwd: Option<String>,
    session_host: Option<&LaneSessionHost>,
    session_host_catalog: &[daruda_config::SessionHostEntry],
    session_host_tombstones: &[daruda_config::SessionHostTombstone],
) -> Result<Option<PaneCwd>, PaneCwdBlocked> {
    if matches!(launch, AgentLaunch::Raw(command) if command.contains(daruda_config::agent::CWD_TOKEN))
    {
        // A blank remote_cwd has nothing to substitute for the remote path
        // — treat it the same as `None` rather than letting it flow into
        // `AgentLaunch::wrap` and produce a broken `cd  && ...` command.
        return remote_cwd
            .filter(|cwd| !cwd.trim().is_empty())
            .map(PaneCwd::Remote)
            .map(Some)
            .ok_or(PaneCwdBlocked::NoRemotePath);
    }
    let Some(local_cwd) = local_cwd else {
        return Ok(None); // no active lane at all
    };
    let host = crate::lane::session_host::effective_session_host(
        session_host,
        remote_cwd.as_deref(),
        launch,
        session_host_catalog,
        session_host_tombstones,
    )
    .map_err(|unusable| PaneCwdBlocked::UnusableSessionHost(unusable.reason))?;
    Ok(Some(match host.session_path() {
        Some(remote_path) => PaneCwd::Remote(remote_path.to_string()),
        None => PaneCwd::Local(local_cwd),
    }))
}

/// Whether to raise the notification: the channel must be enabled, and it is
/// suppressed only when the app is foreground AND the firing pane is on
/// screen (the user is already looking at it). Mirrors the hook-notification
/// gate.
fn should_notify_agent_event(
    enabled: bool,
    skip_pane_in_view: bool,
    app_active: bool,
    pane_on_screen: bool,
) -> bool {
    enabled && !(skip_pane_in_view && app_active && pane_on_screen)
}

/// A catalog agent as a chat pane is given it: the mark is looked up here,
/// where the agent catalog's icons live.
pub(super) fn agent_identity(id: String, name: String) -> AgentIdentity {
    AgentIdentity {
        icon: daruda_config::agent::icon::icon_for_agent(&id),
        id,
        name,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PaneCwdBlocked, resolve_new_pane_cwd_core, resolve_open_agent_id, resolve_restored_agent,
        should_notify_agent_event,
    };
    use daruda_config::{AgentDefinition, AgentLaunch};
    use daruda_store::project::{LaneSessionHost, PaneCwd};
    use std::path::PathBuf;

    fn ssh_launch() -> AgentLaunch {
        AgentLaunch::Ssh {
            adapter_command: "npx acp-adapter".into(),
            host: "vm-work".into(),
        }
    }

    fn docker_launch() -> AgentLaunch {
        AgentLaunch::Docker {
            adapter_command: "npx acp-adapter".into(),
            container: "devbox".into(),
        }
    }

    fn local_launch() -> AgentLaunch {
        AgentLaunch::Raw("npx acp-adapter".into())
    }

    fn token_launch() -> AgentLaunch {
        AgentLaunch::Raw("ssh vm-work \"cd {{cwd}} && run\"".into())
    }

    #[test]
    fn resolve_new_pane_cwd_legacy_remote_combo_is_remote() {
        // The legacy pair (agent-side `Ssh`/`Docker` + the lane's own
        // `remote_cwd`, no `session_host` answered yet) still resolves
        // remotely — `effective_session_host`'s legacy fallback branch.
        for launch in [ssh_launch(), docker_launch()] {
            let result = resolve_new_pane_cwd_core(
                &launch,
                Some(PathBuf::from("/local/lane")),
                Some("/remote/lane".to_string()),
                None,
                &[],
                &[],
            );
            assert_eq!(
                result,
                Ok(Some(PaneCwd::Remote("/remote/lane".to_string()))),
                "launch {launch:?} should resolve to Remote"
            );
        }
    }

    #[test]
    fn resolve_new_pane_cwd_remote_launch_without_remote_cwd_falls_back_to_local() {
        // No `Err` case for `Ssh`/`Docker` anymore — with nothing on either
        // axis to name a host, the lane falls back to `Local` rather than
        // blocking the pane (mirrors `a_half_configured_legacy_pair_stays_local`
        // in `lane::session_host`).
        for launch in [ssh_launch(), docker_launch()] {
            let result = resolve_new_pane_cwd_core(
                &launch,
                Some(PathBuf::from("/local/lane")),
                None,
                None,
                &[],
                &[],
            );
            assert_eq!(
                result,
                Ok(Some(PaneCwd::Local(PathBuf::from("/local/lane")))),
                "launch {launch:?} should fall back to Local"
            );
        }
    }

    #[test]
    fn resolve_new_pane_cwd_remote_launch_with_blank_remote_cwd_falls_back_to_local() {
        // An empty or whitespace-only legacy `remote_cwd` has nothing to
        // substitute in — same fallback as no `remote_cwd` at all, not a
        // bogus `Remote("")`/`Remote("   ")`.
        for launch in [ssh_launch(), docker_launch()] {
            for blank in ["", "   ", "\t"] {
                let result = resolve_new_pane_cwd_core(
                    &launch,
                    Some(PathBuf::from("/local/lane")),
                    Some(blank.to_string()),
                    None,
                    &[],
                    &[],
                );
                assert_eq!(
                    result,
                    Ok(Some(PaneCwd::Local(PathBuf::from("/local/lane")))),
                    "launch {launch:?}, blank remote_cwd {blank:?} should fall back to Local"
                );
            }
        }
    }

    #[test]
    fn resolve_new_pane_cwd_session_host_overrides_a_local_agent_to_remote() {
        // The point of the lane axis: a plain local `Raw` agent still
        // attaches remotely when the lane itself answered `session_host`.
        let host = LaneSessionHost::Ssh {
            target: "vm-work".into(),
            session_path: "/srv/app".into(),
            registry_id: None,
        };
        let result = resolve_new_pane_cwd_core(
            &local_launch(),
            Some(PathBuf::from("/local/lane")),
            None,
            Some(&host),
            &[],
            &[],
        );
        assert_eq!(result, Ok(Some(PaneCwd::Remote("/srv/app".to_string()))));
    }

    #[test]
    fn resolve_new_pane_cwd_session_host_local_wins_over_legacy_remote_cwd() {
        // `Some(Local)` retires the legacy pair even for a fresh pane —
        // matches `answering_local_retires_the_legacy_pair`.
        let result = resolve_new_pane_cwd_core(
            &ssh_launch(),
            Some(PathBuf::from("/local/lane")),
            Some("/legacy/path".to_string()),
            Some(&LaneSessionHost::Local),
            &[],
            &[],
        );
        assert_eq!(
            result,
            Ok(Some(PaneCwd::Local(PathBuf::from("/local/lane"))))
        );
    }

    #[test]
    fn resolve_new_pane_cwd_local_launch_uses_local() {
        let result = resolve_new_pane_cwd_core(
            &local_launch(),
            Some(PathBuf::from("/local/lane")),
            Some("/remote/lane".to_string()),
            None,
            &[],
            &[],
        );
        assert_eq!(
            result,
            Ok(Some(PaneCwd::Local(PathBuf::from("/local/lane"))))
        );
    }

    #[test]
    fn resolve_new_pane_cwd_local_launch_no_local_is_none() {
        let result = resolve_new_pane_cwd_core(&local_launch(), None, None, None, &[], &[]);
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn resolve_new_pane_cwd_token_escape_hatch_still_errs_without_remote_cwd() {
        // The one shape that keeps blocking: the legacy `{{cwd}}` token is
        // untouched by the lane session-host axis (see
        // `session_host::adapter_command`'s doc), so it still needs a
        // non-blank `remote_cwd` up front, `session_host` notwithstanding.
        let host = LaneSessionHost::Ssh {
            target: "other-host".into(),
            session_path: "/other/path".into(),
            registry_id: None,
        };
        let result = resolve_new_pane_cwd_core(
            &token_launch(),
            Some(PathBuf::from("/local/lane")),
            None,
            Some(&host),
            &[],
            &[],
        );
        assert_eq!(result, Err(PaneCwdBlocked::NoRemotePath));
    }

    /// A lane whose persisted host the launch quoting cannot carry blocks the
    /// pane with that reason — not the `{{cwd}}` one, and never `Local`, which
    /// would open the pane on this machine instead of where the lane says.
    #[test]
    fn resolve_new_pane_cwd_unusable_host_blocks_with_its_own_reason() {
        let host = LaneSessionHost::Ssh {
            target: "vm -o ProxyCommand=touch /tmp/pwned".into(),
            session_path: "/srv/app".into(),
            registry_id: None,
        };
        let result = resolve_new_pane_cwd_core(
            &local_launch(),
            Some(PathBuf::from("/local/lane")),
            None,
            Some(&host),
            &[],
            &[],
        );
        assert_eq!(
            result,
            Err(PaneCwdBlocked::UnusableSessionHost(
                crate::lane::session_host::SessionHostError::Unsafe(
                    crate::lane::session_host::SessionHostField::Target
                )
            ))
        );
    }

    #[test]
    fn resolve_new_pane_cwd_token_escape_hatch_uses_its_own_remote_cwd() {
        let result = resolve_new_pane_cwd_core(
            &token_launch(),
            Some(PathBuf::from("/local/lane")),
            Some("/remote/lane".to_string()),
            None,
            &[],
            &[],
        );
        assert_eq!(
            result,
            Ok(Some(PaneCwd::Remote("/remote/lane".to_string())))
        );
    }

    #[test]
    fn should_notify_disabled_channel_never_fires() {
        assert!(!should_notify_agent_event(false, true, true, true));
        assert!(!should_notify_agent_event(false, false, false, false));
    }

    #[test]
    fn should_notify_foreground_focused_pane_is_suppressed() {
        assert!(!should_notify_agent_event(true, true, true, true));
    }

    #[test]
    fn should_notify_backgrounded_pane_fires() {
        // App is active but the firing pane is not the focused one.
        assert!(should_notify_agent_event(true, true, true, false));
    }

    #[test]
    fn should_notify_app_inactive_fires_even_for_focused_pane() {
        assert!(should_notify_agent_event(true, true, false, true));
    }

    #[test]
    fn should_notify_skip_focused_disabled_always_fires() {
        assert!(should_notify_agent_event(true, false, true, true));
    }

    fn default_catalog() -> Vec<AgentDefinition> {
        vec![AgentDefinition::claude_default()]
    }

    #[test]
    fn restored_agent_kept_when_owner_present() {
        let claude = AgentDefinition::claude_default().id;
        let (agent, keep) = resolve_restored_agent(&default_catalog(), Some(claude.clone()));
        assert_eq!(agent, claude);
        assert!(
            keep,
            "session id resumable when owning agent is in the catalog"
        );
    }

    #[test]
    fn restored_none_falls_back_to_claude_and_keeps_session() {
        // Backward compat: a pre-feature save (agent_id None) was created by the
        // built-in Claude agent, which is always in the no-`[[agents]]` default,
        // so its session must still resume.
        let (agent, keep) = resolve_restored_agent(&default_catalog(), None);
        assert_eq!(agent, AgentDefinition::claude_default().id);
        assert!(keep, "pre-feature Claude session stays resumable");
    }

    #[test]
    fn restored_removed_agent_falls_back_and_drops_session() {
        // The persisted owner is no longer in the catalog — fall back to the
        // default agent and drop the session id (resuming it would be invalid).
        let (agent, keep) = resolve_restored_agent(&default_catalog(), Some("ghost".to_string()));
        assert_eq!(agent, AgentDefinition::claude_default().id);
        assert!(!keep, "session id dropped when owning agent was removed");
    }

    #[test]
    fn restored_selects_owner_among_multiple_agents() {
        let agents = vec![
            AgentDefinition::new(
                "other".to_string(),
                "Other".to_string(),
                AgentLaunch::Raw("run-other".to_string()),
            ),
            AgentDefinition::claude_default(),
        ];
        // The default (catalog[0]) is "other", but a persisted claude owner that
        // is still present is kept, not overridden by the default.
        let (agent, keep) =
            resolve_restored_agent(&agents, Some(AgentDefinition::claude_default().id));
        assert_eq!(agent, AgentDefinition::claude_default().id);
        assert!(keep);
        // A removed owner falls back to catalog[0] = "other".
        let (agent, keep) = resolve_restored_agent(&agents, Some("gone".to_string()));
        assert_eq!(agent, "other");
        assert!(!keep);
    }

    fn two_agent_catalog() -> Vec<AgentDefinition> {
        vec![
            AgentDefinition::new(
                "other".to_string(),
                "Other".to_string(),
                AgentLaunch::Raw("run-other".to_string()),
            ),
            AgentDefinition::claude_default(),
        ]
    }

    #[test]
    fn open_agent_keeps_valid_last_id() {
        // A last id still in the catalog is kept, not overridden by catalog[0].
        let claude = AgentDefinition::claude_default().id;
        let agents = two_agent_catalog();
        assert_eq!(resolve_open_agent_id(&agents, Some(&claude)), claude);
    }

    #[test]
    fn open_agent_stale_last_id_falls_back_to_catalog_head() {
        // A last id no longer in the catalog falls back to catalog[0] = "other".
        let agents = two_agent_catalog();
        assert_eq!(resolve_open_agent_id(&agents, Some("gone")), "other");
    }

    #[test]
    fn open_agent_none_falls_back_to_catalog_head() {
        // No prior choice falls back to catalog[0] = "other".
        let agents = two_agent_catalog();
        assert_eq!(resolve_open_agent_id(&agents, None), "other");
    }
}
