use crate::view::tests::make_test_view;

/// The activity-bar title is derived from `session_title` + the first user
/// prompt, neither of which changes per frame — yet resolving it in `render`
/// made it the paint path's top cost on a pane whose first message was long.
/// It belongs with `rows` / `live_units`: recomputed once per model change, at
/// the `rebuild_rows` funnel every `items` / title mutation already ends in.
#[gpui::test]
fn activity_title_is_derived_once_per_model_change(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, _cx| {
            assert_eq!(view.activity_title(), None, "empty session has no title");

            view.items.push(daruda_acp::ChatItem::UserText(
                "  fix the   parser  ".into(),
            ));
            view.rebuild_rows();
            assert_eq!(
                view.activity_title(),
                Some("fix the parser"),
                "the first prompt stands in until the agent names the session"
            );

            // An agent-supplied title supersedes the fallback at the same funnel.
            view.session_title = Some("Refactor fold state".into());
            view.rebuild_rows();
            assert_eq!(view.activity_title(), Some("Refactor fold state"));

            // Clearing the conversation clears the derived title with it.
            view.items.clear();
            view.session_title = None;
            view.rebuild_rows();
            assert_eq!(view.activity_title(), None);
        })
        .expect("the test window is live");
}
