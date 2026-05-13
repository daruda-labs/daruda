//! Label — re-export of `gpui_component::Label`.
//!
//! `Label::new(text)` renders text using `cx.theme().foreground`
//! (gpui_component Theme), which `apply_daruda_palette` retones
//! against the live `DarudaTheme` Global on every light-mode switch.
//! Prefer `crate::ui::Label::new(text)` over a bare `div().child(text)
//! .text_color(theme::current(cx).foo)` whenever the surrounding
//! widget's natural foreground colour is what you want — the Label
//! wrapper inherits theme switching for free.

pub use gpui_component::label::Label;
