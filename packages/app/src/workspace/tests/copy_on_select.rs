//! Copy-on-select through a real terminal view: a settled selection reaches
//! the clipboard only when `[clipboard] copy_on_select` is on.

use super::*;

fn settle_selection(cx: &mut TestAppContext, copy_on_select: bool) -> Option<String> {
    let mut config = daruda_config::Config::default();
    config.clipboard.copy_on_select = copy_on_select;
    let (_wh, ws) = build_workspace_with(cx, &config, None);
    cx.run_until_parked();
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("before".into()));
    ws.update(cx, |ws, cx| {
        let view = ws
            .active_runtime()
            .panes
            .iter()
            .find_map(|p| p.terminal_view().cloned())
            .expect("a terminal pane");
        view.update(cx, |view, cx| {
            view.feed_output_bytes(b"hello", cx);
            view.settle_viewport_selection_for_test(cx);
        });
    });
    cx.read_from_clipboard().and_then(|item| item.text())
}

#[gpui::test]
async fn a_settled_selection_is_copied_when_asked(cx: &mut TestAppContext) {
    let copied = settle_selection(cx, true);
    assert!(
        copied.as_deref().is_some_and(|t| t.contains("hello")),
        "the selection reaches the clipboard: {copied:?}"
    );
}

#[gpui::test]
async fn a_settled_selection_is_left_alone_by_default(cx: &mut TestAppContext) {
    assert_eq!(settle_selection(cx, false).as_deref(), Some("before"));
}
