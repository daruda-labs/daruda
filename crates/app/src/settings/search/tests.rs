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
