//! The Fold and Filter controls on an agent catalog row: a button showing the
//! current value, opening the same editor the chat pane opens.
//!
//! What differs from the pane is only what the axis departs from. A pane resets
//! to the agent's stated default; a row has nothing under it but the built-in,
//! so its footer hands back by dropping the key entirely — see
//! [`SettingsWindow::reset_agent_row_fold_mode`].

use std::rc::Rc;

use gpui::{Anchor, AnyElement, IntoElement, SharedString, prelude::*};

use crate::surface::strings as s;
use crate::transcript::editor::filter::{FilterEditorActions, filter_editor, filter_value};
use crate::transcript::editor::fold::{FoldEditorActions, fold_editor, mode_value};
use crate::transcript::editor::{ResetSpec, panel_root};
use crate::ui::theme;
use crate::ui::{Popover, button};

use super::super::super::{AgentCatalogRow, SettingsWindow};

/// The control a row's editor opens from: an outlined field the width of the
/// dropdowns beside it, so the three transcript axes read as one row of fields
/// rather than two kinds of control.
fn field_trigger(id: String, label: String) -> crate::ui::Button {
    button(SharedString::from(id), SharedString::from(label))
        .outline()
        .w_full()
        .justify_start()
}

/// The value text a row shows without opening the editor, marked when the row
/// states the axis rather than leaving it to the built-in.
fn row_value_label(value: String, overridden: bool) -> String {
    if overridden {
        value
    } else {
        s::settings_agent_transcript_built_in(&value)
    }
}

pub(in crate::settings_window) fn fold_mode_control(
    catalog_index: usize,
    row: &AgentCatalogRow,
    cx: &mut gpui::Context<SettingsWindow>,
) -> impl IntoElement + use<> {
    let window_entity = cx.entity().downgrade();
    let mode = row.fold_mode_value();
    let editor_state = row.fold_editor;
    let overridden = row.fold_mode.is_some();
    Popover::new(SharedString::from(format!(
        "settings-agent-fold-mode-{catalog_index}"
    )))
    .anchor(Anchor::TopLeft)
    .trigger(field_trigger(
        format!("settings-agent-fold-mode-trigger-{catalog_index}"),
        row_value_label(mode_value(mode), overridden),
    ))
    .content(move |_, window, cx| {
        let w = window_entity.clone();
        panel_root(theme::TRANSCRIPT_EDITOR_RULES_PANEL_W, window)
            .child(fold_panel(
                &w,
                catalog_index,
                mode,
                editor_state,
                overridden,
                cx,
            ))
            .into_any_element()
    })
}

fn fold_panel(
    settings: &gpui::WeakEntity<SettingsWindow>,
    catalog_index: usize,
    mode: crate::transcript::fold_mode::FoldMode,
    editor_state: crate::transcript::editor::state::FoldEditorState,
    overridden: bool,
    cx: &mut gpui::Context<crate::ui::PopoverState>,
) -> AnyElement {
    let change = settings.clone();
    let preset = settings.clone();
    let turn = settings.clone();
    let reset = settings.clone();
    fold_editor(
        mode,
        editor_state,
        &format!("settings-agent-{catalog_index}"),
        theme::MODAL_BODY_FONT_SIZE,
        FoldEditorActions {
            on_change: Rc::new(move |mode, _window, app| {
                if let Some(w) = change.upgrade() {
                    w.update(app, |w, cx| {
                        w.set_agent_row_fold_mode(catalog_index, Some(mode), cx)
                    });
                }
            }),
            on_preset: Rc::new(move |p, _window, app| {
                if let Some(w) = preset.upgrade() {
                    w.update(app, |w, cx| {
                        w.select_agent_row_fold_preset(catalog_index, p, cx)
                    });
                }
            }),
            on_turn: Rc::new(move |t, app| {
                if let Some(w) = turn.upgrade() {
                    w.update(app, |w, cx| w.set_agent_row_fold_turn(catalog_index, t, cx));
                }
            }),
            reset: Some(ResetSpec {
                // What the button undoes is the written key, so a row that
                // writes none has nothing to hand back.
                disabled: !overridden,
                on_reset: Rc::new(move |_window, app| {
                    if let Some(w) = reset.upgrade() {
                        w.update(app, |w, cx| w.reset_agent_row_fold_mode(catalog_index, cx));
                    }
                }),
            }),
        },
        cx,
    )
}

pub(in crate::settings_window) fn display_filter_control(
    catalog_index: usize,
    row: &AgentCatalogRow,
    cx: &mut gpui::Context<SettingsWindow>,
) -> impl IntoElement + use<> {
    let window_entity = cx.entity().downgrade();
    let filter = row.display_filter_value();
    let overridden = row.display_filter.is_some();
    Popover::new(SharedString::from(format!(
        "settings-agent-display-filter-{catalog_index}"
    )))
    .anchor(Anchor::TopLeft)
    .trigger(field_trigger(
        format!("settings-agent-display-filter-trigger-{catalog_index}"),
        row_value_label(filter_value(filter), overridden),
    ))
    .content(move |_, window, cx| {
        let w = window_entity.clone();
        panel_root(theme::TRANSCRIPT_EDITOR_PANEL_W, window)
            .child(filter_panel(&w, catalog_index, filter, overridden, cx))
            .into_any_element()
    })
}

fn filter_panel(
    settings: &gpui::WeakEntity<SettingsWindow>,
    catalog_index: usize,
    filter: crate::transcript::display_filter::DisplayFilter,
    overridden: bool,
    cx: &mut gpui::Context<crate::ui::PopoverState>,
) -> AnyElement {
    let toggle = settings.clone();
    let section = settings.clone();
    let reset = settings.clone();
    filter_editor(
        filter,
        &format!("settings-agent-{catalog_index}"),
        theme::MODAL_BODY_FONT_SIZE,
        FilterEditorActions {
            on_toggle: Rc::new(move |facet, app| {
                if let Some(w) = toggle.upgrade() {
                    w.update(app, |w, cx| {
                        w.toggle_agent_row_filter_facet(catalog_index, facet, cx)
                    });
                }
            }),
            on_section: Rc::new(move |parent, on, app| {
                if let Some(w) = section.upgrade() {
                    w.update(app, |w, cx| {
                        w.set_agent_row_filter_section(catalog_index, parent, on, cx)
                    });
                }
            }),
            reset: Some(ResetSpec {
                disabled: !overridden,
                on_reset: Rc::new(move |_window, app| {
                    if let Some(w) = reset.upgrade() {
                        w.update(app, |w, cx| {
                            w.reset_agent_row_display_filter(catalog_index, cx)
                        });
                    }
                }),
            }),
        },
        cx,
    )
}
