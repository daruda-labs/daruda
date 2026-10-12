//! Which mode a pane remembers as its own pick.

use super::make_test_view;

/// Only the user's pick is remembered: a mode the adapter reports — at connect
/// or by switching on its own — must not outrank the agent's `default_mode` on
/// the next connect.
#[gpui::test]
fn only_a_user_mode_pick_is_remembered(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{AcpEvent, ModeStateView, SessionCapabilitiesView, SessionModeView};

    let modes = |current: &str| ModeStateView {
        available: ["default", "plan", "bypassPermissions"]
            .into_iter()
            .map(|id| SessionModeView {
                id: id.into(),
                name: id.into(),
                description: None,
            })
            .collect(),
        current: current.into(),
    };
    let window = make_test_view(cx);
    window
        .update(cx, |v, _, cx| {
            v.apply_event(
                AcpEvent::Connected {
                    program: None,
                    session_id: "s1".into(),
                    modes: Some(modes("default")),
                    config_options: Vec::new(),
                    capabilities: SessionCapabilitiesView::default(),
                    login_methods: Vec::new(),
                },
                "",
                false,
                cx,
            );
            v.apply_event(
                AcpEvent::ModeChanged {
                    state: modes("plan"),
                },
                "",
                false,
                cx,
            );
            assert_eq!(
                v.picked_mode_id, None,
                "a mode the adapter reported is not a pick"
            );

            v.set_mode("bypassPermissions".into(), cx);
            v.apply_event(
                AcpEvent::ModeChanged {
                    state: modes("default"),
                },
                "",
                false,
                cx,
            );
            assert_eq!(
                v.picked_mode_id.as_deref(),
                Some("bypassPermissions"),
                "the adapter switching on its own leaves the user's pick standing"
            );
        })
        .unwrap();
}
