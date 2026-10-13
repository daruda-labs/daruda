//! Public API tests for agent icon.

use daruda_config::agent::icon::*;

#[test]
fn an_adapter_under_two_ids_resolves_to_one_icon() {
    assert_eq!(icon_for_agent("claude"), icon_for_agent("claude-acp"));
    assert_eq!(icon_for_agent("codex"), icon_for_agent("codex-acp"));
}

#[test]
fn an_unknown_agent_has_no_icon() {
    assert_eq!(icon_for_agent("not-a-shipped-agent"), None);
}
