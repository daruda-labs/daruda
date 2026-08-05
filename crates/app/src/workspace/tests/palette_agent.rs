use super::*;

// ---- Command palette integration ----

#[gpui::test]
fn command_palette_toggles_and_resolves_core_actions(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, _cx| {
        assert!(!ws.command_palette.is_open);
        ws.command_palette.open();
        assert!(ws.command_palette.is_open);

        for (query, expected) in [
            ("Toggle Left", "toggle_left_dock"),
            ("New Tab", "new_tab"),
            ("Quit", "quit"),
        ] {
            ws.command_palette.query = query.to_string();
            assert_eq!(
                ws.command_palette.focused_action_id(),
                Some(expected),
                "{query:?} should resolve to {expected}"
            );
        }

        ws.command_palette.close();
        assert!(!ws.command_palette.is_open);
    });
}
