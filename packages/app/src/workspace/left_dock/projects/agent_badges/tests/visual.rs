use super::*;
use gpui::{AppContext as _, HeadlessAppContext, Render};
use std::sync::Arc;

const PREVIEW_WIDTH: f32 = 160.0;
const PREVIEW_HEIGHT: f32 = 64.0;

fn init_dot_theme(cx: &mut gpui::App) {
    use gpui::BorrowAppContext as _;
    theme::DarudaTheme::init(cx);
    crate::settings_store::SettingsStore::init(cx);
    cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
        let mut config = daruda_config::Config::default();
        config.left_dock.status_icon_theme = daruda_config::StatusIconTheme::Dots;
        store.set_user_for_testing(config);
    });
}

struct Indicators {
    outcomes: [Option<AgentOutcome>; 2],
}

impl Render for Indicators {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .bg(theme::current(cx).dock_bg)
            .flex()
            .items_center()
            .justify_center()
            .gap(px(theme::GAP_LG))
            .children(self.outcomes.into_iter().map(|outcome| {
                div()
                    .w(px(theme::STATUS_INDICATOR_CELL_WIDTH))
                    .h(px(theme::STATUS_INDICATOR_SIZE))
                    .child(agent_status_cell(
                        Some(SessionStatus::Idle),
                        outcome,
                        Default::default(),
                        theme::STATUS_INDICATOR_CELL_WIDTH,
                        cx,
                    ))
            }))
    }
}

#[test]
#[ignore = "requires a Metal renderer"]
fn capture_idle_and_completed_grids() {
    let mut app = HeadlessAppContext::with_platform(
        Arc::new(gpui::NoopTextSystem::new()),
        Arc::new(()),
        || {
            Some(Box::new(
                gpui_macos::metal_renderer::MetalHeadlessRenderer::new(),
            ))
        },
    );
    app.update(init_dot_theme);
    let window = app
        .open_window(
            gpui::size(px(PREVIEW_WIDTH), px(PREVIEW_HEIGHT)),
            |_, cx| {
                cx.new(|_| Indicators {
                    outcomes: [None, Some(AgentOutcome::Completed)],
                })
            },
        )
        .unwrap();
    app.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
    let before = app.capture_screenshot(window.into()).unwrap();
    let green_pixels = |pixels: &[u8]| {
        pixels
            .chunks_exact(4)
            .filter(|pixel| {
                pixel[1] > pixel[0].saturating_add(20) && pixel[1] > pixel[2].saturating_add(10)
            })
            .count()
    };
    assert!(green_pixels(before.as_raw()) > 0);
    let view = window.root(&mut app).unwrap();
    view.update(&mut app, |view, cx| {
        view.outcomes[1] = None;
        cx.notify();
    });
    app.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
    let after = app.capture_screenshot(window.into()).unwrap();
    assert_eq!(green_pixels(after.as_raw()), 0);
    let output = tempfile::tempdir().unwrap().keep();
    before.save(output.join("idle-completed.png")).unwrap();
    after.save(output.join("acknowledged.png")).unwrap();
    println!("indicator captures: {}", output.display());
}

struct WorkingIndicators;

impl Render for WorkingIndicators {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .bg(theme::current(cx).dock_bg)
            .flex()
            .items_center()
            .justify_center()
            .gap(px(theme::GAP_LG))
            .children(
                [IndicatorSize::Leading, IndicatorSize::Badge]
                    .into_iter()
                    .flat_map(|size| {
                        [false, true].map(|active| {
                            let badge =
                                AgentStatusBadge::for_status(SessionStatus::Working, size, cx);
                            if active { badge.active() } else { badge }
                        })
                    }),
            )
    }
}

#[test]
#[ignore = "requires a Metal renderer"]
fn capture_working_perimeter_cycle() {
    let mut app = HeadlessAppContext::with_platform(
        Arc::new(gpui::NoopTextSystem::new()),
        Arc::new(()),
        || {
            Some(Box::new(
                gpui_macos::metal_renderer::MetalHeadlessRenderer::new(),
            ))
        },
    );
    app.update(init_dot_theme);
    let window = app
        .open_window(
            gpui::size(px(PREVIEW_WIDTH), px(PREVIEW_HEIGHT)),
            |_, cx| cx.new(|_| WorkingIndicators),
        )
        .unwrap();
    let output = tempfile::tempdir().unwrap().keep();
    let view = window.root(&mut app).unwrap();
    let mut frames = Vec::new();
    for tick in 0..=8 {
        app.update(|cx| cx.set_global(crate::ui::StatusPulseClock { tick }));
        view.update(&mut app, |_, cx| cx.notify());
        app.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        let frame = app.capture_screenshot(window.into()).unwrap();
        frame
            .save(output.join(format!("working-{tick}.png")))
            .unwrap();
        frames.push(frame);
    }
    for pair in frames.windows(2) {
        assert_ne!(pair[0].as_raw(), pair[1].as_raw());
    }
    assert_eq!(frames[0].as_raw(), frames[8].as_raw());
    println!(
        "working indicator captures (16px, active 16px, 12px, active 12px): {}",
        output.display()
    );
}

