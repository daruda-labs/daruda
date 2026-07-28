//! Bottom-dock macro key widget.
//!
//! This stays separate from `gpui_component::Button` because macro keys need
//! external hover styling, right-click handlers, and closure-built tooltips.
//! Use `crate::ui::button*` for modal/footer actions.

use crate::ui::menu::{ContextMenuExt as _, PopupMenu};
use crate::ui::theme;
use gpui::{
    AnyView, App, ClickEvent, Context, ElementId, IntoElement, Pixels, RenderOnce, SharedString,
    Window, div, prelude::*, px,
};

/// Content display mode for a [`MacroKey`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KeyDisplay {
    /// Content-width pill labelled with `label`.
    Text,
    /// Fixed-square key showing `icon` codepoint (falls back to first char of `label`).
    Icon,
}

#[derive(IntoElement)]
pub struct MacroKey {
    id: ElementId,
    label: SharedString,
    icon: Option<SharedString>,
    display: KeyDisplay,
    disabled: bool,
    fixed_width: Option<Pixels>,
    #[allow(clippy::type_complexity)]
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
    #[allow(clippy::type_complexity)]
    context_menu:
        Option<Box<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static>>,
    #[allow(clippy::type_complexity)]
    tooltip_fn: Option<Box<dyn Fn(&mut Window, &mut App) -> AnyView + 'static>>,
}

impl MacroKey {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            display: KeyDisplay::Text,
            disabled: false,
            fixed_width: None,
            on_click: None,
            context_menu: None,
            tooltip_fn: None,
        }
    }

    /// Switch to icon key display. Pair with `.icon(codepoint)`.
    pub fn icon_mode(mut self) -> Self {
        self.display = KeyDisplay::Icon;
        self
    }

    /// Icon codepoint for [`KeyDisplay::Icon`]. Falls back to first char of `label` if empty.
    pub fn icon(mut self, icon: impl Into<SharedString>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Force a fixed pixel width for [`KeyDisplay::Text`] mode (long
    /// labels truncate). Has no effect on [`KeyDisplay::Icon`] mode,
    /// which keeps its `BUTTON_WIDGET_ICON_SIZE` width.
    /// Used by the bottom-dock grid layout for uniform cell sizing.
    pub fn fixed_width(mut self, width: Pixels) -> Self {
        self.fixed_width = Some(width);
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Right-click context menu, same builder shape as every other
    /// `.context_menu(...)` / `.dropdown_menu(...)` call site (see
    /// `crate::ui::menu_builder`).
    pub fn context_menu(
        mut self,
        builder: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        self.context_menu = Some(Box::new(builder));
        self
    }

    /// Tooltip builder passed directly to GPUI `.tooltip(f)`.
    pub fn tooltip(mut self, build: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.tooltip_fn = Some(Box::new(build));
        self
    }
}

impl RenderOnce for MacroKey {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            label,
            icon,
            display,
            disabled,
            fixed_width,
            on_click,
            context_menu,
            tooltip_fn,
        } = self;

        let t = theme::current(cx);
        let disabled_bg = t.disabled_item_bg;
        let disabled_text = t.text_subtle;
        let widget_bg = t.button_widget_bg;
        let widget_bg_hover = t.button_widget_bg_hover;
        let widget_text = t.text_body;

        let base = div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .flex_none()
            .h(px(theme::BUTTON_WIDGET_HEIGHT))
            .rounded(px(theme::BUTTON_WIDGET_RADIUS))
            .text_size(px(theme::BUTTON_WIDGET_FONT_SIZE));

        let styled = if disabled {
            base.bg(disabled_bg).text_color(disabled_text)
        } else {
            base.bg(widget_bg)
                .text_color(widget_text)
                .cursor_pointer()
                .hover(move |d| d.bg(widget_bg_hover))
                .active(|d| d.opacity(0.75))
        };

        let el = match display {
            KeyDisplay::Icon => {
                let content: SharedString = icon
                    .filter(|s| !s.is_empty())
                    .or_else(|| label.chars().next().map(|c| c.to_string().into()))
                    .unwrap_or_default();
                styled.w(px(theme::BUTTON_WIDGET_ICON_SIZE)).child(content)
            }
            KeyDisplay::Text => match fixed_width {
                Some(w) => styled
                    .w(w)
                    .overflow_hidden()
                    .px(px(theme::BUTTON_WIDGET_PAD_X))
                    .child(div().overflow_hidden().whitespace_nowrap().child(label)),
                None => styled.px(px(theme::BUTTON_WIDGET_PAD_X)).child(label),
            },
        };

        let el = match (on_click, disabled) {
            (Some(h), false) => el.on_click(h),
            _ => el,
        };
        // `.context_menu(...)` returns a type that only implements
        // ParentElement/Styled/IntoElement, so it must be the last modifier
        // in the chain — `.tooltip()` runs first, then both branches settle
        // to `AnyElement` so the match arms unify.
        let el = match tooltip_fn {
            Some(f) => el.tooltip(f),
            None => el,
        };
        match (context_menu, disabled) {
            (Some(builder), false) => el.context_menu(builder).into_any_element(),
            _ => el.into_any_element(),
        }
    }
}
