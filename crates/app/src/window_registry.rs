//! Typed registry of open Workspace + singleton windows.
//!
//! Replaces the scattered `cx.windows() + downcast::<T>()` loops with a single
//! authoritative source of truth stored as a GPUI global.
//!
//! # Workspace windows vs. singleton windows
//!
//! `WindowRegistry` tracks both Workspace windows (many) and the Settings
//! singleton (at most one). The Settings entry is needed because the Settings
//! window root is `gpui_component::Root` wrapping a `SettingsWindow` (so
//! `gpui_component::Input::TextElement::paint` can call `Root::read` without
//! panicking), which means a plain `cx.windows().downcast::<SettingsWindow>()`
//! search cannot recover the inner entity.
//!
//! # Lifecycle
//!
//! - **Workspace registration**: `Workspace::new_with_project` calls
//!   `register` after the struct is fully initialised. A `cx.on_release` hook
//!   in the same constructor calls `deregister` when the entity drops.
//! - **Settings registration**: `SettingsWindow::new_with_section` calls
//!   `register_settings` after construction; its `cx.on_release` hook calls
//!   `clear_settings` when the entity drops (window close).
//!
//! # Usage
//!
//! ```rust
//! // Broadcast to every open workspace
//! WindowRegistry::for_each_workspace(cx, |ws, _window, cx| {
//!     ws.reload_config(&config, cx);
//! });
//!
//! // Close all workspace windows (atomically drains the registry)
//! let handles = WindowRegistry::drain_handles(cx);
//!
//! // Replace the window that initiated an open action
//! if let Some(h) = WindowRegistry::active_workspace_handle(cx) { ... }
//!
//! // Focus an already-open Settings window (or detect that it is closed)
//! if let Some(sh) = WindowRegistry::settings(cx) {
//!     sh.update(cx, |this, window, cx| this.focus_section(section, window, cx));
//! }
//! ```

use std::collections::HashSet;

use gpui::{AnyWindowHandle, App, AppContext, Context, Entity, Global, WeakEntity, Window};

use crate::settings_window::SettingsWindow;
use crate::welcome::WelcomeScreen;
use crate::workspace::Workspace;

/// Bundled `(window handle, inner entity)` pair for the Settings singleton.
/// The window's root view is `gpui_component::Root`, so `cx.update_window`
/// targets the window via the handle and then routes mutation into the
/// inner `SettingsWindow` entity through the weak ref.
#[derive(Clone)]
pub(crate) struct SettingsHandle {
    window: AnyWindowHandle,
    inner: WeakEntity<SettingsWindow>,
}

impl SettingsHandle {
    /// Run `f` against the live `SettingsWindow` entity. Returns `None` if
    /// either the window or the inner entity is gone (the caller's open path
    /// should then fall through to creating a fresh window).
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

/// Bundled `(window handle, inner entity)` pair for the Welcome singleton.
/// Mirrors [`SettingsHandle`]: the Welcome window root is `WelcomeScreen`
/// directly (no `gpui_component::Root` wrapper), so the inner entity could
/// also be recovered via `downcast`, but storing it here avoids that fragile
/// path and makes the lifecycle symmetric with the other singletons.
#[derive(Clone)]
pub(crate) struct WelcomeHandle {
    window: AnyWindowHandle,
    inner: WeakEntity<WelcomeScreen>,
}

impl WelcomeHandle {
    /// Upgrade to a strong entity reference. Returns `None` only if the
    /// window was closed between registration and this call — practically
    /// impossible immediately after `open_window` succeeds.
    pub(crate) fn upgrade(&self) -> Option<Entity<WelcomeScreen>> {
        self.inner.upgrade()
    }
}

/// GPUI global that tracks every open Workspace window by pairing its
/// `AnyWindowHandle` with a `WeakEntity<Workspace>` so broadcast and close
/// operations are typed and do not require `downcast` at the call site. Also
/// tracks the Settings singleton (see [`SettingsHandle`]).
///
/// Implements `Default` so GPUI's `default_global` initialises it
/// automatically on first access — no explicit `init` call is required and
/// tests that create Workspace entities without running `main()` work without
/// any setup.
#[derive(Default)]
pub(crate) struct WindowRegistry {
    workspaces: Vec<(AnyWindowHandle, WeakEntity<Workspace>)>,
    settings: Option<SettingsHandle>,
    welcome: Option<WelcomeHandle>,
}

impl Global for WindowRegistry {}

impl WindowRegistry {
    /// Add a new workspace to the registry. Called from inside
    /// `Workspace::new_with_project` so the entry is always present by the
    /// time the first render fires.
    pub(crate) fn register(
        handle: AnyWindowHandle,
        workspace: WeakEntity<Workspace>,
        cx: &mut App,
    ) {
        cx.default_global::<WindowRegistry>()
            .workspaces
            .push((handle, workspace));
    }

