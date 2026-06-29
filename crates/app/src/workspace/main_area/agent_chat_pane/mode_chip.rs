//! Mode-selector chip for the bottom-dock terminal input.
//!
//! Renders the focused Agent chat pane's current session mode as a ghost
//! `xsmall` button with a chevron; clicking it opens a dropdown listing every
//! advertised mode. Selecting a mode dispatches `Workspace::set_agent_mode`
//! for that pane (one-line dispatch — no state logic in this builder, MVU view
//! purity + one-way data flow: the dock view dispatches through `Workspace`,
//! which owns the mutation).
//!
//! Only rendered when the focused pane is an Agent chat pane whose
//! `modes.available` is non-empty (the caller gates).

use daruda_acp::ModeStateView;
use gpui::{IntoElement, SharedString, WeakEntity};

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
/// into `Workspace::set_agent_mode(pane_id, mode_id)`.
pub(in crate::workspace) fn mode_chip(
    pane_id: PaneId,
    modes: &ModeStateView,
    workspace: WeakEntity<Workspace>,
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
    let current = modes.current.clone();

    let chip_id = SharedString::from(format!("agent-chat-mode-chip-{pane_id}"));

    button(chip_id, label)
        .ghost()
        .xsmall()
        .dropdown_menu(move |menu, _window, _cx| {
            build_mode_menu(pane_id, &available, &current, &workspace, menu)
        })
}

/// Build the mode selection popup menu. One item per available mode; the
/// currently-active mode gets a checkmark. Each item is a one-line dispatch
/// into `Workspace::set_agent_mode` (render purity; no logic here).
fn build_mode_menu(
    pane_id: PaneId,
    available: &[(String, String)],
    current: &str,
    workspace: &WeakEntity<Workspace>,
    menu: PopupMenu,
) -> PopupMenu {
    available.iter().fold(menu, |m, (id, name)| {
        let workspace = workspace.clone();
        let mode_id = id.clone();
        let is_current = id == current;
        m.item(
            PopupMenuItem::new(SharedString::from(name.clone()))
                .checked(is_current)
                .on_click(move |_, _window, app| {
                    if let Some(ws) = workspace.upgrade() {
                        let mode_id = mode_id.clone();
                        ws.update(app, |ws, cx| ws.set_agent_mode(pane_id, mode_id, cx));
                    }
                }),
        )
    })
}

#[cfg(test)]
mod tests {
    // The chip builder itself has no pure-logic tests (it is a pure
    // view builder with no extractable logic). Op-level coverage lives in
    // `crate::workspace::tests::agent_chat`.
}
