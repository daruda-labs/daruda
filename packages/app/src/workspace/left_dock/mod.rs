//! Left dock — view tab strip plus per-view renderers.
//!
//! Hosts three swappable views (Lanes / Git / Files) from
//! `daruda_store::project::LeftDockView`; owns the header tab strip that
//! switches between them, with bodies rendered by `render.rs`.

use gpui::{Context, Div, Window, div, prelude::*};

use crate::workspace::Workspace;

/// How long a left-dock cursor rests on a row before its file loads. Arrow
/// keys walk a list far faster than a file read + `git diff` + highlight pass,
/// so without the wait every row merely passed over would spend one.
const PREVIEW_DELAY: std::time::Duration = std::time::Duration::from_millis(150);

/// Next position for a wrap-around cursor step over `len` rows, where `len`
/// is non-zero. `current` is `None` when the cursor is not on any row — Down
/// then starts at the top and Up at the bottom.
///
/// Shared by the Git and Files panels. They answer the same keys with the same
/// meaning, and every time that meaning has lived in two places it has drifted
/// — this is the arithmetic both of them used to keep their own copy of.
pub(in crate::workspace) fn wrap_step(current: Option<usize>, delta: isize, len: usize) -> usize {
    debug_assert!(len > 0, "callers return early on an empty list");
    match current {
        Some(i) => ((i as isize + delta).rem_euclid(len as isize)) as usize,
        None if delta >= 0 => 0,
        None => len - 1,
    }
}

impl Workspace {
    /// Run `open` once the left-dock cursor has rested for [`PREVIEW_DELAY`].
    /// Re-arming drops the previous `Task`, which cancels it, so a row the
    /// cursor only passes over never loads — the same mechanism
    /// `arm_tab_hover_switch` uses for hover-to-switch.
    ///
    /// One slot for both panels: only one of them holds focus at a time, and a
    /// cursor move in either should cancel whatever the other left pending.
    ///
    /// `panel` is the handle the arrow key came from. Both it and the lane are
    /// re-checked when the timer elapses: a preview opens a file *and switches
    /// the active tab*, so firing for a panel the user has already left would
    /// move them somewhere they walked away from.
    ///
    /// `contains_focused`, not `is_focused`: the panel's own header buttons are
    /// `gpui_component::Button`s, which `track_focus` their own handle, and
    /// GPUI's focusable mouse-down handler `prevent_default()`s so the ancestor
    /// panel does not get it back. The arrow keys still reach the panel's key
    /// context from there, so an exact-handle test would walk the cursor with
    /// nothing ever opening.
    pub(in crate::workspace) fn arm_left_dock_preview(
        &mut self,
        panel: gpui::FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
        open: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let armed_for = self.active;
        self.left_dock_preview = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(PREVIEW_DELAY).await;
            this.update_in(cx, |ws, window, cx| {
                if ws.active == armed_for && panel.contains_focused(window, cx) {
                    open(ws, window, cx);
                }
            })
            .ok();
        }));
    }
}

pub(in crate::workspace) mod file_tree_context;
pub(in crate::workspace) mod file_tree_ops;
pub(super) mod files;
pub(super) mod footer;
pub(super) mod git_changes;
pub(in crate::workspace) mod git_ops;
pub(super) mod projects;
pub(super) mod view_tabs;

/// Shared scaffold for a left-dock view body: a full-size vertical flex
/// column that clips overflow, giving all three views one definition for
/// sizing and overflow. Per-view concerns stay at the call site (Lanes
/// adds card `gap`; Git / Files chain `key_context` + `track_focus`).
pub(in crate::workspace) fn left_panel_body() -> Div {
    div().flex().flex_col().size_full().overflow_hidden()
}

#[cfg(test)]
mod tests {
    use super::wrap_step;

    /// The ends wrap in both directions, and a cursor that is on no row enters
    /// the list from the side the key points at.
    #[test]
    fn wrap_step_walks_and_wraps_from_either_end() {
        let cases = [
            ((Some(0), 1isize, 3usize), 1, "down from the top"),
            (
                (Some(2), 1, 3),
                0,
                "down from the last row wraps to the first",
            ),
            (
                (Some(0), -1, 3),
                2,
                "up from the first row wraps to the last",
            ),
            ((Some(2), -1, 3), 1, "up from the bottom"),
            ((None, 1, 3), 0, "no cursor + down enters at the top"),
            ((None, -1, 3), 2, "no cursor + up enters at the bottom"),
            ((Some(0), 1, 1), 0, "a one-row list stays put"),
        ];
        for ((current, delta, len), expected, msg) in cases {
            assert_eq!(wrap_step(current, delta, len), expected, "{msg}");
        }
    }
}
