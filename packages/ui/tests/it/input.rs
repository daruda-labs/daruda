use daruda_ui::input::InputState;
use daruda_ui::test_support::init_gpui_component;
use gpui::TestAppContext;

/// A single-line input must drop newlines on `set_value`. A stored
/// newline routes into the single-line layout path, where gpui's
/// `shape_line` panics ("text argument should not contain newlines").
/// Regression: opening the file-viewer editor search (Cmd+F) over a
/// multi-line selection pre-filled the single-line search input with
/// the selection, newlines and all.
#[gpui::test]
fn single_line_set_value_strips_newlines(cx: &mut TestAppContext) {
    init_gpui_component(cx);
    let window = cx.add_window(InputState::new);
    window
        .update(cx, |state, window, cx| {
            state.set_value("first\nsecond\nthird", window, cx);
            assert_eq!(state.value().as_ref(), "firstsecondthird");
            assert!(!state.value().contains('\n'));
        })
        .unwrap();
}

/// Raising the auto-grow cap shows the rows the old cap hid at once — the
/// expand toggle has no edit to wait for — and lowering it hides them again.
#[gpui::test]
fn set_auto_grow_reclamps_the_shown_rows(cx: &mut TestAppContext) {
    init_gpui_component(cx);
    let window = cx.add_window(|window, cx| InputState::new(window, cx).auto_grow(1, 2));
    window
        .update(cx, |state, window, cx| {
            state.set_value("1\n2\n3\n4", window, cx);
            assert_eq!(state.display_rows(), 2);
            state.set_auto_grow(1, 5);
            assert_eq!(state.display_rows(), 4);
            state.set_auto_grow(1, 3);
            assert_eq!(state.display_rows(), 3);
        })
        .unwrap();
}

/// A multi-line input keeps newlines — the strip is single-line only.
#[gpui::test]
fn multi_line_set_value_keeps_newlines(cx: &mut TestAppContext) {
    init_gpui_component(cx);
    let window = cx.add_window(|window, cx| InputState::new(window, cx).multi_line(true));
    window
        .update(cx, |state, window, cx| {
            state.set_value("first\nsecond", window, cx);
            assert_eq!(state.value().as_ref(), "first\nsecond");
        })
        .unwrap();
}

/// Reproduction: composing a Korean syllable via the macOS IME when the
/// composition sits at a non-zero document offset (here, on the second line
/// after a newline). macOS drives this through `EntityInputHandler`:
/// `set_marked_text` → `replace_and_mark_text_in_range(replacementRange,
/// text, selectedRange)` where `selectedRange` is RELATIVE to the marked
/// text (per NSTextInputClient), then `insert_text` → `replace_text_in_range`
/// on commit. We drive the exact same calls to check the syllable survives.
#[gpui::test]
fn korean_ime_compose_after_newline_preserves_syllable(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler as _;
    init_gpui_component(cx);
    let window = cx.add_window(|window, cx| InputState::new(window, cx).multi_line(true));
    window
        .update(cx, |state, window, cx| {
            // Line 1 = "가", then a newline; compose on line 2 so the marked
            // text is at a non-zero offset.
            state.set_value("가\n", window, cx);
            state.move_cursor_to_end(cx);

            // Compose 한 = ㅎ → 하 → 한. selectedRange {1,0} (cursor after the
            // single-unit marked text), replacementRange = None (continuing
            // composition relies on the app's markedRange).
            state.replace_and_mark_text_in_range(None, "ㅎ", Some(1..1), window, cx);
            state.replace_and_mark_text_in_range(None, "하", Some(1..1), window, cx);
            state.replace_and_mark_text_in_range(None, "한", Some(1..1), window, cx);

            // While marking, the text is "가\n한" and the marked range is the
            // trailing 한 (UTF-16 offsets 2..3).
            assert_eq!(state.value().as_ref(), "가\n한", "marked value");
            assert_eq!(
                state.marked_text_range(window, cx),
                Some(2..3),
                "marked range (utf-16)"
            );
            // The cursor / selection must stay within the document.
            let utf16_len = state.value().encode_utf16().count();
            let sel = state
                .selected_text_range(false, window, cx)
                .expect("selection")
                .range;
            assert!(
                sel.start <= utf16_len && sel.end <= utf16_len,
                "selection {sel:?} out of bounds (utf-16 len {utf16_len})"
            );

            // Commit (insertText with NSNotFound replacementRange → None).
            state.replace_text_in_range(None, "한", window, cx);
            assert_eq!(state.value().as_ref(), "가\n한", "committed value");
        })
        .unwrap();
}
