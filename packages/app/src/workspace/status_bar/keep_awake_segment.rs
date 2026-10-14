//! Status bar's keep-awake segment — the configured mode, whether the
//! computer is being held awake right now, and a menu to switch the mode.

use super::StatusBarDensity;
use crate::keep_awake::KeepAwakeStatus;
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{PopupMenu, PopupMenuItem, button_status_pill_bare};
use crate::workspace::{OpenSettings, Workspace};
use daruda_config::{BuiltinSection, KeepAwakeMode};
use gpui::{AnyElement, App, SharedString, WeakEntity, div, prelude::*, px};

/// Snapshot the segment renders from: the configured mode and what the
/// policy is doing with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct KeepAwakeView {
    pub mode: KeepAwakeMode,
    pub status: KeepAwakeStatus,
}

/// Which theme colour the coffee icon takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Muted,
    Active,
    Failed,
}

/// What follows the icon: nothing at rest, a dot while holding, a warning
/// glyph on failure — so each state differs by shape, not colour alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Marker {
    None,
    Dot,
    Warning,
}

/// How one state looks; the marker and the wording carry it beside colour.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Look {
    tone: Tone,
    marker: Marker,
    summary: String,
}

fn mode_label(mode: KeepAwakeMode) -> String {
    daruda_settings::labels::keep_awake_mode_label(mode)
}

/// A failed lease outranks the mode: an `Off` whose release failed may still
/// be holding the computer awake.
fn look(view: KeepAwakeView) -> Look {
    let idle = |summary| Look {
        tone: Tone::Muted,
        marker: Marker::None,
        summary,
    };
    match (view.mode, view.status) {
        (_, KeepAwakeStatus::Failed) => Look {
            tone: Tone::Failed,
            marker: Marker::Warning,
            summary: s::keep_awake::status_failed(),
        },
        (KeepAwakeMode::Off, _) => idle(s::keep_awake::status_off()),
        (mode, KeepAwakeStatus::Inactive) => idle(s::keep_awake::status_idle(mode_label(mode))),
        (_, KeepAwakeStatus::Active { working, waiting }) => Look {
            tone: Tone::Active,
            marker: Marker::Dot,
            summary: match (working, waiting) {
                (0, 0) => s::keep_awake::status_always(),
                (working, 0) => s::keep_awake::status_working(working),
                (0, waiting) => s::keep_awake::status_waiting(waiting),
                (working, waiting) => s::keep_awake::status_working_waiting(working, waiting),
            },
        },
    }
}

/// The narrowest tier drops the mode word; the icon and its marker still
/// carry the state, and the tooltip and menu name the mode.
fn shows_mode_label(density: StatusBarDensity) -> bool {
    density != StatusBarDensity::IconOnly
}

/// Render the segment's dropdown trigger.
pub(super) fn render(
    view: KeepAwakeView,
    density: StatusBarDensity,
    workspace: WeakEntity<Workspace>,
    cx: &App,
) -> AnyElement {
    let t = theme::current(cx);
    let look = look(view);
    let icon_color = match look.tone {
        Tone::Muted => t.text_muted,
        Tone::Active => t.text_primary,
        Tone::Failed => t.keep_awake_failed,
    };
    let summary = SharedString::from(look.summary);
    let trigger = button_status_pill_bare("status-keep-awake", cx)
        .flex_none()
        .text_size(px(theme::STATUS_BAR_FONT_SIZE))
        .tooltip(summary.clone())
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::STATUS_BAR_USAGE_CHIP_GAP))
                .child(crate::ui::agent_icon(
                    Some(crate::ui::icons::KEEP_AWAKE),
                    px(theme::STATUS_BAR_AGENT_ICON_SIZE),
                    icon_color,
                ))
                .when(shows_mode_label(density), |el| {
                    el.child(SharedString::from(mode_label(view.mode)))
                })
                .map(|el| match look.marker {
                    Marker::None => el,
                    Marker::Dot => el.child(
                        div()
                            .flex_none()
                            .w(px(theme::STATUS_BAR_DOT_SIZE))
                            .h(px(theme::STATUS_BAR_DOT_SIZE))
                            .rounded_full()
                            .bg(t.keep_awake_active),
                    ),
                    Marker::Warning => el.child(crate::ui::agent_icon(
                        Some(crate::ui::icons::WARNING),
                        px(theme::STATUS_BAR_AGENT_ICON_SIZE),
                        t.keep_awake_failed,
                    )),
                }),
        );
    let builder =
        crate::ui::menu_builder(move |menu, _window, _cx| build_menu(view, &workspace, menu));
    super::menu::dropdown("status-keep-awake", trigger, builder, cx)
}

