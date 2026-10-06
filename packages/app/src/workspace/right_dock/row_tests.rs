//! Rendered geometry checks for hover-revealed row actions.

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, IntoElement, Render, TestAppContext,
    VisualTestContext, WeakEntity, Window, div, prelude::*, px, size,
};

use crate::ui::theme;
use crate::workspace::Workspace;

type RowRenderer = dyn Fn(WeakEntity<Workspace>, &App) -> AnyElement;

struct RowProbe {
    workspace: Entity<Workspace>,
    render_row: Box<RowRenderer>,
    _state: tempfile::TempDir,
}

impl Render for RowProbe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().flex_col().w(px(220.)).child(
            div()
                .debug_selector(|| "action-row".into())
                .child((self.render_row)(self.workspace.downgrade(), cx)),
        )
    }
}

pub(super) fn assert_hover_targets_fit(
    cx: &mut TestAppContext,
    selectors: &[&'static str],
    render_row: impl Fn(WeakEntity<Workspace>, &App) -> AnyElement + 'static,
) {
    assert_hover_targets_fit_beside(cx, selectors, None, render_row);
}

/// As [`assert_hover_targets_fit`], and every revealed target also ends
/// left of `beside` — a control the overlay must never cover.
pub(super) fn assert_hover_targets_fit_beside(
    cx: &mut TestAppContext,
    selectors: &[&'static str],
    beside: Option<&'static str>,
    render_row: impl Fn(WeakEntity<Workspace>, &App) -> AnyElement + 'static,
) {
    crate::test_support::init_gpui_component(cx);
    let state = tempfile::tempdir().unwrap();
    let window = cx.add_window(|window, cx| RowProbe {
        workspace: cx.new(|cx| {
            Workspace::new_with_project_for_test(
                &daruda_config::Config::default(),
                None,
                state.path().to_path_buf(),
                window,
                cx,
            )
        }),
        render_row: Box::new(render_row),
        _state: state,
    });
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let row = vcx.debug_bounds("action-row").expect("row painted");
    vcx.simulate_mouse_move(row.center(), None, Default::default());
    vcx.run_until_parked();
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let target = px(theme::CONTROL_TARGET_SIZE);
    for selector in selectors {
        let bounds = vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} revealed"));
        assert_eq!(bounds.size, size(target, target), "{selector}");
        assert!(bounds.top() >= row.top(), "{selector} above row");
        assert!(bounds.bottom() <= row.bottom(), "{selector} below row");
        assert!(bounds.left() >= row.left(), "{selector} left of row");
        assert!(bounds.right() <= row.right(), "{selector} right of row");
        if let Some(beside) = beside {
            let kept = vcx
                .debug_bounds(beside)
                .unwrap_or_else(|| panic!("{beside} painted"));
            assert!(bounds.right() <= kept.left(), "{selector} covers {beside}");
        }
    }
}
