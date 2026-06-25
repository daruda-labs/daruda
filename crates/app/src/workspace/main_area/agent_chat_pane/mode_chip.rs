//! Mode-selector chip for the Agent chat fold toolbar.
//!
//! Renders the current session mode as a ghost `xsmall` button with a
//! chevron; clicking it opens a dropdown listing every advertised mode.
//! Selecting a mode dispatches `Workspace::set_agent_mode` (one-line
//! dispatch — no state logic in this builder, MVU view purity).
//!
//! Only rendered when `modes.available` is non-empty (the caller gates).

use daruda_acp::ModeStateView;
use gpui::{IntoElement, SharedString, prelude::*};

use crate::surface::strings;
use crate::ui::{
    ButtonVariants as _, DropdownMenu as _, PopupMenu, PopupMenuItem, Sizable as _, button,
};
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

/// Build the mode chip element. The caller is responsible for only
/// calling this when `modes.available` is non-empty.
///
/// The chip is a ghost `xsmall` button whose label is the current mode's
/// display name (looked up from `available`; falls back to `current` id if
/// the id is not listed) with a chevron appended. Clicking it opens a
/// dropdown with one item per available mode; each item one-line dispatches
/// into `Workspace::set_agent_mode`.
pub(in crate::workspace) fn mode_chip(
    pane_id: PaneId,
    modes: &ModeStateView,
    cx: &mut Context<Workspace>,
) -> impl IntoElement + use<> {
    // Look up the current mode's display name; fall back to the id itself.
    let display_name = modes
        .available
        .iter()
        .find(|v| v.id == modes.current)
        .map(|v| v.name.as_str())
        .unwrap_or(modes.current.as_str())
        .to_string();

    let label = SharedString::from(format!("{}{}", display_name, strings::TASK_PILL_CHEVRON));

    // Clone the available modes and current id for the dropdown closure.
    // The closure is `'static`, so we capture owned data only.
    let available: Vec<(String, String)> = modes
        .available
        .iter()
        .map(|v| (v.id.clone(), v.name.clone()))
        .collect();
    let ws = cx.weak_entity();

    let chip_id = SharedString::from(format!("agent-chat-mode-chip-{pane_id}"));

    button(chip_id, label)
        .ghost()
        .xsmall()
        .dropdown_menu(move |menu, _window, _cx| build_mode_menu(&available, pane_id, &ws, menu))
}

/// Build the mode selection popup menu. One item per available mode; the
/// currently-active mode gets a checkmark. Each item is a one-line dispatch
/// into `set_agent_mode` (render purity; no logic here).
fn build_mode_menu(
    available: &[(String, String)],
    pane_id: PaneId,
    workspace: &gpui::WeakEntity<Workspace>,
    menu: PopupMenu,
) -> PopupMenu {
    available.iter().fold(menu, |m, (id, name)| {
        let ws = workspace.clone();
        let mode_id = id.clone();
        m.item(
            PopupMenuItem::new(SharedString::from(name.clone())).on_click(
                move |_, _window, app| {
                    if let Some(w) = ws.upgrade() {
                        let mode_id = mode_id.clone();
                        w.update(app, |this, cx| this.set_agent_mode(pane_id, mode_id, cx));
                    }
                },
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    // The chip builder itself has no pure-logic tests (it is a pure
    // view builder with no extractable logic). Op-level coverage lives in
    // `crate::workspace::tests::agent_chat`.
}
