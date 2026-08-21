//! The buttons over the graph.
//!
//! Their own file because the toolbar carries its own vocabulary — two icon
//! paths, two test selectors, and the rule that decides what each button says.

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, div, px,
};

use super::super::{FlowGraphEvent, FlowGraphView};
use crate::surface::strings as s;
use crate::ui::theme::palette;

/// The two act-on-the-flow glyphs. `IconName` — the vendored set — has no play
/// arrow, so both come from daruda's own Material Symbols icons and are named by
/// path, the way the file viewer's toolbar names its modes.
const ICON_PLAY: &str = "icons/ui/play-arrow.svg";
const ICON_CHECK: &str = "icons/ui/check.svg";

/// How the tests find ▶ and ✓ to press them. Named here so a test cannot drift
/// from its button by spelling the selector a second time.
pub(in crate::workspace) const TOOLBAR_RUN_SELECTOR: &str = "flow-toolbar-run-press";
pub(in crate::workspace) const TOOLBAR_CHECK_SELECTOR: &str = "flow-toolbar-check-press";

/// Buttons over the graph: what a person does to the flow, then what they do
/// with it.
///
/// The menu (`pane_menu::FlowGraphMenu`) has the editing pair and calls the same
/// ops — that half is a second way in, not a second implementation. It exists
/// because the menu is a right-click nobody is told about, and adding the first
/// node to a new flow is the moment that matters most.
///
/// Running and checking are here and not in the menu because until now the only
/// way to run the flow on screen was the palette, which asks which flow — a
/// question this pane already has the answer to.
pub(super) fn toolbar(
    has_selection: bool,
    unsaved_form: bool,
    cx: &mut Context<FlowGraphView>,
) -> impl IntoElement {
    use crate::ui::{Disableable as _, Icon, button_bare};

    div()
        .absolute()
        // Each interaction it has to swallow, named — rather than `occlude()`,
        // which blocks every one at once. The toolbar sits inside the canvas's
        // own bounds, so something has to stop the press or it starts a marquee
        // drag underneath; `occlude()` would, but `BlockMouse` also makes
        // everything behind it read as un-hovered, and the canvas takes that as
        // the pointer having left. React Flow draws the same line per
        // interaction (`nodrag` / `nopan` / `nowheel`) for the same reason.
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(gpui::MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .top(px(palette::FLOW_TOOLBAR_INSET))
        .right(px(palette::FLOW_TOOLBAR_INSET))
        .flex()
        .flex_row()
        .gap(px(palette::FLOW_TOOLBAR_GAP))
        .child(
            button_bare("flow-toolbar-add")
                .icon(crate::ui::IconName::Plus)
                .tooltip(s::flow_add_node())
                .on_click(cx.listener(|_, _, _, cx| cx.emit(FlowGraphEvent::AddNode))),
        )
        .child(
            // Disabled rather than absent: a button that comes and goes under
            // the pointer is worse than one that says it is not available.
            button_bare("flow-toolbar-delete")
                .icon(crate::ui::IconName::Minus)
                .tooltip(s::flow_delete_node())
                .disabled(!has_selection)
                .on_click(cx.listener(|_, _, _, cx| cx.emit(FlowGraphEvent::Delete))),
        )
        // Both of these read the *file*, so while the inspector holds unsaved
        // edits they would act on something other than what is on screen — and
        // ✓ would go further and call it valid. Off for the same reason, and
        // together: one greyed button beside an identical live one would read as
        // a glitch rather than as a state. The tooltip carries the reason, since
        // a disabled button still shows one and grey on its own is not an
        // answer.
        .child(
            button_bare("flow-toolbar-check")
                .icon(Icon::empty().path(ICON_CHECK))
                .tooltip(reason_or(unsaved_form, s::flow_check_tooltip()))
                .disabled(unsaved_form)
                .debug_selector(|| TOOLBAR_CHECK_SELECTOR.into())
                .on_click(cx.listener(|_, _, _, cx| cx.emit(FlowGraphEvent::Validate))),
        )
        .child(
            button_bare("flow-toolbar-run")
                .icon(Icon::empty().path(ICON_PLAY))
                .tooltip(reason_or(unsaved_form, s::flow_run_tooltip()))
                .disabled(unsaved_form)
                // The press is what the disabled state has to actually stop, and
                // that cannot be seen without a real click.
                .debug_selector(|| TOOLBAR_RUN_SELECTOR.into())
                .on_click(cx.listener(|_, _, _, cx| cx.emit(FlowGraphEvent::Run))),
        )
}

/// A button's tooltip: why it is off, or what it does.
fn reason_or(unsaved_form: bool, does: String) -> String {
    if unsaved_form {
        s::flow_needs_save()
    } else {
        does
    }
}