    /// Remove a workspace from the registry. Called from the
    /// `cx.on_release` hook wired in `Workspace::new_with_project`.
    pub(crate) fn deregister(workspace: &WeakEntity<Workspace>, cx: &mut App) {
        if cx.try_global::<WindowRegistry>().is_some() {
            cx.global_mut::<WindowRegistry>()
                .workspaces
                .retain(|(_, w)| w != workspace);
        }
    }

    /// Apply `f` to every live workspace. Entries whose window can no longer
    /// be updated (closed mid-iteration) are pruned lazily from the registry.
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
            // After Phase 3 the window root is `gpui_component::Root`,
            // which wraps a `Workspace`. We enter the window context via
            // `cx.update_window` (root-type-agnostic) and route the
            // closure into the inner `Workspace` entity.
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

    /// All window handles currently in the registry. Used when handles are
    /// needed without modifying the registry.
    #[allow(dead_code)]
    pub(crate) fn all_handles(cx: &App) -> Vec<AnyWindowHandle> {
        cx.try_global::<WindowRegistry>()
            .map(|r| r.workspaces.iter().map(|(h, _)| *h).collect())
            .unwrap_or_default()
    }

    /// Atomically remove and return all handles. Used by
    /// `close_all_workspace_windows` so a double-fire (race between two
    /// callers) returns an empty list on the second call and skips the
    /// close loop without doing any work.
    pub(crate) fn drain_handles(cx: &mut App) -> Vec<AnyWindowHandle> {
        if cx.try_global::<WindowRegistry>().is_none() {
            return Vec::new();
        }
        let registry = cx.global_mut::<WindowRegistry>();
        let handles: Vec<AnyWindowHandle> = registry.workspaces.iter().map(|(h, _)| *h).collect();
        registry.workspaces.clear();
        handles
    }

    /// Return the active window handle if it belongs to a registered
    /// Workspace. Used by `open_project_with_mode` to identify which window
    /// initiated the open action so only that window is closed.
    pub(crate) fn active_workspace_handle(cx: &App) -> Option<AnyWindowHandle> {
        let active = cx.active_window()?;
        cx.try_global::<WindowRegistry>()?
            .workspaces
            .iter()
            .find(|(h, _)| *h == active)
            .map(|(h, _)| *h)
    }

    /// Return `(handle, weak_entity)` for the active workspace. Used by
    /// the global `OpenFolder` / `CloseProject` action handlers to enter
    /// the workspace and mutate it (add a project, close one) without
    /// looping the entire registry.
    pub(crate) fn active_workspace(cx: &App) -> Option<(AnyWindowHandle, WeakEntity<Workspace>)> {
        let active = cx.active_window()?;
        cx.try_global::<WindowRegistry>()?
            .workspaces
            .iter()
            .find(|(h, _)| *h == active)
            .map(|(h, w)| (*h, w.clone()))
    }

