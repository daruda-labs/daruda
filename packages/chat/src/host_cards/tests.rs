use super::*;

fn card(id: u64) -> HostCard {
    HostCard {
        id,
        summary: format!("Request {id}"),
        actions: vec![HostCardAction {
            id: "approve".into(),
            label: "Approve".into(),
        }],
    }
}

#[test]
fn duplicate_pushes_cannot_reopen_a_chosen_or_settled_card() {
    let mut cards = HostCards::default();
    cards.push(card(7));
    assert!(cards.choose(7, "approve"));
    cards.push(card(7));
    assert!(!cards.choose(7, "approve"));
    assert_eq!(cards.entries()[0].status.label(), Some("Approve"));

    cards.settle(7, "Handled on phone".into());
    cards.push(card(7));
    assert_eq!(cards.entries().len(), 1);
    assert!(!cards.choose(7, "approve"));
    assert_eq!(cards.entries()[0].status.label(), Some("Handled on phone"));
}

#[test]
fn settlement_keeps_open_cards_and_only_the_latest_result() {
    let mut cards = HostCards::default();
    for id in 0..100 {
        cards.push(card(id));
        cards.settle(id, "Done".into());
    }
    cards.push(card(100));
    cards.push(card(101));
    assert!(cards.choose(101, "approve"));
    cards.push(card(102));
    cards.settle(102, "Done".into());

    assert_eq!(
        cards
            .entries()
            .iter()
            .map(|entry| entry.card.id)
            .collect::<Vec<_>>(),
        [100, 101, 102]
    );
    assert!(matches!(cards.entries()[0].status, HostCardStatus::Pending));
    assert!(matches!(
        cards.entries()[1].status,
        HostCardStatus::Chosen(_)
    ));
    assert!(matches!(
        cards.entries()[2].status,
        HostCardStatus::Settled(_)
    ));
    cards.settle(999, "Unknown".into());
    assert_eq!(cards.entries().len(), 3);
}
