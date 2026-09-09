//! Typed registry for open Workspace and singleton windows.
//!
//! Keeps window lookup in one GPUI global instead of scattering
//! `cx.windows() + downcast::<T>()` loops. Singleton handles store the inner
//! entity as well as the window handle because wrapped roots cannot always be
//! recovered by downcast.

use std::collections::HashSet;

use gpui::{AnyWindowHandle, App, AppContext, Context, Entity, Global, WeakEntity, Window};

use crate::settings_window::SettingsWindow;
use crate::welcome::WelcomeScreen;
use crate::workspace::Workspace;

/// Settings singleton handle; stores the inner entity because the window root
/// is `gpui_component::Root`, not `SettingsWindow`.
#[derive(Clone)]
pub(crate) struct SettingsHandle {
    window: AnyWindowHandle,
    inner: WeakEntity<SettingsWindow>,
}

impl SettingsHandle {
    /// Run `f` against the live `SettingsWindow`; `None` means reopen it.
    pub(crate) fn update<R>(
        &self,
        cx: &mut App,
        f: impl FnOnce(&mut SettingsWindow, &mut Window, &mut Context<SettingsWindow>) -> R,
    ) -> Option<R> {
        let inner = self.inner.upgrade()?;
        cx.update_window(self.window, |_root, window, cx_w| {
            inner.update(cx_w, |this, cx| f(this, window, cx))
        })
        .ok()
    }
}

/// Welcome singleton handle; mirrors [`SettingsHandle`] so singleton lifecycle
/// stays symmetric.
#[derive(Clone)]
pub(crate) struct WelcomeHandle {
    window: AnyWindowHandle,
    inner: WeakEntity<WelcomeScreen>,
}

impl WelcomeHandle {
    /// Upgrade to the live entity, or `None` if the window has closed.
    pub(crate) fn upgrade(&self) -> Option<Entity<WelcomeScreen>> {
        self.inner.upgrade()
    }
}

/// GPUI global mapping open windows to their typed workspace/singleton entity.
/// `Default` lets tests and production paths create it lazily via
/// `default_global`, without a separate init call.
#[derive(Default)]
pub(crate) struct WindowRegistry {
    workspaces: Vec<(AnyWindowHandle, WeakEntity<Workspace>)>,
    settings: Option<SettingsHandle>,
    welcome: Option<WelcomeHandle>,
    /// The workspace hosting the app-global orchestrator session.
    orchestrator: Option<(AnyWindowHandle, WeakEntity<Workspace>)>,
}

impl Global for WindowRegistry {}

impl WindowRegistry {
    /// Add a workspace once construction has produced its window handle.
    pub(crate) fn register(
        handle: AnyWindowHandle,
        workspace: WeakEntity<Workspace>,
        cx: &mut App,
    ) {
        cx.default_global::<WindowRegistry>()
            .workspaces
            .push((handle, workspace));
    }

    /// Remove a workspace from the `cx.on_release` hook.
    pub(crate) fn deregister(workspace: &WeakEntity<Workspace>, cx: &mut App) {
        if cx.try_global::<WindowRegistry>().is_some() {
            cx.global_mut::<WindowRegistry>()
                .workspaces
                .retain(|(_, w)| w != workspace);
        }
    }

    /// Apply `f` to every live workspace and lazily prune closed windows.
    ///
    /// The orchestrator's host remains a normal workspace in this list.
    pub(crate) fn for_each_workspace<F>(cx: &mut App, f: F)
    where
        F: FnMut(&mut Workspace, &mut gpui::Window, &mut gpui::Context<Workspace>),
    {
        let pairs = Self::workspace_pairs(cx);
        Self::for_each_pair(pairs, cx, f);
    }

    /// Snapshot the workspace list so the shared borrow on the global is released
    /// before any per-window update closure runs.
    fn workspace_pairs(cx: &App) -> Vec<(AnyWindowHandle, WeakEntity<Workspace>)> {
        cx.try_global::<WindowRegistry>()
            .map(|r| r.workspaces.clone())
            .unwrap_or_default()
    }

