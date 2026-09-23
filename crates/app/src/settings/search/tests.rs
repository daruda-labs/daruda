use super::*;

fn targets(q: &str) -> Vec<Target> {
    query(q).into_iter().map(|d| d.target).collect()
}

#[test]
fn a_blank_query_finds_nothing() {
    assert!(query("").is_empty());
    assert!(query("   ").is_empty());
}

#[test]
fn a_row_is_found_by_its_label_and_lands_on_its_page() {
    let results = query("scrollback");
    assert!(
        results
            .iter()
            .any(|d| d.target == Target::Text(TextSetting::ScrollbackMaxRows))
    );
    assert!(results.iter().all(|d| d.section == Section::Terminal));
    assert_eq!(counts(&results)[0].0, Section::Terminal);
}

#[test]
fn a_row_is_found_by_its_config_path() {
    assert!(
        targets("long_running_threshold")
            .contains(&Target::Text(TextSetting::NotifyLongRunningThresholdSecs))
    );
}

#[test]
fn a_matched_child_brings_its_parent_switch_first() {
    let found = targets("only while away");
    let child = found
        .iter()
        .position(|t| *t == Target::Bool(BoolSetting::TelegramOnlyWhenAway))
        .expect("child row");
    let parent = found
        .iter()
        .position(|t| *t == Target::Bool(BoolSetting::TelegramEnabled))
        .expect("parent row shown with it");
    assert!(parent < child);
}

#[test]
fn a_retired_page_name_finds_the_page_that_absorbed_it() {
    for (word, section) in [
        ("cursor", Section::Terminal),
        ("clipboard", Section::Terminal),
        ("dock", Section::Workspace),
    ] {
        assert!(
            targets(word).contains(&Target::Page(section)),
            "{word} should find {section:?}"
        );
    }
}

#[test]
fn every_term_must_match() {
    assert!(query("scrollback telegram").is_empty());
}

#[test]
fn hand_drawn_blocks_link_to_their_page() {
    assert!(targets("ssh").contains(&Target::Page(Section::SessionHosts)));
    assert!(targets("slack").contains(&Target::Page(Section::RemoteControl)));
}

#[test]
fn status_bar_items_are_searchable_switches() {
    assert!(targets("ports").contains(&Target::StatusBarItem(daruda_config::StatusBarItem::Ports)));
}

/// Results keep their page's order: on Remote Control the Telegram card
/// comes before the Advanced card, as it does on the page.
#[test]
fn results_follow_the_page_order() {
    let found = targets("away");
    let telegram = found
        .iter()
        .position(|t| *t == Target::Bool(BoolSetting::TelegramOnlyWhenAway))
        .expect("telegram row");
    let advanced = found
        .iter()
        .position(|t| *t == Target::Text(TextSetting::PresenceGraceSecs))
        .expect("presence row");
    assert!(telegram < advanced, "{found:?}");
}

/// Every hand-drawn block is indexed exactly once — an anchor the layout
/// never places would drop its entry from search silently.
#[test]
fn every_hand_drawn_block_is_indexed_once() {
    let all = docs();
    for h in HANDWRITTEN {
        let label = (h.label)();
        let n = all
            .iter()
            .filter(|d| d.label == label && d.section == h.section)
            .count();
        assert_eq!(n, 1, "{label}");
    }
}

/// A hand-drawn block lists where its page shows it: Slack and Discord sit
/// in the integrations card, above the Telegram card.
#[test]
fn hand_drawn_blocks_keep_their_page_position() {
    let found = query("away");
    let slack = found
        .iter()
        .position(|d| d.label == s::remote_slack())
        .expect("slack link");
    let telegram = found
        .iter()
        .position(|d| d.target == Target::Bool(BoolSetting::TelegramEnabled))
        .expect("telegram row");
    assert!(slack < telegram);
}
