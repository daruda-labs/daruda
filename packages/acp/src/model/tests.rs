/// The select state of `view`, or a panic naming the kind that arrived —
/// every assertion below is about a dropdown option.
fn select_of(view: &ConfigOptionView) -> (&str, &[ConfigChoiceView]) {
    match &view.kind {
        ConfigOptionKindView::Select {
            current_value,
            options,
        } => (current_value, options),
        other => panic!("expected a select kind, got {other:?}"),
    }
}
use agent_client_protocol::schema::v1::{
    AvailableCommand, PlanEntry, PlanEntryPriority, PlanEntryStatus, SessionMode, SessionModeState,
    UnstructuredCommandInput,
};

use super::*;

fn tool(kind: ToolKindView, raw_input: Option<serde_json::Value>) -> ToolCallItem {
    ToolCallItem {
        id: "t1".into(),
        title: "cmd".into(),
        kind,
        tool_name: None,
        status: ToolStatusView::InProgress,
        diffs: vec![],
        output: vec![],
        raw_input,
        locations: Vec::new(),
        parent_tool_id: None,
        exit: None,
    }
}

#[test]
fn subagent_type_reads_task_input_field() {
    use serde_json::json;
    let tc = tool(
        ToolKindView::Other,
        Some(json!({ "subagent_type": "code-reviewer", "description": "review" })),
    );
    assert_eq!(tc.subagent_type(), Some("code-reviewer"));
}

#[test]
fn subagent_type_is_none_without_the_field() {
    use serde_json::json;
    let tc = tool(ToolKindView::Execute, Some(json!({ "command": "ls" })));
    assert_eq!(tc.subagent_type(), None);
}

#[test]
fn subagent_type_ignores_empty_string() {
    use serde_json::json;
    let tc = tool(ToolKindView::Other, Some(json!({ "subagent_type": "" })));
    assert_eq!(tc.subagent_type(), None, "empty type is not a real label");
}

#[test]
fn subagent_prompt_reads_task_input_field() {
    use serde_json::json;
    let tc = tool(
        ToolKindView::Think,
        Some(json!({ "subagent_type": "general-purpose", "prompt": "Implement X." })),
    );
    assert_eq!(tc.subagent_prompt(), Some("Implement X."));
}

#[test]
fn subagent_prompt_is_none_without_the_field() {
    use serde_json::json;
    let tc = tool(ToolKindView::Think, Some(json!({ "description": "x" })));
    assert_eq!(tc.subagent_prompt(), None);
}

#[test]
fn subagent_run_in_background_reads_task_input_field() {
    use serde_json::json;
    let tc = tool(
        ToolKindView::Think,
        Some(json!({ "run_in_background": true })),
    );
    assert_eq!(tc.subagent_run_in_background(), Some(true));
}

#[test]
fn subagent_run_in_background_is_none_without_the_field() {
    let tc = tool(ToolKindView::Think, Some(serde_json::json!({})));
    assert_eq!(tc.subagent_run_in_background(), None);
}

#[test]
fn is_subagent_launch_true_with_either_field_alone() {
    use serde_json::json;
    assert!(
        tool(
            ToolKindView::Think,
            Some(json!({ "subagent_type": "general-purpose" }))
        )
        .is_subagent_launch(),
        "subagent_type alone is enough"
    );
    assert!(
        tool(ToolKindView::Think, Some(json!({ "prompt": "Do it." }))).is_subagent_launch(),
        "prompt alone is enough"
    );
}

#[test]
fn is_subagent_launch_false_without_either_field() {
    let tc = tool(ToolKindView::Think, Some(serde_json::json!({ "x": 1 })));
    assert!(!tc.is_subagent_launch());
}

#[test]
fn is_live_groups_pending_with_in_progress() {
    assert!(ToolStatusView::Pending.is_live());
    assert!(ToolStatusView::InProgress.is_live());
    assert!(!ToolStatusView::Completed.is_live());
    assert!(!ToolStatusView::Failed.is_live());
    assert!(!ToolStatusView::Cancelled.is_live());
}

#[test]
fn is_background_true_only_for_execute_with_flag_set() {
    use serde_json::json;
    assert!(
        tool(
            ToolKindView::Execute,
            Some(json!({"run_in_background": true}))
        )
        .is_background()
    );
    assert!(
        !tool(
            ToolKindView::Execute,
            Some(json!({"run_in_background": false}))
        )
        .is_background()
    );
    // Execute but no flag → not background.
    assert!(!tool(ToolKindView::Execute, Some(json!({"command": "ls"}))).is_background());
    // Flag set but not a shell tool → not background.
    assert!(!tool(ToolKindView::Read, Some(json!({"run_in_background": true}))).is_background());
    // No raw input at all → not background.
    assert!(!tool(ToolKindView::Execute, None).is_background());
}

