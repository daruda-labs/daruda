//! Landing view — the center content of a workspace holding no projects.
//!
//! Reached two ways: `NewEmptyWindow`, and closing a workspace's last
//! project. Both used to leave a blank element here; the content is the
//! Welcome window's, which this replaces.
//!
//! Every affordance is a one-line action dispatch, which is also what
//! keeps the "reuse this window" rule: `OpenFolder` resolves to `AddHere`
//! for an empty workspace, and a recent row dispatches
//! `OpenRecentWorkspace`, whose `OpenMode::ReplaceCurrent` closes this
//! window as the successor opens.
//!
//! The chrome is a verbatim port of the Welcome window it replaces — bespoke
//! `div` rather than the `crate::ui` wrappers, which is why the `WELCOME_*`
//! theme constants were kept. Moving it onto the wrappers is a separate,
//! visual change.

use gpui::{AnyElement, Context, MouseButton, SharedString, div, prelude::*, px};

use crate::surface::keybindings as k;
use crate::surface::shortcut_display::display;
use crate::surface::strings as s;
use crate::ui::theme;
use crate::workspace::Workspace;

/// One cheat-sheet row: the binding as the app declares it, and the label
/// describing what it does. Read from [`crate::surface::keybindings`] so a
/// remap moves the sheet with it.
fn shortcut_rows() -> [(&'static str, String); 4] {
    [
        (k::SHORTCUT_OPEN_FOLDER, s::welcome_shortcut_open_folder()),
        (
            k::SHORTCUT_OPEN_FOLDER_IN_NEW_WINDOW,
            s::welcome_shortcut_open_folder_new_window(),
        ),
        (k::SHORTCUT_NEW_WINDOW, s::welcome_shortcut_new_window()),
        (
            k::SHORTCUT_KEYBOARD_SHORTCUTS,
            s::welcome_shortcut_command_palette(),
        ),
    ]
}

/// Build the Landing element: heading → body → actions, the same ordering
/// `present_empty_state` uses one level down.
pub(super) fn render(cx: &mut Context<Workspace>) -> AnyElement {
    // Every colour is copied out before `cx` is used mutably below —
    // `theme::current` borrows it for as long as the returned theme lives.
    let (
        primary,
        faint,
        muted,
        button_bg,
        button_border,
        button_hover_bg,
        recent_hover_bg,
        panel_bg,
    ) = {
        let t = theme::current(cx);
        (
            t.text_primary,
            t.text_subtle,
            t.text_muted,
            t.welcome_button_bg,
            t.border,
            t.welcome_button_hover_bg,
            t.welcome_recent_hover_bg,
            t.welcome_bg,
        )
    };

    let title = div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(theme::WELCOME_GAP_TIGHT))
        .child(
            div()
                .text_size(px(theme::WELCOME_TITLE_FONT_SIZE))
                .text_color(primary)
                .child(s::welcome_title()),
        )
        .child(
            div()
                .text_size(px(theme::WELCOME_VERSION_FONT_SIZE))
                .text_color(faint)
                .child(s::WELCOME_VERSION),
        );

    let open_folder_btn = div()
        .id("landing-open-folder")
        .flex()
        .items_center()
        .justify_center()
        .w_full()
        .px(px(theme::WELCOME_BUTTON_PAD_X))
        .py(px(theme::WELCOME_BUTTON_PAD_Y))
        .bg(button_bg)
        .border_1()
        .border_color(button_border)
        .rounded(px(theme::WELCOME_BUTTON_RADIUS))
        .text_size(px(theme::WELCOME_BUTTON_FONT_SIZE))
        .text_color(primary)
        .cursor_pointer()
        .hover(move |d| d.bg(button_hover_bg))
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.dispatch_action(Box::new(crate::OpenFolder), cx)
        })
        .child(s::welcome_open_folder());

    let new_empty_btn = div()
        .id("landing-new-empty")
        .flex()
        .items_center()
        .justify_center()
        .w_full()
        .px(px(theme::WELCOME_BUTTON_PAD_X))
        .py(px(theme::WELCOME_BUTTON_PAD_Y))
        .rounded(px(theme::WELCOME_BUTTON_RADIUS))
        .text_size(px(theme::WELCOME_BUTTON_FONT_SIZE))
        .text_color(muted)
        .cursor_pointer()
        .hover(move |d| d.bg(button_hover_bg).text_color(primary))
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.dispatch_action(Box::new(crate::NewEmptyWindow), cx)
        })
        .child(s::welcome_new_empty());

    let panel = div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(theme::WELCOME_GAP))
        .w(px(theme::WELCOME_PANEL_WIDTH))
        .p(px(theme::WELCOME_PANEL_PAD))
        .child(title)
        .child(open_folder_btn)
        .child(recent_section(primary, faint, muted, recent_hover_bg, cx))
        .child(new_empty_btn)
        .child(cheat_sheet(faint, muted));

    div()
        .flex_1()
        .w_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(panel_bg)
        .child(panel)
        .into_any_element()
}

