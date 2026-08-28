//! Status bar's right-click toggle menu — one checkable entry per
//! [`StatusBarItem`], letting the user hide/show individual segments.
//! Toggling dispatches to `Workspace::toggle_status_bar_item`
//! (`status_bar_ops.rs`), which owns the persistence chain.

use crate::ui::{PopupMenu, PopupMenuItem};
use crate::workspace::Workspace;
use daruda_config::{StatusBarConfig, StatusBarItem};
use gpui::{Context, SharedString, WeakEntity, Window};

/// Build the right-click menu closure for the status bar's outer container:
/// a checkbox per [`StatusBarItem`], in `StatusBarItem::ALL` order, checked
/// against `visible`. Sizing is not applied here — `root_context_menu` owns
/// it for every menu that goes through it.
pub(super) fn build(
    visible: StatusBarConfig,
    workspace: WeakEntity<Workspace>,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
    move |menu, _window, _cx| {
        StatusBarItem::ALL.iter().fold(menu, |menu, &item| {
            let workspace = workspace.clone();
            menu.item(
                PopupMenuItem::new(toggle_label(item))
                    .checked(visible.is_visible(item))
                    .on_click(move |_, _window, app| {
                        if let Some(ws) = workspace.upgrade() {
                            ws.update(app, |ws, cx| ws.toggle_status_bar_item(item, cx));
                        }
                    }),
            )
        })
    }
}

fn toggle_label(item: StatusBarItem) -> SharedString {
    SharedString::from(match item {
        StatusBarItem::ProjectBranch => crate::surface::strings::status_bar_toggle_project_branch(),
        StatusBarItem::AccountSlot => crate::surface::strings::status_bar_toggle_account_slot(),
        StatusBarItem::Ports => crate::surface::strings::status_bar_toggle_ports(),
        StatusBarItem::Flow => crate::surface::strings::status_bar_toggle_flow(),
        StatusBarItem::ClaudeUsage => crate::surface::strings::status_bar_toggle_claude_usage(),
    })
}
