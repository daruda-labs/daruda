use super::*;
use crate::view::TerminalInput;
use crate::{TerminalConfig, TerminalDims};
use std::cell::Cell;
use std::rc::Rc;

#[gpui::test]
fn only_forwarded_interrupt_keys_notify_the_host(cx: &mut gpui::TestAppContext) {
    let calls = Rc::new(Cell::new(0));
    let observed = calls.clone();
    let session =
        crate::TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    let handle = cx.add_window(|_, cx| {
        TerminalView::new_with_input(session, cx.focus_handle(), TerminalInput::new(|_| {}))
            .with_interrupt_handler(move |_| observed.set(observed.get() + 1))
    });
    handle
        .update(cx, |view, window, cx| {
            let escape = KeyDownEvent {
                keystroke: gpui::Keystroke::parse("escape").unwrap(),
                is_held: false,
                prefer_character_input: false,
            };
            view.state.marked_text = Some("composing".into());
            view.on_key_down(&escape, window, cx);
            assert_eq!(calls.get(), 0, "IME cancellation never reached the PTY");
            view.paste_text("\u{1b}", cx);
            assert_eq!(calls.get(), 0, "paste is not an interrupt key");
            view.on_key_down(&escape, window, cx);
            assert_eq!(calls.get(), 1);
            let control_c = KeyDownEvent {
                keystroke: gpui::Keystroke::parse("ctrl-c").unwrap(),
                is_held: false,
                prefer_character_input: false,
            };
            view.on_key_down(&control_c, window, cx);
            assert_eq!(calls.get(), 2);
            view.send_key_bytes(&[&[crate::ansi::ESC], b"x"], cx);
            assert_eq!(calls.get(), 2, "Alt input is not a plain Escape");
        })
        .unwrap();
}