/// The menu's modes, top to bottom.
const MODES: [KeepAwakeMode; 3] = [KeepAwakeMode::Off, KeepAwakeMode::Agent, KeepAwakeMode::On];

/// The header's short state word; the counts stay in the pill's tooltip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuState {
    Active,
    Inactive,
    Failed,
}

fn menu_state(status: KeepAwakeStatus) -> MenuState {
    match status {
        KeepAwakeStatus::Active { .. } => MenuState::Active,
        KeepAwakeStatus::Inactive => MenuState::Inactive,
        KeepAwakeStatus::Failed => MenuState::Failed,
    }
}

/// The header's right-hand text, e.g. `Agent · Active`.
fn menu_status(view: KeepAwakeView) -> String {
    let state = match menu_state(view.status) {
        MenuState::Active => s::keep_awake::state_active(),
        MenuState::Inactive => s::keep_awake::state_inactive(),
        MenuState::Failed => s::keep_awake::state_failed(),
    };
    s::keep_awake::menu_status(mode_label(view.mode), state)
}

fn mode_description(mode: KeepAwakeMode) -> String {
    match mode {
        KeepAwakeMode::Off => s::keep_awake::desc_off(),
        KeepAwakeMode::Agent => s::keep_awake::desc_agent(),
        KeepAwakeMode::On => s::keep_awake::desc_on(),
    }
}

