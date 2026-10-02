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