    /// Walk live workspace handles and prune closed windows.
    fn for_each_pair<F>(
        pairs: Vec<(AnyWindowHandle, WeakEntity<Workspace>)>,
        cx: &mut App,
        mut f: F,
    ) where
        F: FnMut(&mut Workspace, &mut gpui::Window, &mut gpui::Context<Workspace>),
    {
        let mut stale: HashSet<AnyWindowHandle> = HashSet::new();
        for (handle, weak) in pairs {
            // Enter via the root-type-agnostic window handle, then route into
            // the inner `Workspace` entity.
            let result = cx.update_window(handle, |_root, window, cx_w| {
                let Some(ws) = weak.upgrade() else {
                    return;
                };
                ws.update(cx_w, |ws, cx| f(ws, window, cx));
            });
            if result.is_err() {
                stale.insert(handle);
            }
        }
        if !stale.is_empty() && cx.try_global::<WindowRegistry>().is_some() {
            let registry = cx.global_mut::<WindowRegistry>();
            registry.workspaces.retain(|(h, _)| !stale.contains(h));
            if registry
                .orchestrator
                .as_ref()
                .is_some_and(|(h, _)| stale.contains(h))
            {
                registry.orchestrator = None;
            }
        }
    }

    /// All registered workspace window handles.
    #[allow(dead_code)]
    pub(crate) fn all_handles(cx: &App) -> Vec<AnyWindowHandle> {
        cx.try_global::<WindowRegistry>()
            .map(|r| r.workspaces.iter().map(|(h, _)| *h).collect())
            .unwrap_or_default()
    }

    /// Atomically drain handles; a second close-all caller sees an empty list.
    pub(crate) fn drain_handles(cx: &mut App) -> Vec<AnyWindowHandle> {
        if cx.try_global::<WindowRegistry>().is_none() {
            return Vec::new();
        }
        let registry = cx.global_mut::<WindowRegistry>();
        let handles: Vec<AnyWindowHandle> = registry.workspaces.iter().map(|(h, _)| *h).collect();
        registry.orchestrator = None;
        registry.workspaces.clear();
        handles
    }

    /// Return the active window when it is a registered Workspace.
    pub(crate) fn active_workspace_handle(cx: &App) -> Option<AnyWindowHandle> {
        let active = cx.active_window()?;
        cx.try_global::<WindowRegistry>()?
            .workspaces
            .iter()
            .find(|(h, _)| *h == active)
            .map(|(h, _)| *h)
    }

    /// Return `(handle, weak_entity)` for the active workspace.
    pub(crate) fn active_workspace(cx: &App) -> Option<(AnyWindowHandle, WeakEntity<Workspace>)> {
        let active = cx.active_window()?;
        cx.try_global::<WindowRegistry>()?
            .workspaces
            .iter()
            .find(|(h, _)| *h == active)
            .map(|(h, w)| (*h, w.clone()))
    }

    /// First registered workspace — used by screenshot runs where no
    /// OS-focused active window exists, and by the Settings window
    /// (a separate OS window with no `Workspace` of its own) to pick a
    /// concrete target for an action that must run against *some* live
    /// Workspace (e.g. the Accounts section's add-account button — see
    /// `settings_window::sections::accounts::start_add_account`).
    /// Deterministic (registration order) but arbitrary when more than one
    /// workspace window is open; documented simplification, same class as
    /// `Workspace::panes_referencing_account`'s per-window undercount.
    pub(crate) fn first_workspace(cx: &App) -> Option<(AnyWindowHandle, WeakEntity<Workspace>)> {
        cx.try_global::<WindowRegistry>()?
            .workspaces
            .first()
            .cloned()
    }

    /// Remember the host without changing its normal workspace registration.
    pub(crate) fn register_orchestrator(
        handle: AnyWindowHandle,
        workspace: WeakEntity<Workspace>,
        cx: &mut App,
    ) {
        let registry = cx.default_global::<WindowRegistry>();
        let already_this_host = registry
            .orchestrator
            .as_ref()
            .is_some_and(|(_, held)| held.entity_id() == workspace.entity_id());
        registry.orchestrator = Some((handle, workspace.clone()));
        if already_this_host {
            // The hook below is already installed for this entity. Registering
            // the same host twice happens on a retried start, and a second
            // subscription would accumulate one per attempt.
            return;
        }
        // Separate from the constructor's own release hook, which knows only
        // about the workspace list. It clears through `clear_orchestrator_if`
        // so a host released *after* another window has taken the slot cannot
        // wipe the new entry.
        if let Some(entity) = workspace.upgrade() {
            entity
                .update(cx, |_, cx| {
                    cx.on_release(move |_, cx| Self::clear_orchestrator_if(&workspace, cx))
                })
                .detach();
        }
    }

