//! Shared right-dock chrome: collapsible sections, library rows, footer.

use gpui::{AnyElement, App, Div, IntoElement, SharedString, WeakEntity, div, prelude::*, px};

use super::section::DockSection;
use crate::ui::{Sizable as _, disclosure, icons, theme};
use crate::workspace::Workspace;

/// How a section's summary row responds to a click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum SectionFold {
    Open,
    Folded,
    /// Shown open and not toggleable: a search forces it open, or there
    /// is nothing to fold. A click would flip state the user cannot see.
    Fixed,
}

impl SectionFold {
    pub fn toggleable(is_open: bool) -> Self {
        if is_open { Self::Open } else { Self::Folded }
    }

    pub fn is_open(self) -> bool {
        !matches!(self, Self::Folded)
    }
}

/// A section's summary row (chevron, uppercase label, trailing count) over
/// its body. `divided` draws the hairline that separates it from the block
/// above; the first section in a panel leaves it off.
pub(in crate::workspace) struct ScopeSection {
    pub section: DockSection,
    pub label: SharedString,
    pub count: Option<SharedString>,
    pub fold: SectionFold,
    pub divided: bool,
}

impl ScopeSection {
    /// The body is only built by the caller when the section is open.
    pub fn render(
        self,
        body: Option<AnyElement>,
        workspace: &WeakEntity<Workspace>,
        cx: &App,
    ) -> Div {
        let t = theme::current(cx);
        let Self {
            section,
            label,
            count,
            fold,
            divided,
        } = self;
        let is_open = fold.is_open();
        let workspace = workspace.clone();
        let summary = div()
            .id(("dock-section", section as usize))
            .debug_selector(move || format!("dock-section-{section:?}"))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::LANE_LABEL_GAP))
            .py(px(theme::DOCK_SECTION_HEADER_PAD_Y))
            .text_size(px(theme::LANE_SECTION_HEADER_FONT_SIZE))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(t.text_muted)
            .child(
                disclosure(("dock-section-chevron", section as usize), is_open)
                    .size(theme::DOCK_SECTION_CHEVRON_SIZE),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_ellipsis()
                    .child(SharedString::from(label.to_uppercase())),
            )
            .when_some(count, |row, count| {
                row.child(div().font_weight(gpui::FontWeight::NORMAL).child(count))
            })
            .when(fold != SectionFold::Fixed, |row| {
                row.cursor_pointer().on_click(move |_, _window, cx| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(cx, |ws, cx| ws.toggle_right_dock_section(section, cx));
                    }
                })
            });
        div()
            .flex()
            .flex_col()
            .w_full()
            .when(divided, |d| {
                d.border_t_1()
                    .border_color(t.border)
                    .pt(px(theme::DOCK_SECTION_DIVIDER_PAD_T))
            })
            .child(summary)
            .when_some(body.filter(|_| is_open), |d, body| d.child(body))
    }
}

/// Icon beside a name over an optional description, with a trailing slot.
/// Layout only: the caller owns ids, hover and every interaction.
pub(in crate::workspace) fn library_row(
    icon: &'static str,
    name: impl IntoElement,
    description: Option<AnyElement>,
    cx: &App,
) -> Div {
    let t = theme::current(cx);
    div()
        .flex()
        .flex_row()
        .items_start()
        .w_full()
        .min_w_0()
        .gap(px(theme::DOCK_LIBRARY_ROW_GAP))
        .py(px(theme::DOCK_LIBRARY_ROW_PAD_Y))
        .child(
            icons::icon(icon)
                .with_size(px(theme::DOCK_LIBRARY_ICON_SIZE))
                .text_color(t.text_muted)
                .mt(px(theme::DOCK_LIBRARY_ICON_MT)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .min_w_0()
                        .text_ellipsis()
                        .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                        .text_color(t.text_body)
                        .child(name),
                )
                .when_some(description, |d, desc| {
                    d.child(
                        div()
                            .min_w_0()
                            .mt(px(theme::DOCK_LIBRARY_DESC_MT))
                            .text_ellipsis()
                            .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
                            .text_color(t.text_muted)
                            .child(desc),
                    )
                }),
        )
}

/// Actions a row reveals on hover, laid over its trailing edge on the row's
/// hover fill so they mask the text behind them. The row sets `.group(group)`
/// and `.relative()`; `right_inset` keeps clear of any trailing control.
pub(in crate::workspace) fn hover_actions(group: &'static str, right_inset: f32, cx: &App) -> Div {
    div()
        .absolute()
        .right(px(right_inset))
        .top_0()
        .bottom_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GAP_SM))
        .bg(theme::current(cx).skill_row_hover_bg)
        .pl(px(theme::LIST_ROW_PAD_X))
        .invisible()
        .group_hover(group, |s| s.visible())
}

/// Summary strip pinned beneath a tab's scrolling body.
pub(in crate::workspace) fn panel_footer(
    icon: &'static str,
    text: impl Into<SharedString>,
    cx: &App,
) -> AnyElement {
    let t = theme::current(cx);
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(px(theme::LANE_LABEL_GAP))
        .min_h(px(theme::DOCK_PANEL_FOOTER_HEIGHT))
        .px(px(theme::DOCK_PANEL_FOOTER_PAD_X))
        .border_t_1()
        .border_color(t.border)
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(t.text_muted)
        .child(icons::icon(icon).with_size(px(theme::DOCK_PANEL_FOOTER_ICON_SIZE)))
        .child(div().min_w_0().text_ellipsis().child(text.into()))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui::{Context, TestAppContext, VisualTestContext, Window};

    use super::*;

    struct Probe {
        fold: SectionFold,
    }

    impl Render for Probe {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let body = div().debug_selector(|| "section-body".into()).child("row");
            ScopeSection {
                section: DockSection::UsageRecentSessions,
                label: "Recent".into(),
                count: Some("1".into()),
                fold: self.fold,
                divided: true,
            }
            .render(
                Some(body.into_any_element()),
                &WeakEntity::new_invalid(),
                cx,
            )
        }
    }

    fn body_painted(fold: SectionFold, cx: &mut TestAppContext) -> bool {
        crate::test_support::init_gpui_component(cx);
        let window = cx.add_window(|_, _| Probe { fold });
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("dock-section-UsageRecentSessions")
                .is_some()
        );
        vcx.debug_bounds("section-body").is_some()
    }

    #[gpui::test]
    fn folded_section_keeps_its_summary_and_drops_the_body(cx: &mut TestAppContext) {
        assert!(!body_painted(SectionFold::Folded, cx));
    }

    #[gpui::test]
    fn open_section_paints_the_body(cx: &mut TestAppContext) {
        assert!(body_painted(SectionFold::Open, cx));
    }

    #[gpui::test]
    fn fixed_section_paints_the_body(cx: &mut TestAppContext) {
        assert!(body_painted(SectionFold::Fixed, cx));
    }
}
