use super::*;

#[test]
fn seed_default_sends_canonical_commands() {
    use super::seed::SEED_AI_ENTRIES;
    let state = seed_default();
    let sends: Vec<&str> = state.tabs[0]
        .widgets
        .iter()
        .filter_map(|w| match w {
            MacroKey::Button(b) => Some(b.send.as_str()),
            _ => None,
        })
        .collect();
    let expected: Vec<&str> = SEED_AI_ENTRIES.iter().map(|(_, s)| *s).collect();
    assert_eq!(sends, expected);
}