    /// Drop the orchestrator entry unconditionally — used when replacing one,
    /// where the slot is about to be refilled.
    pub(crate) fn clear_orchestrator(cx: &mut App) {
        if cx.try_global::<WindowRegistry>().is_some() {
            cx.global_mut::<WindowRegistry>().orchestrator = None;
        }
    }

    /// Drop the orchestrator entry only if it still names `workspace`.
    ///
    /// What a released window's `cx.on_release` hook must call: replacing the
    /// orchestrator closes the old window *after* the new one has taken the
    /// slot, so an unconditional clear from the old hook would evict the
    /// replacement.
    pub(crate) fn clear_orchestrator_if(workspace: &WeakEntity<Workspace>, cx: &mut App) {
        if cx.try_global::<WindowRegistry>().is_some() {
            let registry = cx.global_mut::<WindowRegistry>();
            if registry
                .orchestrator
                .as_ref()
                .is_some_and(|(_, w)| w == workspace)
            {
                registry.orchestrator = None;
            }
        }
    }

    /// Return the live orchestrator entry, if one is up.
    pub(crate) fn orchestrator(cx: &App) -> Option<(AnyWindowHandle, WeakEntity<Workspace>)> {
        cx.try_global::<WindowRegistry>()?.orchestrator.clone()
    }

    /// Record the live Settings singleton.
    pub(crate) fn register_settings(
        window: AnyWindowHandle,
        inner: WeakEntity<SettingsWindow>,
        cx: &mut App,
    ) {
        cx.default_global::<WindowRegistry>().settings = Some(SettingsHandle { window, inner });
    }

    /// Drop the Settings singleton entry from its `cx.on_release` hook.
    pub(crate) fn clear_settings(cx: &mut App) {
        if cx.try_global::<WindowRegistry>().is_some() {
            cx.global_mut::<WindowRegistry>().settings = None;
        }
    }

    /// Return the open Settings handle, if any.
    pub(crate) fn settings(cx: &App) -> Option<SettingsHandle> {
        cx.try_global::<WindowRegistry>()?.settings.clone()
    }

    /// Record the live Welcome singleton.
    pub(crate) fn register_welcome(
        window: AnyWindowHandle,
        inner: WeakEntity<WelcomeScreen>,
        cx: &mut App,
    ) {
        cx.default_global::<WindowRegistry>().welcome = Some(WelcomeHandle { window, inner });
    }

    /// Drop the Welcome singleton entry from its `cx.on_release` hook.
    pub(crate) fn clear_welcome(cx: &mut App) {
        if cx.try_global::<WindowRegistry>().is_some() {
            cx.global_mut::<WindowRegistry>().welcome = None;
        }
    }

    /// Return the open Welcome handle, if any.
    pub(crate) fn welcome(cx: &App) -> Option<WelcomeHandle> {
        cx.try_global::<WindowRegistry>()?.welcome.clone()
    }

    /// Look up the window that owns a workspace entity.
    pub(crate) fn handle_for_workspace(
        entity_id: gpui::EntityId,
        cx: &App,
    ) -> Option<AnyWindowHandle> {
        cx.try_global::<WindowRegistry>()?
            .workspaces
            .iter()
            .find(|(_, weak)| weak.entity_id() == entity_id)
            .map(|(h, _)| *h)
    }

    /// Inverse of [`Self::handle_for_workspace`], used by cached pane entities
    /// that need to dispatch into their owning `Workspace`.
    pub(crate) fn workspace_for_window(
        handle: AnyWindowHandle,
        cx: &App,
    ) -> Option<WeakEntity<Workspace>> {
        cx.try_global::<WindowRegistry>()?
            .workspaces
            .iter()
            .find(|(h, _)| *h == handle)
            .map(|(_, weak)| weak.clone())
    }

    /// Return the open Welcome window handle, if any.
    pub(crate) fn welcome_window(cx: &App) -> Option<AnyWindowHandle> {
        cx.try_global::<WindowRegistry>()?
            .welcome
            .as_ref()
            .map(|h| h.window)
    }

