//! Reset to default from a row: offered only off the default, and it removes
//! the key rather than pinning today's default.

use super::*;
use crate::settings::layout::Target;

#[gpui::test]
fn reset_is_offered_off_the_default_and_removes_the_key(cx: &mut TestAppContext) {
    let mut config = daruda_config::Config::default();
    config.scrollback.max_rows = 50_000;
    let (wh, win) = build_window_with_config(cx, config);
    let target = Target::Text(TextSetting::ScrollbackMaxRows);
    assert!(win.read_with(cx, |w, _| w.differs_from_default(target)));

    let win_for_reset = win.clone();
    cx.update_window(wh.into(), |_, window, cx| {
        win_for_reset.update(cx, |w, cx| w.reset_to_default(target, window, cx));
    })
    .unwrap();

    let default_rows = daruda_config::Config::default().scrollback.max_rows;
    win.read_with(cx, |w, cx| {
        assert!(!w.differs_from_default(target));
        assert!(w.conflict.is_none());
        assert_eq!(
            w.scrollback_input.read(cx).value().as_ref(),
            default_rows.to_string()
        );
    });
    let text = cx.read(|cx| {
        std::fs::read_to_string(
            crate::settings_store::SettingsStore::global(cx).writer_path_for_testing(),
        )
        .unwrap()
    });
    assert!(
        !text
            .lines()
            .any(|line| line.trim_start().starts_with("max_rows")),
        "{text}"
    );
}

#[gpui::test]
fn a_default_value_offers_no_reset(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.read_with(cx, |w, _| {
        assert!(!w.differs_from_default(Target::Text(TextSetting::ScrollbackMaxRows)));
        assert!(!w.differs_from_default(Target::Bool(BoolSetting::WindowBlur)));
    });
}