struct StatusThemes;

impl Render for StatusThemes {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let samples = [
            (Some(SessionStatus::Connecting), None),
            (Some(SessionStatus::Working), None),
            (Some(SessionStatus::ExecutingTool), None),
            (Some(SessionStatus::NeedsAttention), None),
            (Some(SessionStatus::Idle), None),
            (Some(SessionStatus::Idle), Some(AgentOutcome::Completed)),
            (Some(SessionStatus::Idle), Some(AgentOutcome::Errored)),
            (Some(SessionStatus::Failed), None),
        ];
        div()
            .size_full()
            .bg(theme::current(cx).dock_bg)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(theme::GAP_LG))
            .children(
                [IndicatorSize::Leading, IndicatorSize::Badge]
                    .into_iter()
                    .map(|size| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(theme::GAP_LG))
                            .child(AgentStatusBadge::disconnected(size, cx))
                            .children(samples.into_iter().map(|(state, outcome)| {
                                resolved_badge(state, outcome, size, cx).unwrap()
                            }))
                    }),
            )
    }
}

#[test]
#[ignore = "requires a Metal renderer"]
fn capture_status_icon_themes_and_live_switch() {
    use crate::settings_store::SettingsStore;
    let mut app = HeadlessAppContext::with_platform(
        Arc::new(gpui::NoopTextSystem::new()),
        Arc::new(crate::assets::DarudaAssets),
        || {
            Some(Box::new(
                gpui_macos::metal_renderer::MetalHeadlessRenderer::new(),
            ))
        },
    );
    app.update(|cx| {
        theme::init_if_missing(cx);
        SettingsStore::init(cx);
        let sources = vec![(
            "user".into(),
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/status-icon-themes"),
        )];
        cx.set_global(crate::status_icon_themes::StatusIconThemes(
            crate::status_icon_themes::load(&sources),
        ));
    });
    let window = app
        .open_window(
            gpui::size(px(PREVIEW_WIDTH * 2.0), px(PREVIEW_HEIGHT * 2.0)),
            |_, cx| cx.new(|_| StatusThemes),
        )
        .unwrap();
    let view = window.root(&mut app).unwrap();
    let output = tempfile::tempdir().unwrap().keep();
    for appearance in ["daruda_dark", "daruda_light"] {
        app.update(|cx| assert!(theme::apply_ui_theme(appearance, cx)));
        let mut captures = Vec::new();
        for (name, choice) in [
            ("dots", daruda_config::StatusIconTheme::Dots),
            ("icons", daruda_config::StatusIconTheme::Icons),
            (
                "custom",
                daruda_config::StatusIconTheme::Custom("user/soft-ice".into()),
            ),
            ("restored", daruda_config::StatusIconTheme::Dots),
        ] {
            app.update(|cx| {
                use gpui::BorrowAppContext as _;
                cx.update_global::<SettingsStore, _>(|store, _| {
                    store
                        .apply_patch(daruda_config::SettingsPatch::StatusIconTheme(choice))
                        .unwrap();
                });
                cx.set_global(crate::ui::StatusPulseClock { tick: 0 });
            });
            view.update(&mut app, |_, cx| cx.notify());
            app.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
                .unwrap();
            let frame = app.capture_screenshot(window.into()).unwrap();
            frame
                .save(output.join(format!("{appearance}-{name}.png")))
                .unwrap();
            captures.push(frame);
        }
        assert_ne!(captures[0].as_raw(), captures[1].as_raw());
        assert_ne!(captures[1].as_raw(), captures[2].as_raw());
        assert_eq!(captures[0].as_raw(), captures[3].as_raw());
    }
    println!(
        "status themes (16px above, 12px below): {}",
        output.display()
    );
}
