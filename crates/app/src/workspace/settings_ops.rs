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

use crate::settings::{SettingsEvent, SettingsView};
use crate::workspace::Workspace;

/// The Settings view a window is showing, with the subscription carrying its
/// close request back. Both are built and dropped together, so they are one
/// value rather than two fields that must agree.
pub(in crate::workspace) struct SettingsHost {
    pub(in crate::workspace) view: Entity<SettingsView>,
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
        match self.settings.as_ref() {
            Some(host) => {
                let view = host.view.clone();
                view.update(cx, |view, cx| view.focus_section(section, window, cx));
            }
            None => {
                let view = cx.new(|cx| SettingsView::new_with_section(section, window, cx));
                let close = cx.subscribe_in(
                    &view,
                    window,
                    |this, _, event: &SettingsEvent, window, cx| match event {
                        SettingsEvent::Close => this.close_settings(window, cx),
                    },
                );
                self.settings = Some(SettingsHost {
                    view,
                    _close: close,
                });
            }
        }
        cx.notify();
    }

    /// Take Settings down and hand focus back to the pane that had it. The
    /// view has already committed whatever was mid-edit (`dismiss`), so
    /// dropping it here loses nothing.
    pub(in crate::workspace) fn close_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings.take().is_none() {
            return;
        }
        let pane_id = self.active_runtime().focused_pane_id;
        self.focus_pane(pane_id, window, cx);
        cx.notify();
    }
}