#[test]
fn plan_entry_view_maps_all_statuses_and_a_priority() {
    let pending = PlanEntry::new("task A", PlanEntryPriority::High, PlanEntryStatus::Pending);
    let view = PlanEntryView::from(&pending);
    assert_eq!(view.content, "task A");
    assert_eq!(view.priority, PlanPriority::High);
    assert_eq!(view.status, PlanStatus::Pending);

    let in_progress = PlanEntry::new(
        "task B",
        PlanEntryPriority::Medium,
        PlanEntryStatus::InProgress,
    );
    let view = PlanEntryView::from(&in_progress);
    assert_eq!(view.content, "task B");
    assert_eq!(view.priority, PlanPriority::Medium);
    assert_eq!(view.status, PlanStatus::InProgress);

    let completed = PlanEntry::new("task C", PlanEntryPriority::Low, PlanEntryStatus::Completed);
    let view = PlanEntryView::from(&completed);
    assert_eq!(view.content, "task C");
    assert_eq!(view.priority, PlanPriority::Low);
    assert_eq!(view.status, PlanStatus::Completed);
}

#[test]
fn slash_command_from_no_input() {
    let cmd = AvailableCommand::new("compact", "Compact conversation history");
    let view = SlashCommand::from(&cmd);
    assert_eq!(view.name, "compact");
    assert_eq!(view.description, "Compact conversation history");
    assert_eq!(view.input, SlashCommandInput::NoInput);
}

#[test]
fn slash_command_from_unstructured_input() {
    use agent_client_protocol::schema::v1::AvailableCommandInput;
    let cmd = AvailableCommand::new("search", "Search the codebase").input(
        AvailableCommandInput::Unstructured(UnstructuredCommandInput::new("search query")),
    );
    let view = SlashCommand::from(&cmd);
    assert_eq!(view.name, "search");
    assert_eq!(view.description, "Search the codebase");
    assert_eq!(
        view.input,
        SlashCommandInput::FreeText {
            hint: "search query".to_string()
        }
    );
}

#[test]
fn slash_command_free_text_preserves_name_description_and_hint() {
    // Verifies that a FreeText command round-trips all three fields
    // (name, description, hint) together — not covered by the NoInput
    // or unstructured-input tests which check fields in isolation.
    use agent_client_protocol::schema::v1::AvailableCommandInput;
    let cmd = AvailableCommand::new("explain", "Explain selected code").input(
        AvailableCommandInput::Unstructured(UnstructuredCommandInput::new("what to explain")),
    );
    let view = SlashCommand::from(&cmd);
    assert_eq!(view.name, "explain");
    assert_eq!(view.description, "Explain selected code");
    assert_eq!(
        view.input,
        SlashCommandInput::FreeText {
            hint: "what to explain".to_string()
        }
    );
}

#[test]
fn mode_state_view_from_session_mode_state_maps_fields() {
    let protocol_state = SessionModeState::new(
        "acceptEdits",
        vec![
            SessionMode::new("default", "Default"),
            SessionMode::new("acceptEdits", "Accept Edits")
                .description("Automatically accept file edits"),
        ],
    );

    let view = ModeStateView::from(&protocol_state);

    assert_eq!(view.current, "acceptEdits");
    assert_eq!(view.available.len(), 2);

    let first = &view.available[0];
    assert_eq!(first.id, "default");
    assert_eq!(first.name, "Default");
    assert_eq!(first.description, None);

    let second = &view.available[1];
    assert_eq!(second.id, "acceptEdits");
    assert_eq!(second.name, "Accept Edits");
    assert_eq!(
        second.description.as_deref(),
        Some("Automatically accept file edits")
    );
}

#[test]
fn mode_state_view_from_empty_available_modes() {
    let protocol_state = SessionModeState::new("default", vec![]);
    let view = ModeStateView::from(&protocol_state);
    assert_eq!(view.current, "default");
    assert!(view.available.is_empty());
}

#[test]
fn into_on_option_ref_matches_from() {
    // Verify the `Into` blanket impl — used in session.rs as
    // `new_session.modes.as_ref().map(Into::into)`.
    let state = SessionModeState::new("plan", vec![SessionMode::new("plan", "Plan")]);
    let via_into: ModeStateView = (&state).into();
    let via_from = ModeStateView::from(&state);
    assert_eq!(via_into, via_from);
}

