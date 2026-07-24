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
    pub(crate) fn for_each_workspace<F>(cx: &mut App, mut f: F)
    where
        F: FnMut(&mut Workspace, &mut gpui::Window, &mut gpui::Context<Workspace>),
    {
        // Snapshot (handle, weak) pairs so the shared borrow on the global is
        // released before we start calling per-window update closures.
        let pairs: Vec<(AnyWindowHandle, WeakEntity<Workspace>)> = cx
            .try_global::<WindowRegistry>()
            .map(|r| r.workspaces.clone())
            .unwrap_or_default();

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
            cx.global_mut::<WindowRegistry>()
                .workspaces
                .retain(|(h, _)| !stale.contains(h));
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

    /// Construct the same `Root`-wrapped Workspace shape used in production.
    fn make_window(
        cx: &mut TestAppContext,
        config: &daruda_config::Config,
    ) -> (
        gpui::WindowHandle<gpui_component::Root>,
        gpui::Entity<Workspace>,
    ) {
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
