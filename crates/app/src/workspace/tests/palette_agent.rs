use super::*;

// ---- Command palette integration ----

#[gpui::test]
fn command_palette_toggles_and_resolves_core_actions(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, _cx| {
        assert!(!ws.command_palette.is_open);
        ws.command_palette.open();
        assert!(ws.command_palette.is_open);

        // Both spellings of every query on purpose: matching is smart-case,
        // so the Title-Case form exercises the case-*sensitive* branch (it
        // only matches because it mirrors the label's own capitalization)
        // and the lowercase form the case-insensitive one users actually
        // type. An all-caps form would match nothing — pinned in
        // `command::palette`'s own tests.
        for (query, expected) in [
            ("Toggle Left", "toggle_left_dock"),
            ("toggle left", "toggle_left_dock"),
            ("New Tab", "new_tab"),
            ("new tab", "new_tab"),
            ("Quit", "quit"),
            ("quit", "quit"),
        ] {
            ws.command_palette.open();
            for ch in query.chars() {
                let visible_len = ws.command_palette.visible().len();
                ws.command_palette
                    .picker
                    .on_key(&ch.to_string(), Some(ch), visible_len);
            }
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
