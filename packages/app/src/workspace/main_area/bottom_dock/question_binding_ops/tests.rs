use super::*;

#[test]
fn forgetting_a_pane_drops_only_its_marks() {
    let mut marks = QuestionMarks::default();
    marks.bound.insert(
        1,
        BoundField {
            question_id: 7,
            field: 0,
            send: false,
        },
    );
    marks.bound.insert(
        2,
        BoundField {
            question_id: 8,
            field: 1,
            send: false,
        },
    );
    marks.blocked_prompt = Some(BlockedPrompt {
        pane: 1,
        question_id: 7,
        text: "later".into(),
    });
    marks.esc_released = Some(2);
    marks.text_refused = Some((
        1,
        BoundField {
            question_id: 7,
            field: 0,
            send: false,
        },
    ));

    marks.forget_pane(1);

    assert!(!marks.bound.contains_key(&1));
    assert!(marks.bound.contains_key(&2));
    assert!(marks.blocked_prompt.is_none());
    assert_eq!(marks.esc_released, Some(2));
    assert!(marks.text_refused.is_none());
}
