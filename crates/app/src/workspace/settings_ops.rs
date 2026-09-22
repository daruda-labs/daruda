//! Opening and closing the Settings view this window hosts.
//!
//! Settings is a body-level surface, not a pane: while it is up it replaces
//! `body_layout` wholesale, so the docks and the tab strip are not on screen
//! and the workspace actions that drive them are not registered (see
//! `render/mod.rs`). The tab and pane entities behind it stay alive untouched,
//! which is why closing needs no record of "where the user came from" — there
//! is only one place to come back to.

use daruda_config::BuiltinSection;
use gpui::{AppContext as _, Context, Entity, Subscription, Window};

use crate::settings::{LoginRequest, SettingsEvent, SettingsView};
use crate::workspace::Workspace;

/// The Settings view a window is showing, with the subscription carrying its
/// close request back. Both are built and dropped together, so they are one
/// value rather than two fields that must agree.
pub(in crate::workspace) struct SettingsHost {
    view: Entity<SettingsView>,
    _close: Subscription,
}

impl Workspace {
    /// Show Settings on `section`, or move the open one to it.
    pub(in crate::workspace) fn open_settings(
        &mut self,
        section: BuiltinSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = match self.settings.as_ref() {
            Some(host) => host.view.clone(),
            None => {
                let view = cx.new(|cx| SettingsView::new_with_section(section, window, cx));
                let close = cx.subscribe_in(
                    &view,
                    window,
                    |this, _, event: &SettingsEvent, window, cx| match event {
                        SettingsEvent::Close => this.close_settings(window, cx),
                        SettingsEvent::Login(request) => {
                            this.run_settings_login(request, window, cx)
                        }
                    },
                );
                self.settings = Some(SettingsHost {
                    view: view.clone(),
                    _close: close,
                });
                view
            }
        };
        // Both paths land focus, and that is the whole contract of the swap:
        // the pane that had it is no longer rendered, and gpui resolves an
        // unrendered focus id to the root dispatch node — which carries
        // neither the view's key handler nor this window's actions. Skipping
        // it on the build path left Escape and Cmd+W inert until the user
        // clicked something.
        view.update(cx, |view, cx| view.focus_section(section, window, cx));
        cx.notify();
    }

    /// Take Settings down and hand focus back to the workspace.
    ///
    /// Refused when a pending edit could not be written: that value and the
    /// banner naming the failure both live in the view, so taking it down
    /// would be the silent drop [`SettingsView::commit_pending_edits`] exists
    /// to prevent.
    pub(in crate::workspace) fn close_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.commit_settings_edits(window, cx) {
            cx.notify();
            return;
        }
        if self.settings.take().is_none() {
            return;
        }
        // Focus has to land on something this window still renders. A
        // workspace holding no projects has no pane to return to, so the root
        // takes it — leaving the window pointed at the view just dropped kills
        // the keyboard exactly the way an unfocused open did.
        let pane_id = self.active_runtime().focused_pane_id;
        if self.active_runtime().panes.iter().any(|p| p.id == pane_id) {
            self.focus_pane(pane_id, window, cx);
        } else {
            self.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    /// Land every pending Settings edit, whatever is taking the view away.
    /// The one funnel: the back button, Escape and the window closing
    /// underneath all pass through here, so no exit can be added that skips
    /// the commit. `false` means a write failed and the view must stay up.
    pub(in crate::workspace) fn commit_settings_edits(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(host) = self.settings.as_ref() else {
            return true;
        };
        let view = host.view.clone();
        view.update(cx, |view, cx| view.commit_pending_edits(window, cx))
    }

    /// Run a login an account row asked for. The view has no business spawning
    /// one — the login command comes from this window's agent catalog, and this
    /// window keeps the process handle and the Cancel that goes with it.
    fn run_settings_login(
        &mut self,
        request: &LoginRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match *request {
            LoginRequest::AddAccount(recipe) => self.add_managed_account(recipe, window, cx),
            LoginRequest::Reauthenticate(account) => self.reauthenticate_account(account, cx),
            LoginRequest::ReauthenticateSystem(recipe) => self.reauthenticate_system(recipe, cx),
        }
    }

    /// Whether Settings is on screen. `render` reads this to pick the body and
    /// to decide which actions this window answers.
    pub(in crate::workspace) fn settings_is_open(&self) -> bool {
        self.settings.is_some()
    }

    /// The view on screen, for the two callers that legitimately need the
    /// entity itself: `render`, to embed it, and the screenshot driver, to
    /// seed a state no settled workspace can reach. Everything else goes
    /// through the operations above — reaching in is how `Workspace` grew 82
    /// fields its collaborators could write.
    pub(in crate::workspace) fn settings_view(&self) -> Option<&Entity<SettingsView>> {
        self.settings.as_ref().map(|host| &host.view)
    }
}
