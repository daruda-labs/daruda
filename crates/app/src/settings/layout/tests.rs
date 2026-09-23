use super::*;
use crate::settings::spec;

fn placements() -> Vec<Target> {
    placed()
        .into_iter()
        .filter_map(|(_, _, p)| match p {
            Placed::Setting(t) => Some(t),
            _ => None,
        })
        .collect()
}

/// Every spec row, and every status-bar item, is placed on exactly one page.
#[test]
fn every_setting_is_placed_exactly_once() {
    let placed = placements();
    let mut expected: Vec<Target> = spec::TEXT_SETTINGS
        .iter()
        .map(|r| Target::Text(r.setting))
        .chain(
            spec::SELECT_SETTINGS
                .iter()
                .map(|r| Target::Select(r.setting)),
        )
        .chain(spec::BOOL_SETTINGS.iter().map(|r| Target::Bool(r.setting)))
        .chain(Item::ALL.iter().map(|i| Target::StatusBarItem(*i)))
        .collect();
    for target in &expected {
        let n = placed.iter().filter(|p| *p == target).count();
        assert_eq!(n, 1, "{target:?} is placed {n} times");
    }
    expected.sort_by_key(|t| format!("{t:?}"));
    let mut got = placed.clone();
    got.sort_by_key(|t| format!("{t:?}"));
    assert_eq!(
        got, expected,
        "the layout places something no table defines"
    );
}

/// A switch that heads indented rows is itself placed just before them.
#[test]
fn a_parent_switch_precedes_its_rows() {
    for section in Section::ALL {
        for card in page(*section).unwrap_or(&[]) {
            let rows = card.rows();
            for (i, row) in rows.iter().enumerate() {
                if let Row::Under(parent, _) = row {
                    assert!(
                        i > 0
                            && matches!(rows[i - 1], Row::Setting(Target::Bool(p)) if p == *parent),
                        "{parent:?} must sit right before its rows"
                    );
                }
            }
        }
    }
}
