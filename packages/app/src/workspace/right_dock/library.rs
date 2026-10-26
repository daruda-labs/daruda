//! Pane configuration projection and per-window watcher ownership.

mod launch;
mod palette;

use crate::agent::configuration::{
    Configuration, Provider,
    library::{Inventory, discover},
};
use crate::workspace::{
    Workspace,
    main_area::pane::{AccountDomain, FocusedAccount},
};
use daruda_store::{accounts::AccountRecipeId, project::PaneCwd};
use gpui::{Context, Task};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Weak},
    time::Duration,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::workspace) struct LibraryView {
    pub config: Option<Configuration>,
    pub remote: bool,
    pub provider: Option<Provider>,
    pub inventory: Inventory,
    pub error: Option<String>,
    pub loading: bool,
    pub local_management: bool,
    pub account_label: Option<String>,
}

#[derive(Default)]
pub(in crate::workspace) struct Library {
    launched: HashMap<crate::workspace::main_area::pane_tree::PaneId, LibraryView>,
    pub manual: Option<Provider>,
    pub manual_pane: Option<crate::workspace::main_area::pane_tree::PaneId>,
    pub view: LibraryView,
    pub pins: Vec<Weak<Configuration>>,
    pub revision: u64,
    pub generation: u64,
    selected: Option<Configuration>,
    pub selected_target: Option<crate::agent::configuration::library::Target>,
}

impl Library {
    pub(in crate::workspace) fn observe_context(&mut self, view: &LibraryView) {
        if self.selected != view.config {
            self.selected = view.config.clone();
            self.generation = self.generation.wrapping_add(1);
            self.revision = self.revision.wrapping_add(1);
        }
    }
}

impl Workspace {
    pub(in crate::workspace) fn library_context(&self) -> LibraryView {
        let mut view = LibraryView::default();
        let pane_id = self.active_runtime().focused_pane_id;
        let identity = self.agent_chat_identity(pane_id);
        view.local_management =
            identity.is_some() && self.right_views.library.manual_pane == Some(pane_id);
        if !view.local_management
            && identity.is_some()
            && let Some(captured) = self.right_views.library.launched.get(&pane_id)
        {
            return captured.clone();
        }
        let identity = if view.local_management {
            None
        } else {
            identity
        };
        let mut env = Vec::new();
        let cwd = if let Some((agent, cwd)) = identity {
            view.remote = matches!(cwd, Some(PaneCwd::Remote(_)));
            let spec = self.agent_launch_for(agent);
            view.provider = spec
                .as_ref()
                .and_then(|spec| match &spec.launch {
                    daruda_config::AgentLaunch::Ssh {
                        adapter_command, ..
                    }
                    | daruda_config::AgentLaunch::Docker {
                        adapter_command, ..
                    } => daruda_config::account_recipe_for_local_command(adapter_command),
                    launch => launch.account_recipe(false),
                })
                .map(provider);
            if let Some(spec) = spec {
                env = spec.env;
            }
            if view.remote {
                return view;
            }
            if matches!(
                AccountDomain::for_pane(&self.focused_account_pane()),
                AccountDomain::Unsupported
            ) {
                return view;
            }
            cwd.and_then(PaneCwd::as_local)
                .map(std::path::Path::to_path_buf)
        } else {
            view.provider = self.right_views.library.manual;
            self.active_lane_root()
        };
        let Some(selected) = view.provider else {
            return view;
        };
        let key = match selected {
            Provider::Claude => "CLAUDE_CONFIG_DIR",
            Provider::Codex => "CODEX_HOME",
        };
        let managed = match self.focused_account() {
            FocusedAccount::Managed {
                id,
                recipe,
                config_dir,
            } if provider(recipe) == selected && !view.local_management => {
                view.account_label = Some(
                    self.accounts
                        .find(id)
                        .and_then(|a| a.email.clone())
                        .unwrap_or_else(|| id.0.to_string()),
                );
                Some(config_dir)
            }
            _ => None,
        };
        let root = managed
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
            launch::effective_home(&env, None, dirs::home_dir()),
            root,
            cwd,
        );
        if view.config.is_none() {
            view.error = Some("Unresolved configuration root".into());
        }
        view
    }

    pub(in crate::workspace) fn pin_library(&mut self) -> Option<Arc<Configuration>> {
        let current = self.library_context().config?;
        let view = &self.right_views.library.view;
        if view.config.as_ref() != Some(&current) || view.loading || view.error.is_some() {
            self.right_views.library.selected_target = None;
            return None;
        }
        let config = Arc::new(current);
        self.right_views.library.pins.push(Arc::downgrade(&config));
        Some(config)
    }

    pub(in crate::workspace) fn refresh_library(&mut self, cx: &mut Context<Self>) {
        self.right_views.library.revision = self.right_views.library.revision.wrapping_add(1);
        cx.notify();
    }
}

