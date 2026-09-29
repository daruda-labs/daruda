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

use daruda_acp::{
    ConfigChoiceView, ConfigOptionCategoryView, ConfigOptionKindView, ConfigOptionView,
    ConfigValueView,
};
use gpui::{IntoElement, ParentElement as _, SharedString, Styled as _, WeakEntity, px};

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
    // Both kinds collapse to the same shape — an ordered choice list plus which
    // one is current — so the chip below stays kind-agnostic the way it is
    // already category-agnostic.
    let (choices, current) = choices_of(option);
    let display_name = choices
        .iter()
        .find(|(value, _)| *value == current)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| current_fallback_label(&current));

    let tooltip = if option.name.is_empty() {
        display_name.clone()
    } else {
        strings::agent_chat_config_chip(&option.name, &display_name)
    };
    let label = SharedString::from(visible_config_label(&display_name));

    let config_id = option.id.clone();
    let option_name = option.name.clone();

    let chip_id = SharedString::from(format!("agent-chat-config-chip-{pane_id}-{}", option.id));

    // Same chrome as the mode chip (ghost variant: transparent bg, fills on
    // hover; 28px height, radius md) so the chip row reads as one control group.
    button(chip_id, label)
        .child(crate::ui::icons::icon(crate::ui::icons::EXPAND_MORE))
        .ghost()
        .xsmall()
        .h(px(theme::BUTTON_HEIGHT))
        .rounded(px(theme::RADIUS_MD))
        .tooltip(tooltip)
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

fn visible_config_label(display_name: &str) -> String {
    display_name.to_owned()
}

/// The option's choices as `(value, display name)` in menu order, plus the
/// value currently selected.
///
/// A boolean has no protocol-supplied labels — it carries only a `bool` — so
/// the host names its two states from i18n. On-first ordering matches how the
/// adapters' `select` fallback lists them, so the menu reads the same whether
/// the agent sent a native boolean or degraded to a two-value select.
fn choices_of(option: &ConfigOptionView) -> (Vec<(ConfigValueView, String)>, ConfigValueView) {
    match &option.kind {
        ConfigOptionKindView::Select {
            current_value,
            options,
        } => (
            options
                .iter()
                .map(|c| {
                    (
                        ConfigValueView::Id(c.value.clone()),
                        choice_label(option.category, c),
                    )
                })
                .collect(),
            ConfigValueView::Id(current_value.clone()),
        ),
        ConfigOptionKindView::Boolean { current_value } => (
            vec![
                (
                    ConfigValueView::Bool(true),
                    strings::agent_chat_config_boolean_on(),
                ),
                (
                    ConfigValueView::Bool(false),
                    strings::agent_chat_config_boolean_off(),
                ),
            ],
            ConfigValueView::Bool(*current_value),
        ),
    }
}

/// A choice's display name; model choices go through the shared model label
/// so the chip and Settings name the default model alike.
fn choice_label(category: ConfigOptionCategoryView, choice: &ConfigChoiceView) -> String {
    if category == ConfigOptionCategoryView::Model {
        strings::agent_model_choice_label(
            &choice.value,
            &choice.name,
            choice.description.as_deref(),
        )
    } else {
        choice.name.clone()
    }
}

/// Chip label when the current value is absent from the choice list — an
/// adapter transient. A select falls back to the raw id (better than a blank
/// chip); a boolean always matches one of its two synthetic entries, so its
/// arm is unreachable in practice and still yields a sensible label.
fn current_fallback_label(current: &ConfigValueView) -> String {
    match current {
        ConfigValueView::Id(id) => id.clone(),
        ConfigValueView::Bool(true) => strings::agent_chat_config_boolean_on(),
        ConfigValueView::Bool(false) => strings::agent_chat_config_boolean_off(),
    }
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
    choices: &[(ConfigValueView, String)],
    current: &ConfigValueView,
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
        let is_current = value == *current;
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
    use super::*;
    use daruda_acp::DEFAULT_MODEL_CHOICE;

    #[test]
    fn visible_label_is_only_the_current_option_value() {
        assert_eq!(visible_config_label("Sonnet 4"), "Sonnet 4");
    }

    fn choice(value: &str, name: &str, description: Option<&str>) -> ConfigChoiceView {
        ConfigChoiceView {
            value: value.to_string(),
            name: name.to_string(),
            description: description.map(str::to_string),
        }
    }

    #[test]
    fn default_model_choice_names_the_model_it_resolves_to() {
        let default = choice(
            DEFAULT_MODEL_CHOICE,
            "Default (recommended)",
            Some("Opus 5.5"),
        );
        assert_eq!(
            choice_label(ConfigOptionCategoryView::Model, &default),
            "Default (Opus 5.5)"
        );
    }

    #[test]
    fn other_choices_keep_the_adapter_name() {
        let concrete = choice("opus", "Opus 5.5", Some("For complex work"));
        assert_eq!(
            choice_label(ConfigOptionCategoryView::Model, &concrete),
            "Opus 5.5"
        );
        // Effort also has a "default" entry; its description is not a model.
        let effort = choice(DEFAULT_MODEL_CHOICE, "Default", Some("Adaptive"));
        assert_eq!(
            choice_label(ConfigOptionCategoryView::ThoughtLevel, &effort),
            "Default"
        );
        let bare = choice(DEFAULT_MODEL_CHOICE, "Default (recommended)", None);
        assert_eq!(
            choice_label(ConfigOptionCategoryView::Model, &bare),
            "Default (recommended)"
        );
    }
}