#[test]
fn config_option_select_maps_id_value_and_choices() {
    use agent_client_protocol::schema::v1::{
        SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelectOption,
    };
    let opt = SessionConfigOption::select(
        "model",
        "Model",
        "default",
        vec![
            SessionConfigSelectOption::new("default", "Default (recommended)"),
            SessionConfigSelectOption::new("sonnet", "Sonnet")
                .description("Efficient for routine tasks"),
        ],
    )
    .category(SessionConfigOptionCategory::Model);

    let view = ConfigOptionView::from_protocol(&opt).expect("select option maps");
    assert_eq!(view.id, "model");
    assert_eq!(view.name, "Model");
    assert_eq!(view.category, ConfigOptionCategoryView::Model);
    let (current, choices) = select_of(&view);
    assert_eq!(current, "default");
    assert_eq!(choices.len(), 2);
    assert_eq!(choices[0].value, "default");
    assert_eq!(choices[1].value, "sonnet");
    assert_eq!(
        choices[1].description.as_deref(),
        Some("Efficient for routine tasks")
    );
}

#[test]
fn config_option_boolean_kind_maps() {
    // Claude's "Fast mode" arrives as a native boolean once the host
    // advertises `session.configOptions.boolean` at `initialize`; without
    // that opt-in the same option degrades to a two-value select.
    use agent_client_protocol::schema::v1::{SessionConfigOption, SessionConfigOptionCategory};
    let opt = SessionConfigOption::boolean("fast", "Fast mode", true)
        .category(SessionConfigOptionCategory::ModelConfig);

    let view = ConfigOptionView::from_protocol(&opt).expect("boolean option maps");
    assert_eq!(view.id, "fast");
    assert_eq!(view.name, "Fast mode");
    assert_eq!(view.category, ConfigOptionCategoryView::ModelConfig);
    assert_eq!(
        view.kind,
        ConfigOptionKindView::Boolean {
            current_value: true
        }
    );
}

#[test]
fn mode_state_ignores_a_boolean_mode_option() {
    // A mode is a choice among named modes, so a boolean carrying the Mode
    // category is malformed — it must yield no mode state rather than a
    // bogus two-entry selector.
    let boolean_mode = ConfigOptionView {
        id: "mode".to_string(),
        name: "Mode".to_string(),
        description: None,
        category: ConfigOptionCategoryView::Mode,
        kind: ConfigOptionKindView::Boolean {
            current_value: true,
        },
    };
    assert!(ModeStateView::from_config_options(&[boolean_mode]).is_none());
}

#[test]
fn config_option_thought_level_category_maps() {
    use agent_client_protocol::schema::v1::{
        SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelectOption,
    };
    let opt = SessionConfigOption::select(
        "effort",
        "Effort",
        "high",
        vec![SessionConfigSelectOption::new("high", "High")],
    )
    .category(SessionConfigOptionCategory::ThoughtLevel);
    let view = ConfigOptionView::from_protocol(&opt).expect("maps");
    assert_eq!(view.category, ConfigOptionCategoryView::ThoughtLevel);
}

#[test]
fn config_option_without_category_is_other() {
    use agent_client_protocol::schema::v1::{SessionConfigOption, SessionConfigSelectOption};
    let opt = SessionConfigOption::select(
        "agent",
        "Agent",
        "default",
        vec![SessionConfigSelectOption::new("default", "Default")],
    );
    let view = ConfigOptionView::from_protocol(&opt).expect("maps");
    assert_eq!(view.category, ConfigOptionCategoryView::Other);
}

