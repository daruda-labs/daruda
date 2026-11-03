use crate::surface::strings as s;

/// Derive the bottom-input placeholder string from focused-pane context.
///
/// Pure function — no side effects, unit-testable.
///
/// - `is_agent` — the focused pane is an Agent chat pane.
/// - `mode_name` — the human-readable label of the agent's current mode
///   (`SessionModeView::name`), when the session advertises modes.
/// - `use_modifier_to_send` — `AgentConfig::use_modifier_to_send`; when
///   `true` the submit key is ⌘↵, otherwise plain Enter.
pub(crate) fn bottom_input_placeholder_for_context(
    is_agent: bool,
    mode_name: Option<&str>,
    use_modifier_to_send: bool,
) -> String {
    if !is_agent {
        return s::bottom_dock::input_placeholder();
    }
    match (mode_name, use_modifier_to_send) {
        (Some(name), false) => s::bottom_dock::input_agent_mode_placeholder(name),
        (Some(name), true) => s::bottom_dock::input_agent_mode_modifier_placeholder(name),
        (None, false) => s::bottom_dock::input_agent_placeholder(),
        (None, true) => s::bottom_dock::input_agent_modifier_placeholder(),
    }
}

/// Resolve the shared submit chord with the host platform's modifier names.
pub(crate) fn question_placeholder_modifier() -> String {
    question_placeholder_modifier_for(cfg!(target_os = "macos"))
}

fn question_placeholder_modifier_for(mac: bool) -> String {
    let shortcut = crate::surface::shortcut_display::display_for(
        crate::surface::keybindings::SHORTCUT_COMPOSER_SUBMIT,
        mac,
    );
    rust_i18n::t!(
        "bottom_dock.question_placeholder_modifier",
        shortcut = shortcut
    )
    .into_owned()
}

#[cfg(test)]
mod tests {
    use super::question_placeholder_modifier_for;

    #[test]
    fn question_submit_hint_uses_the_platform_modifier() {
        assert!(question_placeholder_modifier_for(true).contains("⌘⏎"));
        let other = question_placeholder_modifier_for(false);
        assert!(other.contains("Ctrl+Enter"));
        assert!(!other.contains('⌘'));
    }
}