/// A titled header, the three modes with a dot on the current one, then the
/// Settings page. Each item is a one-line dispatch.
fn build_menu(
    view: KeepAwakeView,
    workspace: &WeakEntity<Workspace>,
    menu: PopupMenu,
) -> PopupMenu {
    // A failure takes its own colour so it is not read past.
    let tone = if menu_state(view.status) == MenuState::Failed {
        super::menu::Tone::Themed(|cx| theme::current(cx).keep_awake_failed)
    } else {
        super::menu::Tone::Plain
    };
    let menu = menu
        .item(super::menu::menu_header(
            s::keep_awake::menu_title(),
            Some(menu_status(view).into()),
            tone,
        ))
        .separator();
    let menu = MODES.into_iter().fold(menu, |menu, mode| {
        let workspace = workspace.clone();
        let current = mode == view.mode;
        menu.item(
            PopupMenuItem::element(move |_window, cx| {
                super::menu::choice_row(
                    mode_label(mode).into(),
                    Some(mode_description(mode).into()),
                    current,
                    cx,
                )
            })
            .on_click(move |_, _window, app| {
                if let Some(ws) = workspace.upgrade() {
                    ws.update(app, |ws, cx| ws.set_keep_awake_mode(mode, cx));
                }
            }),
        )
    });
    super::menu::with_footer(menu, s::keep_awake::menu_settings(), |_, window, app| {
        window.dispatch_action(Box::new(OpenSettings(BuiltinSection::Agent)), app);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(mode: KeepAwakeMode, status: KeepAwakeStatus) -> KeepAwakeView {
        KeepAwakeView { mode, status }
    }

    #[test]
    fn off_is_muted_without_a_dot() {
        let look = look(view(KeepAwakeMode::Off, KeepAwakeStatus::Inactive));
        assert_eq!(look.tone, Tone::Muted);
        assert_eq!(look.marker, Marker::None);
        assert_eq!(look.summary, s::keep_awake::status_off());
    }

    #[test]
    fn an_idle_agent_mode_says_it_is_not_active() {
        let look = look(view(KeepAwakeMode::Agent, KeepAwakeStatus::Inactive));
        assert_eq!(look.tone, Tone::Muted);
        assert_eq!(look.marker, Marker::None);
        assert_eq!(
            look.summary,
            s::keep_awake::status_idle(mode_label(KeepAwakeMode::Agent))
        );
    }

    #[test]
    fn an_active_hold_names_only_the_counts_it_has() {
        let active = |working, waiting| {
            look(view(
                KeepAwakeMode::Agent,
                KeepAwakeStatus::Active { working, waiting },
            ))
        };
        let both = active(2, 1);
        assert_eq!(both.tone, Tone::Active);
        assert_eq!(both.marker, Marker::Dot);
        assert_eq!(both.summary, s::keep_awake::status_working_waiting(2, 1));
        assert_eq!(active(1, 0).summary, s::keep_awake::status_working(1));
        assert_eq!(active(0, 3).summary, s::keep_awake::status_waiting(3));
    }

    #[test]
    fn always_on_with_no_sessions_says_always() {
        let look = look(view(
            KeepAwakeMode::On,
            KeepAwakeStatus::Active {
                working: 0,
                waiting: 0,
            },
        ));
        assert_eq!(look.tone, Tone::Active);
        assert_eq!(look.marker, Marker::Dot);
        assert_eq!(look.summary, s::keep_awake::status_always());
    }

    /// Whatever the mode, a refused lease is what the user needs to see,
    /// and its warning glyph tells it from a hold without relying on colour.
    #[test]
    fn a_failure_warns_in_every_mode() {
        for mode in [KeepAwakeMode::Off, KeepAwakeMode::Agent, KeepAwakeMode::On] {
            let look = look(view(mode, KeepAwakeStatus::Failed));
            assert_eq!(look.tone, Tone::Failed, "{mode:?}");
            assert_eq!(look.marker, Marker::Warning, "{mode:?}");
            assert_eq!(look.summary, s::keep_awake::status_failed());
        }
    }

    #[test]
    fn only_the_narrowest_tier_drops_the_mode_word() {
        assert!(shows_mode_label(StatusBarDensity::Full));
        assert!(shows_mode_label(StatusBarDensity::Compact));
        assert!(!shows_mode_label(StatusBarDensity::IconOnly));
    }

    #[test]
    fn the_menu_lists_off_then_agent_then_on() {
        assert_eq!(
            MODES,
            [KeepAwakeMode::Off, KeepAwakeMode::Agent, KeepAwakeMode::On]
        );
    }

    #[test]
    fn the_header_status_names_the_mode_and_its_state() {
        let status = |mode, status| menu_status(view(mode, status));
        assert_eq!(
            status(KeepAwakeMode::Off, KeepAwakeStatus::Inactive),
            s::keep_awake::menu_status(
                mode_label(KeepAwakeMode::Off),
                s::keep_awake::state_inactive()
            )
        );
        assert_eq!(
            status(KeepAwakeMode::Agent, KeepAwakeStatus::Inactive),
            s::keep_awake::menu_status(
                mode_label(KeepAwakeMode::Agent),
                s::keep_awake::state_inactive()
            )
        );
        assert_eq!(
            status(
                KeepAwakeMode::Agent,
                KeepAwakeStatus::Active {
                    working: 2,
                    waiting: 1
                }
            ),
            s::keep_awake::menu_status(
                mode_label(KeepAwakeMode::Agent),
                s::keep_awake::state_active()
            )
        );
        assert_eq!(
            status(KeepAwakeMode::On, KeepAwakeStatus::Failed),
            s::keep_awake::menu_status(
                mode_label(KeepAwakeMode::On),
                s::keep_awake::state_failed()
            )
        );
    }

    #[test]
    fn every_mode_has_a_description() {
        for mode in MODES {
            assert!(!mode_description(mode).is_empty(), "{mode:?}");
        }
    }
}
