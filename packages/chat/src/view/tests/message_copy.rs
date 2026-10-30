//! The whole-message copy button: what it puts on the clipboard.
use super::*;
use daruda_acp::ChatItem;
use gpui::{MouseButton, VisualTestContext};

/// Hover the row, click the button the hover reveals, and read the clipboard.
fn copy_through(
    selector: &'static str,
    items: Vec<ChatItem>,
    cx: &mut gpui::TestAppContext,
) -> Option<String> {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _, cx| {
            view.items = items;
            view.rebuild_rows();
            cx.notify();
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let button = visual.debug_bounds(selector).expect("copy button laid out");
    let at = button.center();
    visual.simulate_mouse_move(at, None, Default::default());
    visual.run_until_parked();
    visual.simulate_mouse_down(at, MouseButton::Left, Default::default());
    visual.simulate_mouse_up(at, MouseButton::Left, Default::default());
    visual.run_until_parked();
    visual.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
}

#[gpui::test]
fn a_prompt_copies_verbatim(cx: &mut gpui::TestAppContext) {
    let prompt = "# not a heading\n- not a list";
    let copied = copy_through(
        "agent-chat-user-copy-0",
        vec![ChatItem::UserText(prompt.into())],
        cx,
    );
    assert_eq!(copied.as_deref(), Some(prompt));
}

#[gpui::test]
fn an_answer_copies_its_markdown_source(cx: &mut gpui::TestAppContext) {
    let answer = "Done. See **`src/lib.rs`**:\n\n```rust\nfn main() {}\n```";
    let copied = copy_through(
        "agent-chat-answer-copy-1",
        vec![ChatItem::UserText("q".into()), assistant_text_item(answer)],
        cx,
    );
    assert_eq!(copied.as_deref(), Some(answer));
}
