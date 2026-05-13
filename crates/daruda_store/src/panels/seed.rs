//! First-launch seed — populates `panels.json` with a single "AI" tab
//! containing Claude / Codex / Gemini macros so the user has something
//! to click immediately.

use super::{
    ButtonDisplay, ButtonWidget, PanelTab, PanelsState, SCHEMA_VERSION, TabLayout, Widget,
    new_tab_id, new_widget_id,
};

/// Canonical (label, send) pairs for every built-in seed macro.
/// Referenced by `migrate_builtin_flags` so the send strings are
/// defined exactly once.
pub(crate) const SEED_AI_ENTRIES: &[(&str, &str)] = &[
    ("Claude", r#"claude --dangerously-skip-permissions"#),
    (
        "Codex",
        r#"codex -c model_reasoning_effort="high" --dangerously-bypass-approvals-and-sandbox -c model_reasoning_summary="detailed" -c model_supports_reasoning_summaries=true"#,
    ),
    ("Gemini", "gemini --yolo"),
    ("Opencode", "opencode"),
];

/// Build the default panels state. Called when no `panels.json` exists.
pub fn seed_default() -> PanelsState {
    let tab_id = new_tab_id();
    let widgets = SEED_AI_ENTRIES
        .iter()
        .map(|(label, send)| {
            Widget::Button(ButtonWidget {
                id: new_widget_id(),
                label: (*label).to_string(),
                send: (*send).to_string(),
                auto_enter: true,
                display: ButtonDisplay::Text,
                icon: None,
                shortcut: None,
                style: None,
                builtin: true,
            })
        })
        .collect();
    PanelsState {
        schema_version: SCHEMA_VERSION,
        active_tab_id: Some(tab_id.clone()),
        tabs: vec![PanelTab {
            id: tab_id,
            name: "AI".to_string(),
            order: 0,
            height: None,
            layout: TabLayout::FlexWrap,
            widgets,
        }],
    }
}