    /// Return the open Settings window handle, if any.
    #[cfg(feature = "screenshot")]
    pub(crate) fn settings_window(cx: &App) -> Option<AnyWindowHandle> {
        cx.try_global::<WindowRegistry>()?
            .settings
            .as_ref()
            .map(|h| h.window)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    use crate::workspace::Workspace;

    fn test_data_dir() -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!("daruda_wr_test_{id}"))
    }

    #[test]
    fn default_registry_is_empty() {
        let registry = WindowRegistry::default();
        assert!(registry.workspaces.is_empty());
    }

    #[gpui::test]
    fn all_handles_returns_empty_without_global(cx: &mut TestAppContext) {
        cx.update(|cx| {
            assert!(WindowRegistry::all_handles(cx).is_empty());
        });
    }

    #[gpui::test]
    fn for_each_workspace_noop_when_empty(cx: &mut TestAppContext) {
        let mut visited = 0usize;
        cx.update(|cx| {
            WindowRegistry::for_each_workspace(cx, |_, _, _| {
                visited += 1;
            });
        });
        assert_eq!(visited, 0);
    }

    /// What [`make_window`] hands back: the window and the `Workspace` inside
    /// it, both of which a test has to keep alive for the registry entry to
    /// stay upgradable.
    type TestWindow = (
        gpui::WindowHandle<gpui_component::Root>,
        gpui::Entity<Workspace>,
    );

    /// Construct the same `Root`-wrapped Workspace shape used in production.
    fn make_window(cx: &mut TestAppContext, config: &daruda_config::Config) -> TestWindow {
        crate::test_support::init_gpui_component(cx);
        let workspace_for_root = std::cell::RefCell::new(None);
        let wh = cx.add_window(|window, cx| {
            let workspace = cx.new(|cx| Workspace::new(config, test_data_dir(), window, cx));
            *workspace_for_root.borrow_mut() = Some(workspace.clone());
            gpui_component::Root::new(workspace, window, cx)
        });
        let workspace = workspace_for_root.borrow().clone().unwrap();
        (wh, workspace)
    }

    #[gpui::test]
    fn single_workspace_registers_and_appears_in_all_handles(cx: &mut TestAppContext) {
        let config = daruda_config::Config::default();
        let (_wh, _ws) = make_window(cx, &config);

        cx.update(|cx| {
            assert_eq!(WindowRegistry::all_handles(cx).len(), 1);
        });
    }

    #[gpui::test]
    fn two_workspaces_both_appear_in_all_handles(cx: &mut TestAppContext) {
        let config = daruda_config::Config::default();
        let (_wh1, _ws1) = make_window(cx, &config);
        let (_wh2, _ws2) = make_window(cx, &config);

        cx.update(|cx| {
            assert_eq!(WindowRegistry::all_handles(cx).len(), 2);
        });
    }

    #[gpui::test]
    fn for_each_workspace_visits_all_registered(cx: &mut TestAppContext) {
        let config = daruda_config::Config::default();
        let (_wh1, _ws1) = make_window(cx, &config);
        let (_wh2, _ws2) = make_window(cx, &config);

        let mut visited = 0usize;
        cx.update(|cx| {
            WindowRegistry::for_each_workspace(cx, |_, _, _| {
                visited += 1;
            });
        });
        assert_eq!(visited, 2);
    }

    #[gpui::test]
    fn deregister_removes_entry(cx: &mut TestAppContext) {
        let config = daruda_config::Config::default();
        let (_wh, workspace) = make_window(cx, &config);

        cx.update(|cx| {
            assert_eq!(WindowRegistry::all_handles(cx).len(), 1);
            let weak = workspace.downgrade();
            WindowRegistry::deregister(&weak, cx);
            assert_eq!(WindowRegistry::all_handles(cx).len(), 0);
        });
    }

    #[gpui::test]
    fn drain_handles_clears_registry(cx: &mut TestAppContext) {
        let config = daruda_config::Config::default();
        let (_wh1, _ws1) = make_window(cx, &config);
        let (_wh2, _ws2) = make_window(cx, &config);

        cx.update(|cx| {
            let drained = WindowRegistry::drain_handles(cx);
            assert_eq!(drained.len(), 2);
            assert!(WindowRegistry::all_handles(cx).is_empty());
        });
    }

    /// Two workspaces, one hosting the orchestrator. Both stay live for the caller.
    fn register_user_and_orchestrator(cx: &mut TestAppContext) -> (TestWindow, TestWindow) {
        // `make_window` goes through the production `Workspace::new`, which
        // registers itself, so only the host registration below is explicit here.
        let config = daruda_config::Config::default();
        let user = make_window(cx, &config);
        let orch = make_window(cx, &config);
        cx.update(|cx| {
            WindowRegistry::register_orchestrator(orch.0.into(), orch.1.downgrade(), cx);
        });
        (user, orch)
    }

    #[gpui::test]
    fn drain_handles_includes_the_orchestrator(cx: &mut TestAppContext) {
        let (_user, _orch) = register_user_and_orchestrator(cx);
        cx.update(|cx| {
            assert_eq!(
                WindowRegistry::drain_handles(cx).len(),
                2,
                "close-all must actually close it"
            );
            assert!(
                WindowRegistry::orchestrator(cx).is_none(),
                "drain clears the slot"
            );
        });
    }

    /// Hosting a session must neither remove nor duplicate the workspace.
    #[gpui::test]
    fn registering_the_orchestrator_preserves_the_workspace_list(cx: &mut TestAppContext) {
        let config = daruda_config::Config::default();
        let (handle, ws) = make_window(cx, &config);
        cx.update(|cx| {
            assert_eq!(WindowRegistry::all_handles(cx).len(), 1);
            WindowRegistry::register_orchestrator(handle.into(), ws.downgrade(), cx);
            assert_eq!(WindowRegistry::all_handles(cx).len(), 1);
            let mut seen = 0usize;
            WindowRegistry::for_each_workspace(cx, |_, _, _| seen += 1);
            assert_eq!(seen, 1, "walked once, not twice");
        });
    }

    /// A replaced orchestrator's window is closed *after* the replacement took
    /// the slot, so its release hook must not evict the live one.
    #[gpui::test]
    fn a_released_predecessor_does_not_evict_its_replacement(cx: &mut TestAppContext) {
        let config = daruda_config::Config::default();
        let first = make_window(cx, &config);
        let second = make_window(cx, &config);
        cx.update(|cx| {
            WindowRegistry::register_orchestrator(first.0.into(), first.1.downgrade(), cx);
            WindowRegistry::register_orchestrator(second.0.into(), second.1.downgrade(), cx);
            // The predecessor's hook fires now, naming itself.
            WindowRegistry::clear_orchestrator_if(&first.1.downgrade(), cx);
            let held = WindowRegistry::orchestrator(cx).expect("replacement still held");
            assert_eq!(held.1, second.1.downgrade());

            WindowRegistry::clear_orchestrator_if(&second.1.downgrade(), cx);
            assert!(WindowRegistry::orchestrator(cx).is_none());
        });
    }

    #[gpui::test]
    fn only_one_orchestrator_is_held(cx: &mut TestAppContext) {
        let (_user, _orch) = register_user_and_orchestrator(cx);
        let config = daruda_config::Config::default();
        let second = make_window(cx, &config);
        cx.update(|cx| {
            WindowRegistry::register_orchestrator(second.0.into(), second.1.downgrade(), cx);
            let mut seen = 0usize;
            WindowRegistry::for_each_workspace(cx, |_, _, _| seen += 1);
            assert_eq!(
                seen, 3,
                "replacing the host does not remove either workspace"
            );
        });
    }

    #[gpui::test]
    fn orchestrator_host_remains_an_active_workspace(cx: &mut TestAppContext) {
        let (_, host) = register_user_and_orchestrator(cx);
        cx.update_window(host.0.into(), |_, window, _| window.activate_window())
            .unwrap();
        cx.update(|cx| {
            assert_eq!(
                WindowRegistry::active_workspace(cx).unwrap().0,
                host.0.into()
            );
        });
    }

    #[gpui::test]
    fn clearing_the_orchestrator_leaves_the_user_list_alone(cx: &mut TestAppContext) {
        let (_user, _orch) = register_user_and_orchestrator(cx);
        cx.update(|cx| {
            WindowRegistry::clear_orchestrator(cx);
            assert!(WindowRegistry::orchestrator(cx).is_none());
            assert_eq!(WindowRegistry::all_handles(cx).len(), 2);
        });
    }

    #[gpui::test]
    fn double_deregister_is_idempotent(cx: &mut TestAppContext) {
        let config = daruda_config::Config::default();
        let (_wh, workspace) = make_window(cx, &config);

        cx.update(|cx| {
            let weak = workspace.downgrade();
            WindowRegistry::deregister(&weak, cx);
            WindowRegistry::deregister(&weak, cx);
            assert!(WindowRegistry::all_handles(cx).is_empty());
        });
    }
}
