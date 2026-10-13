use super::*;

#[test]
fn command_palette_name_maps_to_scenario() {
    assert_eq!(
        ScreenshotScenario::from_cli_name("command-palette"),
        Some(ScreenshotScenario::CommandPalette)
    );
}

#[test]
fn error_modal_name_maps_to_scenario() {
    assert_eq!(
        ScreenshotScenario::from_cli_name("error-modal"),
        Some(ScreenshotScenario::ErrorModal)
    );
}

#[test]
fn client_chrome_name_maps_to_scenario() {
    assert_eq!(
        ScreenshotScenario::from_cli_name("client-chrome"),
        Some(ScreenshotScenario::ClientChrome)
    );
}

#[test]
fn landing_name_maps_to_scenario() {
    assert_eq!(
        ScreenshotScenario::from_cli_name("landing"),
        Some(ScreenshotScenario::Landing)
    );
}

#[test]
fn unknown_name_maps_to_none() {
    assert_eq!(ScreenshotScenario::from_cli_name("nope"), None);
    assert_eq!(ScreenshotScenario::from_cli_name(""), None);
}

#[test]
fn bare_settings_maps_to_default_section() {
    assert_eq!(
        ScreenshotScenario::from_cli_name("settings"),
        Some(ScreenshotScenario::Settings(BuiltinSection::default()))
    );
}

#[test]
fn settings_slug_maps_to_that_section() {
    assert_eq!(
        ScreenshotScenario::from_cli_name("settings:font"),
        Some(ScreenshotScenario::Settings(BuiltinSection::Font))
    );
    assert_eq!(
        ScreenshotScenario::from_cli_name("settings:notifications"),
        Some(ScreenshotScenario::Settings(BuiltinSection::Notifications))
    );
}

#[test]
fn the_settings_error_token_is_its_own_scenario() {
    assert_eq!(
        ScreenshotScenario::from_cli_name("settings-error"),
        Some(ScreenshotScenario::SettingsError),
        "the hyphenated token must not be read as a `settings:<slug>` section"
    );
    assert_eq!(
        ScreenshotScenario::from_cli_name("settings-search"),
        Some(ScreenshotScenario::SettingsSearch)
    );
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-catalog-expanded"),
        Some(ScreenshotScenario::AgentCatalogExpanded)
    );
}

#[test]
fn unknown_settings_slug_maps_to_none() {
    assert_eq!(ScreenshotScenario::from_cli_name("settings:bogus"), None);
}

#[test]
fn mermaid_lightbox_name_maps_to_scenario() {
    assert_eq!(
        ScreenshotScenario::from_cli_name("mermaid-lightbox"),
        Some(ScreenshotScenario::MermaidLightbox)
    );
}

#[test]
fn agent_chat_names_map_to_their_scenarios() {
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-chat-empty"),
        Some(ScreenshotScenario::AgentChatEmpty)
    );
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-chat"),
        Some(ScreenshotScenario::AgentChat)
    );
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-chat-working"),
        Some(ScreenshotScenario::AgentChatWorking)
    );
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-chat-queue-parked"),
        Some(ScreenshotScenario::AgentChatQueueParked)
    );
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-chat-queue-armed"),
        Some(ScreenshotScenario::AgentChatQueueArmed)
    );
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-chat-narrowed"),
        Some(ScreenshotScenario::AgentChatNarrowed)
    );
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-chat-fold"),
        Some(ScreenshotScenario::AgentChatFold)
    );
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-chat-tail"),
        Some(ScreenshotScenario::AgentChatTail)
    );
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-chat-tail-open"),
        Some(ScreenshotScenario::AgentChatTailOpen)
    );
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-chat-group-tail"),
        Some(ScreenshotScenario::AgentChatGroupTail)
    );
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-chat-group-tail-open"),
        Some(ScreenshotScenario::AgentChatGroupTailOpen)
    );
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-chat-subagent-tail"),
        Some(ScreenshotScenario::AgentChatSubagentTail)
    );
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-chat-subagent-tail-open"),
        Some(ScreenshotScenario::AgentChatSubagentTailOpen)
    );
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-chat-failure"),
        Some(ScreenshotScenario::AgentChatFailure)
    );
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-chat-transport-closed"),
        Some(ScreenshotScenario::AgentChatTransportClosed)
    );
}

#[test]
fn every_thoughts_shot_is_addressable() {
    for (name, shot) in [
        ("agent-chat-thoughts", ThoughtsShot::Settled),
        ("agent-chat-thoughts-working", ThoughtsShot::Working),
        ("agent-chat-thoughts-only", ThoughtsShot::ThoughtsOnly),
    ] {
        assert_eq!(
            ScreenshotScenario::from_cli_name(name),
            Some(ScreenshotScenario::AgentChatThoughts(shot))
        );
    }
}

/// Every tab the panel offers has to be reachable from the CLI, or a tab
/// silently loses its permanent capture coverage.
#[test]
fn every_options_tab_is_addressable_by_its_own_token() {
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-chat-options"),
        Some(ScreenshotScenario::AgentChatOptions(
            ActivityOptionsTab::Fold
        ))
    );
    for tab in ActivityOptionsTab::ALL {
        assert_eq!(
            ScreenshotScenario::from_cli_name(&format!("agent-chat-options:{}", tab.token())),
            Some(ScreenshotScenario::AgentChatOptions(tab)),
            "{tab:?}"
        );
    }
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-chat-options:nope"),
        None
    );
}

/// Each Settings editor is reachable open, or its panel loses capture
/// coverage — a closed field shows only the trigger.
#[test]
fn every_catalog_editor_is_addressable_by_its_own_token() {
    use daruda_settings::EditorShot;
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-catalog-editor"),
        Some(ScreenshotScenario::AgentCatalogEditor(EditorShot::Fold))
    );
    for axis in EditorShot::ALL {
        assert_eq!(
            ScreenshotScenario::from_cli_name(&format!("agent-catalog-editor:{}", axis.token())),
            Some(ScreenshotScenario::AgentCatalogEditor(axis)),
            "{axis:?}"
        );
    }
    assert_eq!(
        ScreenshotScenario::from_cli_name("agent-catalog-editor:nope"),
        None
    );
}

#[test]
fn lane_switcher_name_maps_to_scenario() {
    assert_eq!(
        ScreenshotScenario::from_cli_name("lane-switcher"),
        Some(ScreenshotScenario::LaneSwitcher)
    );
}

#[test]
fn tab_indicators_name_maps_to_scenario() {
    assert_eq!(
        ScreenshotScenario::from_cli_name("tab-indicators"),
        Some(ScreenshotScenario::TabIndicators)
    );
}

#[test]
fn toast_name_maps_to_scenario() {
    assert_eq!(
        ScreenshotScenario::from_cli_name("toast"),
        Some(ScreenshotScenario::Toast)
    );
}