/// Build the option views the adapter's `config_option_update` carries:
/// a `Mode`-category select plus an unrelated `Model` one.
fn mode_and_model_options(
    current_mode: &'static str,
    mode_ids: &[&'static str],
) -> Vec<ConfigOptionView> {
    use agent_client_protocol::schema::v1::{
        SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelectOption,
    };
    let mode = SessionConfigOption::select(
        "mode",
        "Mode",
        current_mode,
        mode_ids
            .iter()
            .map(|id| SessionConfigSelectOption::new(*id, *id).description("desc"))
            .collect::<Vec<_>>(),
    )
    .category(SessionConfigOptionCategory::Mode);
    let model = SessionConfigOption::select(
        "model",
        "Model",
        "sonnet",
        vec![SessionConfigSelectOption::new("sonnet", "Sonnet")],
    )
    .category(SessionConfigOptionCategory::Model);
    [mode, model]
        .iter()
        .filter_map(ConfigOptionView::from_protocol)
        .collect()
}

#[test]
fn mode_state_view_from_config_options_maps_the_mode_option() {
    let options = mode_and_model_options("auto", &["auto", "default", "plan"]);

    let view = ModeStateView::from_config_options(&options).expect("mode option maps");

    assert_eq!(view.current, "auto");
    assert_eq!(
        view.available
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["auto", "default", "plan"],
        "choices map to advertised modes in order"
    );
    assert_eq!(view.available[0].name, "auto");
    assert_eq!(view.available[0].description.as_deref(), Some("desc"));
}

#[test]
fn mode_state_view_from_config_options_reflects_a_shrunken_mode_list() {
    // The adapter drops `auto` after a switch to a model without classifier
    // support and clamps the current mode — the host must follow both.
    let before = mode_and_model_options("auto", &["auto", "default", "plan"]);
    let after = mode_and_model_options("default", &["default", "plan"]);

    let before = ModeStateView::from_config_options(&before).expect("maps");
    let after = ModeStateView::from_config_options(&after).expect("maps");

    assert_eq!(before.available.len(), 3);
    assert_eq!(after.available.len(), 2);
    assert!(after.available.iter().all(|m| m.id != "auto"));
    assert_eq!(after.current, "default");
}

#[test]
fn mode_state_view_from_config_options_without_a_mode_option_is_none() {
    use agent_client_protocol::schema::v1::{
        SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelectOption,
    };
    let model = SessionConfigOption::select(
        "model",
        "Model",
        "sonnet",
        vec![SessionConfigSelectOption::new("sonnet", "Sonnet")],
    )
    .category(SessionConfigOptionCategory::Model);
    let options: Vec<ConfigOptionView> = [model]
        .iter()
        .filter_map(ConfigOptionView::from_protocol)
        .collect();

    assert_eq!(ModeStateView::from_config_options(&options), None);
    assert_eq!(ModeStateView::from_config_options(&[]), None);
}

#[test]
fn mode_state_view_from_config_options_with_no_choices_is_none() {
    // An empty selector can't be rendered or cycled; the caller keeps the
    // mode state it already has rather than blanking it.
    let options = mode_and_model_options("default", &[]);
    assert_eq!(ModeStateView::from_config_options(&options), None);
}

#[test]
fn only_the_default_choice_resolves_to_its_described_model() {
    assert_eq!(
        resolved_default_model(DEFAULT_MODEL_CHOICE, Some("Opus 5.5")),
        Some("Opus 5.5")
    );
    assert_eq!(resolved_default_model(DEFAULT_MODEL_CHOICE, Some("")), None);
    assert_eq!(resolved_default_model(DEFAULT_MODEL_CHOICE, None), None);
    assert_eq!(
        resolved_default_model("opus", Some("For complex work")),
        None
    );
}

/// A mid-turn fill that prices nothing keeps the last reported cost, and a
/// priced report replaces it.
#[test]
fn a_report_without_cost_carries_the_previous_one() {
    let priced = UsageView {
        used: 10,
        size: 100,
        cost: Some(CostView {
            amount: 1.5,
            currency: "USD".to_owned(),
        }),
        rate_limit: None,
    };
    let unpriced = UsageView {
        used: 20,
        size: 100,
        cost: None,
        rate_limit: None,
    };
    let carried = unpriced.clone().carrying_cost_from(Some(&priced));
    assert_eq!(carried.used, 20);
    assert_eq!(carried.cost, priced.cost);
    assert_eq!(unpriced.clone().carrying_cost_from(None).cost, None);

    let repriced = UsageView {
        cost: Some(CostView {
            amount: 2.0,
            currency: "USD".to_owned(),
        }),
        ..unpriced
    };
    assert_eq!(
        repriced.clone().carrying_cost_from(Some(&priced)).cost,
        repriced.cost
    );
}

fn rate_limit(is_using_overage: bool) -> RateLimitView {
    RateLimitView {
        window: Some(RateLimitWindow::FiveHour),
        resets_at: Some(1_791_340_200),
        is_using_overage,
    }
}

#[test]
fn only_the_overage_flag_bills_extra_usage() {
    assert!(rate_limit(true).bills_overage());
    assert!(!rate_limit(false).bills_overage());
}

#[test]
fn rate_limit_window_keeps_an_unknown_value() {
    assert_eq!(
        RateLimitWindow::from_wire("five_hour"),
        RateLimitWindow::FiveHour
    );
    assert_eq!(
        RateLimitWindow::from_wire("seven_day"),
        RateLimitWindow::SevenDay
    );
    assert_eq!(
        RateLimitWindow::from_wire("seven_day_opus"),
        RateLimitWindow::Other("seven_day_opus".to_owned())
    );
}

/// A report without a rate limit says nothing about overage, so the previous
/// report's standing must not leak into it the way its cost does.
#[test]
fn a_report_without_a_rate_limit_does_not_carry_the_previous_one() {
    let billed = UsageView {
        used: 10,
        size: 100,
        cost: None,
        rate_limit: Some(rate_limit(true)),
    };
    let silent = UsageView {
        used: 20,
        size: 100,
        cost: None,
        rate_limit: None,
    };
    assert_eq!(silent.carrying_cost_from(Some(&billed)).rate_limit, None);
}

#[test]
fn the_protocol_usage_update_carries_no_rate_limit() {
    use agent_client_protocol::schema::v1::UsageUpdate;
    let update = UsageUpdate::new(5, 50);
    assert_eq!(UsageView::from(&update).rate_limit, None);
}