fn provider(recipe: AccountRecipeId) -> Provider {
    match recipe {
        AccountRecipeId::Claude => Provider::Claude,
        AccountRecipeId::Codex => Provider::Codex,
    }
}

type Watch = crate::hooks::source_watch::SourceWatch;

fn watch(configs: &[Configuration], extra: Vec<PathBuf>) -> Watch {
    let mut paths: std::collections::BTreeSet<_> = extra.into_iter().collect();
    for config in configs {
        paths.extend([
            config.user_file.clone(),
            config.personal_skills.clone(),
            config.root.join("plugins"),
            config.root.join("skills"),
            config.root.join("settings.json"),
        ]);
        for dir in config.project_dirs() {
            paths.extend([
                config.project_file(&dir),
                config.project_skills(&dir),
                dir.join(".claude/settings.json"),
                dir.join(".claude/settings.local.json"),
            ]);
        }
    }
    if configs.iter().any(|c| c.provider == Provider::Claude)
        && let Some(dir) = crate::agent::configuration::library::readonly::managed_dir()
    {
        paths.extend([
            dir.join("managed-mcp.json"),
            dir.join("managed-settings.json"),
        ]);
    }
    Watch::new(paths)
}

pub(in crate::workspace) fn spawn(cx: &mut Context<Workspace>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        let mut cache: HashMap<Configuration, Inventory> = HashMap::new();
        let mut watched = Vec::new();
        let mut watched_extra = std::collections::BTreeSet::new();
        let mut subscription: Option<Watch> = None;
        let mut previous = None;
        let mut revision = 0;
        loop {
            cx.background_executor().timer(Duration::from_millis(200)).await;
            let Ok((context, desired, next_revision, generation)) = this.update(cx, |ws, cx| {
                let stale: Vec<_> = ws.right_views.library.launched.keys().copied().filter(|id| ws.agent_chat_identity(*id).is_none()).collect();
                for id in stale { ws.right_views.library.launched.remove(&id); }
                let context = ws.library_context();
                ws.right_views.library.observe_context(&context);
                ws.right_views.library.pins.retain(|p| p.strong_count() > 0);
                let mut desired: Vec<_> = ws.right_views.library.pins.iter().filter_map(Weak::upgrade).map(|p| (*p).clone()).collect();
                if ws.docks.right.read(cx).is_open && ws.docks.right_view != daruda_store::project::RightDockView::Usage {
                    desired.extend(context.config.clone());
                }
                let mut unique = Vec::new();
                for config in desired { if !unique.contains(&config) { unique.push(config); } }
                (context, unique, ws.right_views.library.revision, ws.right_views.library.generation)
            }) else { break };
            let event = subscription.as_ref().is_some_and(Watch::changed);
            let extra: std::collections::BTreeSet<_> = cache.iter().filter(|(key,_)| desired.contains(key)).flat_map(|(_,inventory)| {
                inventory.groups.iter().map(|g| g.target.path.clone()).chain(inventory.skills.project.iter().chain(inventory.skills.personal.iter()).chain(inventory.skills.plugin.iter()).map(|s| s.dir.clone()))
            }).collect();
            let sources_changed = extra != watched_extra || desired.len() != watched.len() || desired.iter().any(|c| !watched.contains(c));
            if sources_changed || event {
                // Re-resolve canonical anchors after directory/symlink replacement.
                // A single subscription deduplicates visible and dialog sources.
                subscription = None;
                if !desired.is_empty() {
                    let input = desired.clone();
                    let extra_paths = extra.iter().cloned().collect();
                    subscription = Some(cx.background_executor().spawn(async move { watch(&input, extra_paths) }).await);
                }
                watched = desired.clone();
                watched_extra = extra;
            }
            cache.retain(|key, _| desired.contains(key));
            let changed = previous != context.config;
            previous = context.config.clone();
            let refresh = sources_changed || event || changed || next_revision != revision;
            revision = next_revision;
            if this.update(cx, |ws, cx| {
                let old = &ws.right_views.library.view;
                if old.config != context.config || old.remote != context.remote || old.provider != context.provider || old.local_management != context.local_management {
                    ws.right_views.library.view = context.clone();
                    ws.right_views.library.view.loading = context.config.is_some();
                    cx.notify();
                } else if sources_changed && context.config.is_some() {
                    ws.right_views.library.view.loading = true;
                    cx.notify();
                }
            }).is_err() { break; }
            if !refresh { continue; }
            for config in desired {
                let input = config.clone();
                let result = cx.background_executor().spawn(async move { discover(&input) }).await;
                if let Ok(inventory) = &result { cache.insert(config.clone(), inventory.clone()); }
                if this.update(cx, |ws, cx| {
                    let current = ws.library_context();
                    if current.config.as_ref() != Some(&config) || ws.right_views.library.generation != generation { return; }
                    let mut view = current;
                    view.inventory = cache.get(&config).cloned().unwrap_or_default();
                    view.error = result.as_ref().err().cloned();
                    if view.error.is_some() && ws.right_views.library.view.error.is_none() {
                        ws.report_error(daruda_store::observability::error_report::ErrorReport::new(crate::surface::strings::error::mcp_reload_failed(config.user_file.display())).severity(daruda_store::observability::error_report::ErrorSeverity::Warning).at(file!(), line!()).dedup("library.reload").build(), cx);
                    }
                    ws.right_views.library.view = view;
                    cx.notify();
                }).is_err() { return; }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn returning_to_a_context_advances_generation_and_a_dialog_pin_outlives_focus() {
        let temp = tempfile::tempdir().unwrap();
        let a =
            Configuration::resolve(Provider::Claude, Some(temp.path().into()), None, None).unwrap();
        let b =
            Configuration::resolve(Provider::Codex, Some(temp.path().into()), None, None).unwrap();
        let mut library = Library::default();
        let view = |config| LibraryView {
            config: Some(config),
            ..Default::default()
        };
        library.observe_context(&view(a.clone()));
        let first = library.generation;
        let pin = Arc::new(a.clone());
        library.pins.push(Arc::downgrade(&pin));
        library.observe_context(&view(b));
        library.observe_context(&view(a));
        assert_ne!(first, library.generation);
        assert!(library.pins[0].upgrade().is_some());
        drop(pin);
        assert!(library.pins[0].upgrade().is_none());
    }
    #[test]
    fn configuration_watch_detects_first_file_creation() {
        let _serial = crate::hooks::tests_common::fsevent_serial();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("account");
        let config = Configuration::resolve(
            Provider::Claude,
            Some(temp.path().into()),
            Some(root.clone()),
            None,
        )
        .unwrap();
        let subscription = watch(std::slice::from_ref(&config), Vec::new());
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(config.user_file, "{}").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !subscription.changed() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
