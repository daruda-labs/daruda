//! Config-option selector chip (model / effort) for the bottom-dock input.
//!
//! Sibling of [`super::mode_chip`]: where the mode chip renders the session's
//! permission mode (via `ModeStateView`), this renders one ACP *select* config
//! option (model, reasoning effort, …) as a ghost `xsmall` button with a
//! chevron; clicking it opens a dropdown of the option's choices. Selecting a
//! choice dispatches `Workspace::set_agent_config_option` for that pane (one-line
//! dispatch — no state logic here, MVU view purity + one-way data flow).
//!
//! One chip is built per option the caller passes; the caller surfaces every
//! advertised category except `Mode` (see `render/snapshots.rs`) and gates on
//! a focused Agent pane. Category-agnostic by design: a chip renders from
//! `id` / `current_value` / `options` alone, so a new agent advertising an
//! unfamiliar select option (e.g. Codex's `fast-mode` speed toggle) needs no
//! per-agent UI code.

use daruda_acp::ConfigOptionView;
use gpui::{IntoElement, SharedString, Styled as _, WeakEntity, px};

use crate::surface::strings;
use crate::ui::{
    ButtonVariants as _, DropdownMenu as _, PopupMenu, PopupMenuItem, Sizable as _, button, theme,
};
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

/// Build a config-option chip. The label is the current choice's display name
/// (looked up from `option.options`; falls back to the raw `current_value` if
/// not listed) with a chevron appended. Clicking opens a dropdown with one item
/// per choice; each item one-line dispatches into
/// `Workspace::set_agent_config_option(pane_id, config_id, value)`.
pub(in crate::workspace) fn config_chip(
    pane_id: PaneId,
    option: &ConfigOptionView,
    workspace: WeakEntity<Workspace>,
) -> impl IntoElement + use<> {
    let display_name = option
        .options
        .iter()
        .find(|c| c.value == option.current_value)
        .map(|c| c.name.as_str())
        .unwrap_or(option.current_value.as_str())
        .to_string();

    let label = SharedString::from(format!("{}{}", display_name, strings::TASK_PILL_CHEVRON));

    // Owned data for the `'static` dropdown closure.
    let choices: Vec<(String, String)> = option
        .options
        .iter()
        .map(|c| (c.value.clone(), c.name.clone()))
        .collect();
    let current = option.current_value.clone();
    let config_id = option.id.clone();
    let option_name = option.name.clone();

    let chip_id = SharedString::from(format!("agent-chat-config-chip-{pane_id}-{}", option.id));

    // Same chrome as the mode chip (ghost variant: transparent bg, fills on
    // hover; 28px height, radius md) so the chip row reads as one control group.
    button(chip_id, label)
        .ghost()
        .xsmall()
        .h(px(theme::BUTTON_HEIGHT))
        .rounded(px(theme::RADIUS_MD))
        .dropdown_menu(move |menu, _window, _cx| {
            build_config_menu(
                pane_id,
                &config_id,
                &option_name,
                &choices,
                &current,
                &workspace,
                menu,
            )
        })
}

/// Build the choice popup menu. A non-interactive header names the option
/// (the chip label only shows the current *value*, e.g. "Off", so the option's
/// own name — "Fast mode", "Model", … — would otherwise be invisible). One item
/// per choice follows; the active one gets a checkmark. Each item is a one-line
/// dispatch into `Workspace::set_agent_config_option` (render purity; no logic
/// here).
fn build_config_menu(
    pane_id: PaneId,
    config_id: &str,
    option_name: &str,
    choices: &[(String, String)],
    current: &str,
    workspace: &WeakEntity<Workspace>,
    menu: PopupMenu,
) -> PopupMenu {
    // Skip the header if the adapter advertised no name — an empty disabled row
    // would just reserve blank vertical space and the check-icon gutter. A
    // separator sets the title apart from the choices below.
    let menu = if option_name.is_empty() {
        menu
    } else {
        menu.label(SharedString::from(option_name.to_string()))
            .separator()
    };
    choices.iter().fold(menu, |m, (value, name)| {
        let workspace = workspace.clone();
        let config_id = config_id.to_string();
        let value = value.clone();
        let is_current = value == current;
        m.item(
            PopupMenuItem::new(SharedString::from(name.clone()))
                .checked(is_current)
                .on_click(move |_, _window, app| {
                    if let Some(ws) = workspace.upgrade() {
                        let config_id = config_id.clone();
                        let value = value.clone();
                        ws.update(app, |ws, cx| {
                            ws.set_agent_config_option(pane_id, config_id, value, cx)
                        });
                    }
                }),
        )
    })
}

#[cfg(test)]
mod tests {
    // Pure view builder with no extractable logic (mirrors `mode_chip`).
    // Op-level coverage lives in `crate::workspace::tests::agent_chat`.
}