/// Recent-projects rows, or the "none yet" line.
///
/// The list comes from [`crate::menus::RecentSnapshot`], not `load_recent_in`:
/// reading the recent file here would be a disk read inside `render`. A
/// missing global (no menu bar installed yet) renders as "no recent"
/// rather than panicking — the same degradation the app-drawn menu takes.
fn recent_section(
    primary: gpui::Hsla,
    faint: gpui::Hsla,
    muted: gpui::Hsla,
    hover_bg: gpui::Hsla,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let recent = cx
        .try_global::<crate::menus::RecentSnapshot>()
        .map(|snap| snap.0.clone())
        .unwrap_or_default();

    if recent.is_empty() {
        return div()
            .flex()
            .flex_col()
            .gap(px(theme::WELCOME_GAP_TIGHT))
            .w_full()
            .child(
                div()
                    .text_size(px(theme::WELCOME_HEADING_FONT_SIZE))
                    .text_color(faint)
                    .child(s::welcome_no_recent()),
            )
            .into_any_element();
    }

    // Each row carries the workspace's identity, not its position, so the
    // label and what the click opens cannot disagree even if the snapshot
    // has drifted from the list on disk.
    let rows = recent
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let action = crate::OpenRecentWorkspace(entry.workspace_uuid);
            let display_name = SharedString::from(entry.display_name.clone());
            div()
                .id(("landing-recent", i))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::WELCOME_GAP_LOOSE))
                .w_full()
                .px(px(theme::WELCOME_RECENT_PAD_X))
                .py(px(theme::WELCOME_RECENT_PAD_Y))
                .rounded(px(theme::WELCOME_RECENT_RADIUS))
                .cursor_pointer()
                .hover(move |d| d.bg(hover_bg))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    window.dispatch_action(Box::new(action.clone()), cx)
                })
                .child(
                    div()
                        .text_size(px(theme::WELCOME_RECENT_FONT_SIZE))
                        .text_color(primary)
                        .child(display_name),
                )
        })
        .collect::<Vec<_>>();

    div()
        .flex()
        .flex_col()
        .gap(px(theme::WELCOME_GAP_TIGHT))
        .w_full()
        .child(
            div()
                .text_size(px(theme::WELCOME_HEADING_FONT_SIZE))
                .text_color(muted)
                .child(s::welcome_recent()),
        )
        .children(rows)
        .into_any_element()
}

/// Keyboard cheat sheet — binding on the right, what it does on the left.
fn cheat_sheet(faint: gpui::Hsla, muted: gpui::Hsla) -> AnyElement {
    let rows = shortcut_rows().map(|(binding, label)| {
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .w_full()
            .gap(px(theme::WELCOME_GAP_LOOSE))
            .child(
                div()
                    .text_size(px(theme::WELCOME_VERSION_FONT_SIZE))
                    .text_color(muted)
                    .child(label),
            )
            .child(
                div()
                    .text_size(px(theme::WELCOME_VERSION_FONT_SIZE))
                    .text_color(faint)
                    .child(display(binding)),
            )
    });

    div()
        .flex()
        .flex_col()
        .gap(px(theme::WELCOME_GAP_TIGHT))
        .w_full()
        .child(
            div()
                .text_size(px(theme::WELCOME_HEADING_FONT_SIZE))
                .text_color(muted)
                .child(s::welcome_shortcuts()),
        )
        .children(rows)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every cheat-sheet row must name a binding the app actually declares,
    /// and render it non-empty. A row whose binding const was deleted would
    /// otherwise show a blank key column.
    #[test]
    fn every_cheat_sheet_row_renders_a_binding() {
        for (binding, label) in shortcut_rows() {
            assert!(!binding.is_empty(), "binding const is empty");
            assert!(!label.is_empty(), "label for {binding} is empty");
            assert!(
                !display(binding).is_empty(),
                "binding {binding} renders empty"
            );
        }
    }

    /// Rows address a workspace by identity, so what a row opens is fixed
    /// at render time and cannot drift with the list's ordering — the
    /// property that lets Landing render from a cached snapshot at all.
    #[test]
    fn a_row_carries_the_workspace_identity_it_names() {
        let uuid = daruda_store::project::WorkspaceUuid::new();
        let action = crate::OpenRecentWorkspace(uuid);
        assert_eq!(action.0, uuid);
        assert_eq!(action.clone(), crate::OpenRecentWorkspace(uuid));
        assert_ne!(
            action,
            crate::OpenRecentWorkspace(daruda_store::project::WorkspaceUuid::new())
        );
    }
}
