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
//! Chrome comes from the `crate::ui` wrappers, like its sibling
//! `present_empty_state` — the two empty states are the same family and the
//! decision matrix in `crates/app/src/CLAUDE.md` bans hand-rolled `div`
//! buttons at a call site. The `WELCOME_*` theme constants that survive here
//! are the layout metrics the wrappers do not own (panel width, gaps, the
//! title and version type scale).

use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};

use crate::surface::keybindings as k;
use crate::surface::shortcut_display::display;
use crate::surface::strings as s;
use crate::ui::{button, button_primary, picker_row, theme};
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
    // Copied out before `cx` is used mutably below — `theme::current`
    // borrows it for as long as the returned theme lives. Button and row
    // chrome is the wrappers' to colour; what is left is the text this
    // module lays out itself.
    let (primary, faint, muted, panel_bg) = {
        let t = theme::current(cx);
        (t.text_primary, t.text_subtle, t.text_muted, t.welcome_bg)
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

    let open_folder_btn = button_primary("landing-open-folder", s::welcome_open_folder())
        .w_full()
        .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::OpenFolder), cx));

    let new_empty_btn = button("landing-new-empty", s::welcome_new_empty())
        .w_full()
        .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::NewEmptyWindow), cx));

    let panel = div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(theme::WELCOME_GAP))
        .w(px(theme::WELCOME_PANEL_WIDTH))
        .p(px(theme::WELCOME_PANEL_PAD))
        .child(title)
        .child(open_folder_btn)
        .child(recent_section(faint, muted, cx))
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
fn recent_section(faint: gpui::Hsla, muted: gpui::Hsla, cx: &mut Context<Workspace>) -> AnyElement {
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
    // has drifted from the list on disk. `picker_row` is the shared row
    // chrome every search-and-pick overlay uses; nothing here is focused, so
    // it renders in its resting state.
    let rows = recent
        .iter()
        .map(|entry| {
            let action = crate::OpenRecentWorkspace(entry.workspace_uuid);
            picker_row(
                false,
                SharedString::from(entry.display_name.clone()),
                None,
                move |window, cx| window.dispatch_action(Box::new(action.clone()), cx),
                cx,
            )
            .into_any_element()
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
