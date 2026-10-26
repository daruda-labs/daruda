//! Capture the configuration identity alongside the actual session launch.
use super::{Configuration, LibraryView, Provider, Workspace, provider};
use crate::{agent::account::PreparedAccount, workspace::main_area::pane_tree::PaneId};
use daruda_store::project::PaneCwd;
use std::path::PathBuf;

impl Workspace {
    pub(in crate::workspace) fn capture_library_launch(
        &mut self,
        pane_id: PaneId,
        launch: &daruda_config::AgentLaunch,
        env: &[(String, String)],
        account: Option<&PreparedAccount>,
        cwd: &PaneCwd,
    ) {
        let recipe = match launch {
            daruda_config::AgentLaunch::Ssh {
                adapter_command, ..
            }
            | daruda_config::AgentLaunch::Docker {
                adapter_command, ..
            } => daruda_config::account_recipe_for_local_command(adapter_command),
            launch => launch.account_recipe(false),
        };
        let mut view = LibraryView {
            provider: recipe.map(provider),
            remote: matches!(cwd, PaneCwd::Remote(_)),
            ..Default::default()
        };
        if !view.remote
            && let Some(selected) = view.provider
        {
            let key = match selected {
                Provider::Claude => "CLAUDE_CONFIG_DIR",
                Provider::Codex => "CODEX_HOME",
            };
            let root = account
                .map(|a| a.config_dir.clone())
                .or_else(|| {
                    daruda_core::process::env_value(
                        env.iter().map(|(key, value)| (key, value)),
                        std::ffi::OsStr::new(key),
                    )
                    .map(PathBuf::from)
                })
                .or_else(|| std::env::var_os(key).map(PathBuf::from));
            view.config = Configuration::resolve(
                selected,
                effective_home(env, account.map(|a| &a.env), dirs::home_dir()),
                root,
                cwd.as_local().map(ToOwned::to_owned),
            );
            if view.config.is_none() {
                view.error = Some("Unresolved configuration root".into());
            }
            if account.is_some() {
                view.account_label =
                    self.agent_chat_account_selection(pane_id)
                        .account_id()
                        .map(|id| {
                            self.accounts
                                .find(id)
                                .and_then(|a| a.email.clone())
                                .unwrap_or_else(|| id.0.to_string())
                        });
            }
        }
        self.right_views.library.launched.insert(pane_id, view);
    }
}

pub(super) fn effective_home(
    env: &[(String, String)],
    account: Option<&daruda_config::AccountEnv>,
    ambient: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(account) = account {
        if let Some(value) = daruda_core::process::env_value(
            account.inject.iter().map(|(key, value)| (key, value)),
            std::ffi::OsStr::new("HOME"),
        ) {
            return Some(value.into());
        }
        if account.strip.iter().any(|key| {
            daruda_core::process::env_name_eq(key.as_ref(), std::ffi::OsStr::new("HOME"))
        }) {
            return None;
        }
    }
    daruda_core::process::env_value(
        env.iter().map(|(key, value)| (key, value)),
        std::ffi::OsStr::new("HOME"),
    )
    .map(PathBuf::from)
    .or(ambient)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_home_override_and_removal_precede_agent_and_ambient_values() {
        let env = vec![("HOME".into(), "agent-home".into())];
        let ambient = Some(PathBuf::from("ambient-home"));
        let account = daruda_config::AccountEnv {
            inject: vec![("HOME".into(), "account-home".into())],
            strip: vec!["HOME"],
        };
        assert_eq!(
            effective_home(&env, Some(&account), ambient.clone()),
            Some(PathBuf::from("account-home"))
        );
        let removed = daruda_config::AccountEnv {
            inject: vec![],
            strip: vec!["HOME"],
        };
        assert_eq!(effective_home(&env, Some(&removed), ambient.clone()), None);
        assert_eq!(
            effective_home(&[("HOME".into(), String::new())], None, ambient),
            Some(PathBuf::new())
        );
    }

    #[cfg(windows)]
    #[test]
    fn local_home_lookup_and_removal_accept_windows_environment_spelling() {
        let env = vec![("Home".into(), "agent-home".into())];
        assert_eq!(effective_home(&env, None, None), Some("agent-home".into()));
        let account = daruda_config::AccountEnv {
            inject: vec![],
            strip: vec!["hOmE"],
        };
        assert_eq!(effective_home(&env, Some(&account), None), None);
    }

    #[test]
    fn home_follows_the_launch_environment_even_for_managed_accounts() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("custom");
        let env = vec![("HOME".into(), home.to_string_lossy().into_owned())];
        let ambient = Some(temp.path().join("ambient"));
        for provider in [Provider::Claude, Provider::Codex] {
            let config = Configuration::resolve(
                provider,
                effective_home(&env, None, ambient.clone()),
                None,
                None,
            )
            .unwrap();
            assert_eq!(config.home, home);
        }
        let account = daruda_config::AccountEnv {
            inject: vec![],
            strip: vec![],
        };
        let config = Configuration::resolve(
            Provider::Codex,
            effective_home(&env, Some(&account), ambient.clone()),
            Some(temp.path().join("account")),
            None,
        )
        .unwrap();
        assert_eq!(config.personal_skills, home.join(".agents/skills"));
        assert_eq!(effective_home(&[], None, ambient.clone()), ambient);
        assert!(
            Configuration::resolve(
                Provider::Claude,
                effective_home(&[("HOME".into(), "relative".into())], None, ambient),
                None,
                None
            )
            .is_none()
        );
    }
}
