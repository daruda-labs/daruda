//! Public API tests for agent transcript default.

use daruda_config::AgentEntry;
use daruda_config::agent::transcript_default::*;
use daruda_config::agent::{AgentDefinition, PresetOverrides};

fn tokens(values: &[&str]) -> Option<Vec<String>> {
    Some(values.iter().map(|v| v.to_string()).collect())
}

fn claude_reference() -> AgentEntry {
    AgentEntry::preset_with(
        "claude-acp",
        PresetOverrides {
            id: Some("claude".into()),
            ..PresetOverrides::default()
        },
    )
}

#[test]
fn a_preset_reference_takes_the_value_as_an_override() {
    let next = with_transcript_default(
        &[claude_reference()],
        "claude",
        TranscriptDefault::FoldMode(tokens(&["summary", "last.diff=expanded"])),
    )
    .expect("claude resolves");
    let definition = next[0].resolve().unwrap();
    assert_eq!(
        definition.fold_mode,
        tokens(&["summary", "last.diff=expanded"])
    );
    assert_eq!(next[0].preset_id(), Some("claude-acp"), "still a reference");
}

#[test]
fn a_custom_entry_takes_the_value_on_its_definition() {
    let custom = AgentEntry::custom(AgentDefinition::claude_default());
    let next = with_transcript_default(
        &[custom],
        "claude",
        TranscriptDefault::TailWindows {
            steps: Some(5),
            calls: None,
        },
    )
    .unwrap();
    let definition = next[0].resolve().unwrap();
    assert_eq!(definition.tail_window, Some(5));
    assert_eq!(definition.tail_window_calls, None);
}

#[test]
fn writing_none_drops_the_key_and_leaves_the_other_axes() {
    let mut entry = claude_reference();
    entry.set_transcript_default(TranscriptDefault::DisplayFilter(tokens(&["prose"])));
    entry.set_transcript_default(TranscriptDefault::TailWindows {
        steps: None,
        calls: Some(3),
    });
    entry.set_transcript_default(TranscriptDefault::DisplayFilter(None));
    let definition = entry.resolve().unwrap();
    assert_eq!(definition.display_filter, None);
    assert_eq!(definition.tail_window_calls, Some(3), "untouched axis kept");
}

#[test]
fn only_the_switched_on_entry_for_the_id_is_written() {
    let off = claude_reference().disabled();
    let on = claude_reference();
    let codex = AgentEntry::preset("codex-acp");
    let next = with_transcript_default(
        &[off.clone(), on, codex.clone()],
        "claude",
        TranscriptDefault::DisplayFilter(tokens(&["tools"])),
    )
    .unwrap();
    assert_eq!(next[0], off, "the disabled entry never started the pane");
    assert_eq!(
        next[1].resolve().unwrap().display_filter,
        tokens(&["tools"])
    );
    assert_eq!(next[2], codex);
}

#[test]
fn an_id_no_entry_launches_writes_nothing() {
    assert_eq!(
        with_transcript_default(
            &[claude_reference()],
            "gone",
            TranscriptDefault::FoldMode(None)
        ),
        None
    );
}