    /// Find a workspace window that already hosts a project at `root`.
    ///
    /// Walks every registered Workspace and inspects each project's
    /// `root` path. Returns the first match so the caller can focus the
    /// existing window instead of opening a duplicate. Used by the
    /// "Open Project" flow when the user picks a folder that is already
    /// open somewhere — policy is ignored and the live window wins.
    pub(crate) fn find_workspace_by_root(
        cx: &mut App,
        root: &std::path::Path,
    ) -> Option<AnyWindowHandle> {
        let pairs: Vec<(AnyWindowHandle, WeakEntity<Workspace>)> = cx
            .try_global::<WindowRegistry>()
            .map(|r| r.workspaces.clone())
            .unwrap_or_default();
        let mut found: Option<AnyWindowHandle> = None;
        for (handle, weak) in pairs {
            if found.is_some() {
                break;
            }
            // SILENT-OK: stale window in registry — skip and continue iteration
            let _ = cx.update_window(handle, |_root, _window, cx_w| {
                let Some(ws) = weak.upgrade() else {
                    return;
                };
                let matched = ws.read(cx_w).has_project_root(root);
                if matched {
                    found = Some(handle);
                }
            });
        }
        found
    }

    /// Record the live Settings singleton. Called from
    /// `SettingsWindow::new_with_section` so the entry is in place before the
    /// first render. Replaces any previous entry (the caller path guarantees
    /// at most one Settings window is open at a time).
    pub(crate) fn register_settings(
        window: AnyWindowHandle,
        inner: WeakEntity<SettingsWindow>,
        cx: &mut App,
    ) {
        cx.default_global::<WindowRegistry>().settings = Some(SettingsHandle { window, inner });
    }

    /// Drop the Settings singleton entry. Called from the `cx.on_release`
    /// hook wired in `SettingsWindow::new_with_section`.
    pub(crate) fn clear_settings(cx: &mut App) {
        if cx.try_global::<WindowRegistry>().is_some() {
            cx.global_mut::<WindowRegistry>().settings = None;
        }
    }

    /// Return a clone of the Settings handle if a Settings window is open.
    /// Used by `open_settings_window` to bring an existing window to the
    /// front instead of opening a second one.
    pub(crate) fn settings(cx: &App) -> Option<SettingsHandle> {
        cx.try_global::<WindowRegistry>()?.settings.clone()
    }

    /// Record the live Welcome singleton. Called from `WelcomeScreen::new`
    /// so the entry is in place before the first render. Replaces any
    /// previous entry (at most one Welcome window is open at a time).
    pub(crate) fn register_welcome(
        window: AnyWindowHandle,
        inner: WeakEntity<WelcomeScreen>,
        cx: &mut App,
    ) {
        cx.default_global::<WindowRegistry>().welcome = Some(WelcomeHandle { window, inner });
    }

    /// Drop the Welcome singleton entry. Called from the `cx.on_release`
    /// hook wired in `WelcomeScreen::new`.
    pub(crate) fn clear_welcome(cx: &mut App) {
        if cx.try_global::<WindowRegistry>().is_some() {
            cx.global_mut::<WindowRegistry>().welcome = None;
        }
    }

    /// Return a clone of the Welcome handle if a Welcome window is open.
    /// Used by `open_welcome_window` to retrieve the entity for subscription.
    pub(crate) fn welcome(cx: &App) -> Option<WelcomeHandle> {
        cx.try_global::<WindowRegistry>()?.welcome.clone()
    }

    /// Return the window handle of the open Welcome window, if any.
    /// Used by `active_window_to_close` to detect whether the frontmost
    /// window is the Welcome screen.
    pub(crate) fn welcome_window(cx: &App) -> Option<AnyWindowHandle> {
        cx.try_global::<WindowRegistry>()?
            .welcome
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

    /// Construct a Workspace wrapped in `gpui_component::Root` —
    /// matches the production windowing path so APIs that walk the
    /// window root (`WindowExt::has_active_dialog` etc.) don't panic
    /// during construction.
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
