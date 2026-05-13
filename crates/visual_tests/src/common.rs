use std::sync::{Arc, Mutex};

use gpui::{Context, Entity, TestAppContext, VisualTestContext};

use daruda_terminal::view::{TerminalInput, TerminalView};
use daruda_terminal::{TerminalConfig, TerminalSession};

/// Opens a window containing a `TerminalView` with no PTY. Returns the entity
/// and the window-scoped test context. Shadow `cx` with the returned
/// `VisualTestContext` for the rest of the test.
pub fn open_terminal<'a>(
    cx: &'a mut TestAppContext,
) -> (Entity<TerminalView>, &'a mut VisualTestContext) {
    cx.add_window_view(|_window, cx: &mut Context<TerminalView>| {
        let focus = cx.focus_handle();
        let session =
            TerminalSession::new(TerminalConfig::default()).expect("TerminalSession::new failed");
        TerminalView::new(session, focus)
    })
}

/// Opens a window whose `TerminalView` records every byte it would send to the
/// PTY into `sink`. Use `drain` to read and clear the sink between assertions.
pub fn open_terminal_with_sink<'a>(
    cx: &'a mut TestAppContext,
    sink: Arc<Mutex<Vec<u8>>>,
) -> (Entity<TerminalView>, &'a mut VisualTestContext) {
    cx.add_window_view(|_window, cx: &mut Context<TerminalView>| {
        let focus = cx.focus_handle();
        let session =
            TerminalSession::new(TerminalConfig::default()).expect("TerminalSession::new failed");
        let input = TerminalInput::new({
            let sink = sink.clone();
            move |bytes| sink.lock().unwrap().extend_from_slice(bytes)
        });
        TerminalView::new_with_input(session, focus, input)
    })
}

/// Feeds raw VT bytes into the view and flushes pending tasks.
pub fn feed(view: &Entity<TerminalView>, cx: &mut VisualTestContext, bytes: &[u8]) {
    view.update(cx, |tv, cx| {
        tv.feed_output_bytes(bytes, cx);
        cx.notify();
    });
    cx.run_until_parked();
}

/// Moves keyboard focus to the `TerminalView` so that keystroke/input
/// simulation is routed to the view's key handler.
pub fn focus(view: &Entity<TerminalView>, cx: &mut VisualTestContext) {
    let handle = view.update(cx, |tv, _| tv.focus_handle().clone());
    cx.update(|window, cx| window.focus(&handle, cx));
    cx.run_until_parked();
}

/// Queues VT bytes via the same path used by the real PTY reader
/// (`queue_output_bytes` → `reconcile_dirty_viewport_after_output`).
///
/// Use this instead of `feed` when you need the smart dirty-row overlap
/// logic that preserves / clears the selection only where output actually
/// changed.  `feed` uses `feed_output_bytes` which unconditionally calls
/// `refresh_viewport` (clears selection regardless of position).
pub fn queue_output(view: &Entity<TerminalView>, cx: &mut VisualTestContext, bytes: &[u8]) {
    view.update(cx, |tv, cx| {
        tv.queue_output_bytes(bytes, cx);
    });
    cx.run_until_parked();
}

/// Takes all bytes accumulated in `sink` since the last call and clears it.
pub fn drain(sink: &Arc<Mutex<Vec<u8>>>) -> Vec<u8> {
    std::mem::take(&mut sink.lock().unwrap())
}
