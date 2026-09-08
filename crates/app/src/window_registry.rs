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
    /// The app-global orchestrator window. An `Option` rather than a flagged
    /// entry in `workspaces`, so the singleton constraint is the type's and
    /// not a comment's.
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
    /// Includes the orchestrator: it is a `Workspace`, so per-window machinery
    /// — the status pulse that drives completion settle, the deferred relay
    /// flush, a targeted `PaneRef` dispatch — has to reach it. It is kept out
    /// of `workspaces` only so the Welcome policy and `/list` do not count it
    /// as the user's work.
    pub(crate) fn for_each_workspace<F>(cx: &mut App, f: F)
    where
        F: FnMut(&mut Workspace, &mut gpui::Window, &mut gpui::Context<Workspace>),
    {
        let mut pairs = Self::user_pairs(cx);
        if let Some(orch) = cx
            .try_global::<WindowRegistry>()
            .and_then(|r| r.orchestrator.clone())
        {
            pairs.push(orch);
        }
        Self::for_each_pair(pairs, cx, f);
    }

    /// Apply `f` to every *user* workspace — the ones the user opened.
    ///
    /// Distinct from [`Self::for_each_workspace`], which also reaches the
    /// orchestrator window. A caller answering "what is the user working on?"
    /// wants this one; a caller driving per-window machinery wants the other.
    pub(crate) fn for_each_user_workspace<F>(cx: &mut App, f: F)
    where
        F: FnMut(&mut Workspace, &mut gpui::Window, &mut gpui::Context<Workspace>),
    {
        let pairs = Self::user_pairs(cx);
        Self::for_each_pair(pairs, cx, f);
    }

    /// Snapshot the user list so the shared borrow on the global is released
    /// before any per-window update closure runs.
    fn user_pairs(cx: &App) -> Vec<(AnyWindowHandle, WeakEntity<Workspace>)> {
        cx.try_global::<WindowRegistry>()
            .map(|r| r.workspaces.clone())
            .unwrap_or_default()
    }

    /// The walk both `for_each_*` variants share, including the lazy prune of
    /// windows that have since closed. Only the pair list differs between them.
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
        let mut handles: Vec<AnyWindowHandle> =
            registry.workspaces.iter().map(|(h, _)| *h).collect();
        // Close-all means every window daruda opened, orchestrator included —
        // and taking the slot here is what stops a second caller re-closing it.
        if let Some((handle, _)) = registry.orchestrator.take() {
            handles.push(handle);
        }
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

    /// Record the orchestrator window: move it out of the user list, take the
    /// slot, and arm its teardown.
    ///
    /// All three, not just the third, because each of the other two is a
    /// correctness requirement a caller could otherwise forget.
    /// `Workspace::new` registers every window it builds, so a workspace
    /// promoted here would otherwise sit in both places — walked twice by
    /// [`Self::for_each_workspace`] and counted as the user's by
    /// [`Self::all_handles`]. And a slot nobody clears would keep naming a
    /// closed window, so the next request would address a dead pane instead of
    /// starting a fresh orchestrator.
    ///
    /// Replaces any previous orchestrator; the slot is an `Option` because a
    /// second one would compete for the same conversation.
    pub(crate) fn register_orchestrator(
        handle: AnyWindowHandle,
        workspace: WeakEntity<Workspace>,
        cx: &mut App,
    ) {
        let registry = cx.default_global::<WindowRegistry>();
        registry.workspaces.retain(|(_, w)| *w != workspace);
        registry.orchestrator = Some((handle, workspace.clone()));
        // Separate from the constructor's own release hook, which knows only
        // about the user list. Conditional, because replacing an orchestrator
        // closes the old window *after* the new one has taken the slot.
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

    /// `true` when the OS-active window is the orchestrator's.
    pub(crate) fn active_is_orchestrator(cx: &App) -> bool {
        let Some(active) = cx.active_window() else {
            return false;
        };
        cx.try_global::<WindowRegistry>()
            .and_then(|r| r.orchestrator.as_ref())
            .is_some_and(|(handle, _)| *handle == active)
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

    /// One user window plus a registered orchestrator, which is the shape every
    /// split below is about. Returns both weaks so a caller can keep them live.
    fn register_user_and_orchestrator(cx: &mut TestAppContext) -> (TestWindow, TestWindow) {
        // `make_window` goes through the production `Workspace::new`, which
        // registers itself — so only the promotion below is explicit here.
        let config = daruda_config::Config::default();
        let user = make_window(cx, &config);
        let orch = make_window(cx, &config);
        cx.update(|cx| {
            WindowRegistry::register_orchestrator(orch.0.into(), orch.1.downgrade(), cx);
        });
        (user, orch)
    }

    #[gpui::test]
    fn for_each_workspace_reaches_the_orchestrator(cx: &mut TestAppContext) {
        let (_user, _orch) = register_user_and_orchestrator(cx);
        let mut seen = 0usize;
        cx.update(|cx| WindowRegistry::for_each_workspace(cx, |_, _, _| seen += 1));
        assert_eq!(seen, 2, "pulse must reach the orchestrator too");
    }

    #[gpui::test]
    fn for_each_user_workspace_excludes_the_orchestrator(cx: &mut TestAppContext) {
        let (_user, _orch) = register_user_and_orchestrator(cx);
        let mut seen = 0usize;
        cx.update(|cx| WindowRegistry::for_each_user_workspace(cx, |_, _, _| seen += 1));
        assert_eq!(seen, 1, "a listing must not mix the orchestrator in");
    }

    #[gpui::test]
    fn all_handles_excludes_the_orchestrator(cx: &mut TestAppContext) {
        let (_user, _orch) = register_user_and_orchestrator(cx);
        cx.update(|cx| {
            assert_eq!(
                WindowRegistry::all_handles(cx).len(),
                1,
                "the Welcome policy asks about user windows only"
            );
        });
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

    /// Registering the orchestrator takes it *out* of the user list: the
    /// `Workspace` constructor registers every window it builds, so without
    /// this the same window would be walked twice and counted as the user's.
    #[gpui::test]
    fn registering_the_orchestrator_removes_it_from_the_user_list(cx: &mut TestAppContext) {
        let config = daruda_config::Config::default();
        let (handle, ws) = make_window(cx, &config);
        cx.update(|cx| {
            assert_eq!(WindowRegistry::all_handles(cx).len(), 1);
            WindowRegistry::register_orchestrator(handle.into(), ws.downgrade(), cx);
            assert!(WindowRegistry::all_handles(cx).is_empty());
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
                seen, 2,
                "registering a second orchestrator replaces the first"
            );
        });
    }

    #[gpui::test]
    fn the_orchestrator_is_recognisable_as_the_active_window(cx: &mut TestAppContext) {
        let (user, orch) = register_user_and_orchestrator(cx);
        let activate = |handle: gpui::AnyWindowHandle, cx: &mut TestAppContext| {
            cx.update_window(handle, |_, window, _| window.activate_window())
                .expect("window is live");
        };

        activate(orch.0.into(), cx);
        cx.update(|cx| {
            assert!(WindowRegistry::active_is_orchestrator(cx));
            assert!(
                WindowRegistry::active_workspace(cx).is_none(),
                "it must never answer as a user workspace: a caller that would \
                 add a project to the active one has to keep falling through"
            );
        });

        activate(user.0.into(), cx);
        cx.update(|cx| {
            assert!(!WindowRegistry::active_is_orchestrator(cx));
            assert!(WindowRegistry::active_workspace(cx).is_some());
        });

        activate(orch.0.into(), cx);
        cx.update(|cx| {
            WindowRegistry::clear_orchestrator(cx);
            assert!(
                !WindowRegistry::active_is_orchestrator(cx),
                "an empty slot names no window"
            );
        });
    }

    #[gpui::test]
    fn clearing_the_orchestrator_leaves_the_user_list_alone(cx: &mut TestAppContext) {
        let (_user, _orch) = register_user_and_orchestrator(cx);
        cx.update(|cx| {
            WindowRegistry::clear_orchestrator(cx);
            assert!(WindowRegistry::orchestrator(cx).is_none());
            assert_eq!(WindowRegistry::all_handles(cx).len(), 1);
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
