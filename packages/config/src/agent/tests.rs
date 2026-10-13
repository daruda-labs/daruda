use super::*;

#[test]
fn default_agents_is_a_single_claude_acp_reference_under_the_claude_id() {
    assert_eq!(
        default_agents(),
        vec![AgentEntry::preset_with(
            "claude-acp",
            PresetOverrides {
                id: Some("claude".to_string()),
                name: Some("Claude Code".to_string()),
                ..PresetOverrides::default()
            }
        )]
    );
    assert_eq!(
        default_agents()[0].resolve(),
        Some(AgentDefinition::claude_default())
    );
}
