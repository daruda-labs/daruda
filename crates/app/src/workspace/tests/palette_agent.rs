use super::*;

// ---- Command palette integration ----

#[gpui::test]
fn test_command_palette_toggle(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, _cx| {
        assert!(!ws.command_palette.is_open);
        ws.command_palette.open();
        assert!(ws.command_palette.is_open);
        ws.command_palette.close();
        assert!(!ws.command_palette.is_open);
    });
}

#[gpui::test]
fn test_palette_resolves_toggle_left_dock(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, _cx| {
        ws.command_palette.open();
        ws.command_palette.query = "Toggle Left".to_string();
        let id = ws.command_palette.focused_action_id();
        assert_eq!(id, Some("toggle_left_dock"));
    });
}

#[gpui::test]
fn test_palette_resolves_new_tab(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, _cx| {
        ws.command_palette.open();
        ws.command_palette.query = "New Tab".to_string();
        let id = ws.command_palette.focused_action_id();
        assert_eq!(id, Some("new_tab"));
    });
}

#[gpui::test]
fn test_palette_resolves_quit(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, _cx| {
        ws.command_palette.open();
        ws.command_palette.query = "Quit".to_string();
        let id = ws.command_palette.focused_action_id();
        assert_eq!(id, Some("quit"));
    });
}
