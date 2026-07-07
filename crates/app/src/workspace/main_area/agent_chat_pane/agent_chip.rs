//! Agent-selector chip for the bottom-dock terminal input.
//!
//! Renders the focused Agent chat pane's current agent as a ghost `xsmall`
//! button with a chevron; clicking it opens a dropdown listing every configured
//! agent. Selecting a *different* agent dispatches `Workspace::request_switch_agent`
//! for that pane (one-line dispatch — no state logic in this builder, MVU view
//! purity + one-way data flow: the dock view dispatches through `Workspace`,
//! which owns the mutation and the confirm dialog). Selecting the current agent
//! is a no-op.
//!
//! Only rendered when the config catalog has >= 2 agents (the caller gates); a
//! single-agent setup keeps the prior UX with no chip.

use gpui::{IntoElement, SharedString, Styled as _, WeakEntity, px};

use crate::surface::strings;
use crate::ui::{
    ButtonVariants as _, DropdownMenu as _, PopupMenu, PopupMenuItem, Sizable as _, button, theme,
};
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

/// Build the agent chip element. The caller is responsible for only calling
/// this when the catalog has >= 2 agents.
///
/// The chip is a ghost `xsmall` button whose label is the current agent's
/// display name (looked up in `catalog`; falls back to the id itself when the
/// id is not listed) with a chevron appended. Clicking it opens a dropdown with
/// one item per catalog entry; selecting a *different* agent one-line dispatches
/// into `Workspace::request_switch_agent(agent_id)` (which confirms, then opens
/// a new pane).
pub(in crate::workspace) fn agent_chip(
    pane_id: PaneId,
    current_agent_id: &str,
    catalog: &[(String, String)],
    workspace: WeakEntity<Workspace>,
) -> impl IntoElement + use<> {
    // Look up the current agent's display name; fall back to the id itself.
    let display_name = catalog
        .iter()
        .find(|(id, _)| id == current_agent_id)
        .map(|(_, name)| name.as_str())
        .unwrap_or(current_agent_id)
        .to_string();

    let label = SharedString::from(format!("{}{}", display_name, strings::TASK_PILL_CHEVRON));

    // The closure is `'static`, so capture owned data only.
    let catalog: Vec<(String, String)> = catalog.to_vec();
    let current = current_agent_id.to_string();

    let chip_id = SharedString::from(format!("agent-chat-agent-chip-{pane_id}"));

    // Ghost variant matches the sibling mode / model / effort chips so all read
    // as one inline control group; height and radius pin the shared spec.
    button(chip_id, label)
        .ghost()
        .xsmall()
        .h(px(theme::BUTTON_HEIGHT))
        .rounded(px(theme::RADIUS_MD))
        .dropdown_menu(move |menu, _window, _cx| {
            build_agent_menu(&catalog, &current, &workspace, menu)
        })
}

/// Build the agent selection popup menu. One item per catalog entry; the
/// currently-active agent gets a checkmark. Each item is a one-line dispatch
/// into `Workspace::request_switch_agent` — except the current agent, which is a
/// no-op (re-selecting it must not spawn a new conversation). The switch op
/// opens a brand-new pane, so no pane id is threaded here.
fn build_agent_menu(
    catalog: &[(String, String)],
    current: &str,
    workspace: &WeakEntity<Workspace>,
    menu: PopupMenu,
) -> PopupMenu {
    catalog.iter().fold(menu, |m, (id, name)| {
        let workspace = workspace.clone();
        let agent_id = id.clone();
        let is_current = id == current;
        m.item(
            PopupMenuItem::new(SharedString::from(name.clone()))
                .checked(is_current)
                .on_click(move |_, window, app| {
                    // Re-selecting the current agent is a no-op (no new chat).
                    if is_current {
                        return;
                    }
                    if let Some(ws) = workspace.upgrade() {
                        let agent_id = agent_id.clone();
                        ws.update(app, |ws, cx| ws.request_switch_agent(agent_id, window, cx));
                    }
                }),
        )
    })
}

#[cfg(test)]
mod tests {
    // The chip builder is a pure view builder with no extractable logic, so it
    // has no unit tests of its own. The agent-selection behavior it drives is
    // covered by the `resolve_open_agent_id` / `resolve_restored_agent` unit
    // tests in the sibling `agent_chat_ops` module.
}
