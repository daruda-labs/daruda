//! `tab_bar` / `tab` factories with `Small + underline` baked in.
//!
//! `tab(label)` adds padding because upstream underline tabs zero their inner
//! padding. Font-size independence relies on the vendor patch documented in
//! `crates/app/src/ui/CLAUDE.md`; action chips stay outside this wrapper except
//! the bottom-dock suffix.

use gpui::{
    ElementId, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, px,
};
use gpui_component::{Sizable as _, Size};

pub use gpui_component::tab::{Tab, TabBar};

/// Underline-style `Small` TabBar with `text_xs` cascaded to children.
pub fn tab_bar(id: impl Into<ElementId>) -> TabBar {
    TabBar::new(id)
        .with_size(Size::Small)
        .underline()
        .text_xs()
        .h(px(super::theme::TAB_BAR_HEIGHT))
}

/// Tab paired with [`tab_bar`]; padding widens the underline/click target.
pub fn tab(label: impl Into<SharedString>) -> Tab {
    Tab::new()
        .label(label)
        .px_2p5()
        .min_h(px(super::theme::TAB_BAR_HEIGHT))
}

/// Compact dock navigation; artwork stays independent of the vendor tab tier.
pub fn dock_tab(icon: impl Into<super::Icon>, label: impl Into<SharedString>) -> Tab {
    Tab::new()
        .w(px(super::theme::DOCK_TAB_WIDTH))
        .min_h(px(super::theme::TAB_BAR_HEIGHT))
        .child(icon.into().with_size(px(super::theme::CONTROL_ICON_SIZE)))
        .tooltip(super::tooltip::text(label))
}

/// Shared strip metrics for both docks, without changing content-area tabs.
pub fn dock_tab_bar(id: impl Into<ElementId>) -> TabBar {
    tab_bar(id).w_full().gap(px(0.))
}

#[cfg(test)]
mod tests {
    use super::super::theme;

    #[test]
    fn dock_tabs_keep_artwork_inside_a_full_control_target() {
        const {
            assert!(theme::DOCK_TAB_WIDTH >= theme::CONTROL_TARGET_SIZE);
            assert!(theme::TAB_BAR_HEIGHT >= theme::CONTROL_TARGET_SIZE);
            assert!(theme::CONTROL_TARGET_SIZE > theme::CONTROL_ICON_SIZE);
        }
    }
}
