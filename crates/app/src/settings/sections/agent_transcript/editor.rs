//! The Fold, Filter and Range controls on an agent catalog row: a button showing the
//! current value, opening the same editor the chat pane opens.
//!
//! What differs from the pane is only what the axis departs from. A pane resets
//! to the agent's stated default; a row has nothing under it but the built-in,
//! so its footer hands back by dropping the key entirely — see
//! [`SettingsView::reset_agent_row_fold_mode`].

use std::rc::Rc;

use gpui::{Anchor, AnyElement, IntoElement, SharedString, Window, div, prelude::*, px};

use crate::surface::strings as s;
use crate::transcript::editor::filter::{FilterEditorActions, filter_editor, filter_value};
use crate::transcript::editor::fold::{FoldEditorActions, fold_editor, mode_value};
use crate::transcript::editor::range::{range_editor, value_label};
use crate::transcript::editor::{
    ResetSpec, TextRoles, aux_icon, dismiss_press, panel_header, panel_root,
};
use crate::ui::theme;
use crate::ui::{Icon, IconName, Popover, PopoverState, button_bare, icons};

use super::super::super::{AgentCatalogRow, SettingsView};

/// One of the three editors a catalog row opens — how a capture names the one
/// it wants open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum EditorShot {
    Fold,
    Filter,
    Range,
}

#[cfg(feature = "screenshot")]
impl EditorShot {
    pub(crate) const ALL: [Self; 3] = [Self::Fold, Self::Filter, Self::Range];

    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::Fold => "fold",
            Self::Filter => "filter",
            Self::Range => "range",
        }
    }

    pub(crate) fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|shot| shot.token() == token)
    }
}

/// Whether `row` renders the `axis` editor already open — only ever for a
/// capture; a live Settings opens every editor shut.
fn opens_for_shot(row: &AgentCatalogRow, axis: EditorShot) -> bool {
    #[cfg(feature = "screenshot")]
    return row.shot_editor == Some(axis);
    #[cfg(not(feature = "screenshot"))]
    {
        let _ = (row, axis);
        false
    }
}

/// The base size the Settings editors scale from — the same 13px the chat
/// pane's editors default to, so one panel design serves both hosts.
const EDITOR_FONT_BASE: f32 = theme::FONT_SIZE_LG;

/// The source line a row's footer and field both read: the row states the
/// axis, or leaves it to the built-in.
fn row_source(overridden: bool) -> String {
    if overridden {
        s::agent_chat::source_agent_set()
    } else {
        s::agent_chat::source_built_in()
    }
}

/// The control a row's editor opens from: an outlined field the width of the
/// dropdowns beside it. The value takes the room and truncates; the source and
/// caret keep their place at the right edge, so a long value cannot push them.
fn field_trigger(id: String, value: String, overridden: bool, cx: &gpui::App) -> crate::ui::Button {
    let t = theme::current(cx);
    button_bare(SharedString::from(id))
        .outline()
        .tab_stop(true)
        .w_full()
        // A column whose items stretch makes the vendored label row span the
        // field, so the value can sit left and the source right.
        .flex_col()
        .items_stretch()
        .tooltip(SharedString::from(value.clone()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_color(t.text_primary)
                .child(SharedString::from(value)),
        )
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(theme::GAP_SM))
                .text_size(px(theme::FONT_SIZE_SM))
                .text_color(t.text_muted)
                .child(SharedString::from(row_source(overridden)))
                .when(overridden, |source| {
                    source.child(aux_icon(icons::PIN, theme::PRIMARY))
                }),
        )
        .child(Icon::new(IconName::ChevronDown))
}

/// The header every Settings editor opens with: the axis it edits.
fn settings_header(
    id: String,
    title: String,
    window: &Window,
    cx: &mut gpui::Context<PopoverState>,
) -> impl IntoElement + use<> {
    let close = dismiss_press(window, cx);
    panel_header(
        SharedString::from(id),
        title,
        TextRoles::from_base(EDITOR_FONT_BASE),
        close,
        cx,
    )
}

pub(in crate::settings) fn fold_mode_control(
    catalog_index: usize,
    row: &AgentCatalogRow,
    cx: &mut gpui::Context<SettingsView>,
) -> impl IntoElement + use<> {
    let window_entity = cx.entity().downgrade();
    let mode = row.fold_mode_value();
    let editor_state = row.fold_editor;
    let overridden = row.fold_mode.is_some();
    Popover::new(SharedString::from(format!(
        "settings-agent-fold-mode-{catalog_index}"
    )))
    .default_open(opens_for_shot(row, EditorShot::Fold))
    .anchor(Anchor::TopLeft)
    .p_0()
    .trigger(field_trigger(
        format!("settings-agent-fold-mode-trigger-{catalog_index}"),
        mode_value(mode),
        overridden,
        cx,
    ))
    .content(move |state, window, cx| {
        let w = window_entity.clone();
        panel_root(window, state)
            .child(settings_header(
                format!("settings-agent-fold-mode-close-{catalog_index}"),
                s::agent_chat::view_options_fold(),
                window,
                cx,
            ))
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
    settings: &gpui::WeakEntity<SettingsView>,
    catalog_index: usize,
    mode: crate::transcript::fold_mode::FoldMode,
    editor_state: crate::transcript::editor::state::FoldEditorState,
    overridden: bool,
    cx: &mut gpui::Context<crate::ui::PopoverState>,
) -> AnyElement {
    let change = settings.clone();
    let preset = settings.clone();
    let history_rules = settings.clone();
    let tools = settings.clone();
    let reset = settings.clone();
    fold_editor(
        mode,
        editor_state,
        &format!("settings-agent-{catalog_index}"),
        EDITOR_FONT_BASE,
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
            on_history_rules: Rc::new(move |app| {
                if let Some(w) = history_rules.upgrade() {
                    w.update(app, |w, cx| {
                        w.toggle_agent_row_fold_history_rules(catalog_index, cx)
                    });
                }
            }),
            on_tools: Rc::new(move |turn, app| {
                if let Some(w) = tools.upgrade() {
                    w.update(app, |w, cx| {
                        w.toggle_agent_row_fold_tools(catalog_index, turn, cx)
                    });
                }
            }),
            reset: Some(ResetSpec {
                label: s::agent_chat::use_built_in(),
                source: row_source(overridden),
                // What the button undoes is the written key, so a row that
                // writes none has nothing to hand back.
                overridden,
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

pub(in crate::settings) fn display_filter_control(
    catalog_index: usize,
    row: &AgentCatalogRow,
    cx: &mut gpui::Context<SettingsView>,
) -> impl IntoElement + use<> {
    let window_entity = cx.entity().downgrade();
    let filter = row.display_filter_value();
    let editor_state = row.filter_editor;
    let overridden = row.display_filter.is_some();
    Popover::new(SharedString::from(format!(
        "settings-agent-display-filter-{catalog_index}"
    )))
    .default_open(opens_for_shot(row, EditorShot::Filter))
    .anchor(Anchor::TopLeft)
    .p_0()
    .trigger(field_trigger(
        format!("settings-agent-display-filter-trigger-{catalog_index}"),
        filter_value(filter),
        overridden,
        cx,
    ))
    .content(move |state, window, cx| {
        let w = window_entity.clone();
        panel_root(window, state)
            .child(settings_header(
                format!("settings-agent-display-filter-close-{catalog_index}"),
                s::agent_chat::view_options_filter(),
                window,
                cx,
            ))
            .child(filter_panel(
                &w,
                catalog_index,
                filter,
                editor_state,
                overridden,
                cx,
            ))
            .into_any_element()
    })
}

pub(in crate::settings) fn range_control(
    catalog_index: usize,
    row: &AgentCatalogRow,
    cx: &mut gpui::Context<SettingsView>,
) -> impl IntoElement + use<> {
    let settings = cx.entity().downgrade();
    let values = row.range_values(cx);
    let overridden = row.tail_window(cx).is_some() || row.tail_window_calls(cx).is_some();
    Popover::new(SharedString::from(format!(
        "settings-agent-range-{catalog_index}"
    )))
    .default_open(opens_for_shot(row, EditorShot::Range))
    .anchor(Anchor::TopLeft)
    .p_0()
    .trigger(field_trigger(
        format!("settings-agent-range-trigger-{catalog_index}"),
        s::agent_chat::tail_window_pair(value_label(values[0]), value_label(values[1])),
        overridden,
        cx,
    ))
    .content(move |state, window, cx| {
        let change = settings.clone();
        let reset = settings.clone();
        panel_root(window, state)
            .child(settings_header(
                format!("settings-agent-range-close-{catalog_index}"),
                s::agent_chat::recent_steps_label(),
                window,
                cx,
            ))
            .child(range_editor(
                &format!("settings-agent-{catalog_index}"),
                values,
                EDITOR_FONT_BASE,
                Rc::new(move |level, size, window, app| {
                    if let Some(settings) = change.upgrade() {
                        settings.update(app, |s, cx| {
                            s.set_agent_row_range_size(catalog_index, level, size, window, cx)
                        });
                    }
                }),
                Some(ResetSpec {
                    label: s::agent_chat::use_built_in(),
                    source: row_source(overridden),
                    overridden,
                    on_reset: Rc::new(move |window, app| {
                        if let Some(settings) = reset.upgrade() {
                            settings.update(app, |s, cx| {
                                s.reset_agent_row_range(catalog_index, window, cx)
                            });
                        }
                    }),
                }),
                cx,
            ))
            .into_any_element()
    })
}

fn filter_panel(
    settings: &gpui::WeakEntity<SettingsView>,
    catalog_index: usize,
    filter: crate::transcript::display_filter::DisplayFilter,
    editor_state: crate::transcript::editor::state::FilterEditorState,
    overridden: bool,
    cx: &mut gpui::Context<crate::ui::PopoverState>,
) -> AnyElement {
    let toggle = settings.clone();
    let section = settings.clone();
    let disclose = settings.clone();
    let reset = settings.clone();
    filter_editor(
        filter,
        editor_state,
        &format!("settings-agent-{catalog_index}"),
        EDITOR_FONT_BASE,
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
            on_disclose: Rc::new(move |axis, app| {
                if let Some(w) = disclose.upgrade() {
                    w.update(app, |w, cx| {
                        w.toggle_agent_row_filter_disclosure(catalog_index, axis, cx)
                    });
                }
            }),
            reset: Some(ResetSpec {
                label: s::agent_chat::use_built_in(),
                source: row_source(overridden),
                overridden,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A row names its source in its own vocabulary — not the pane's, which
    /// follows an agent rather than the built-in.
    #[test]
    fn a_row_names_its_source_apart_from_a_pane() {
        assert_eq!(row_source(false), s::agent_chat::source_built_in());
        assert_eq!(row_source(true), s::agent_chat::source_agent_set());
        for source in [row_source(false), row_source(true)] {
            assert_ne!(source, s::agent_chat::source_following_agent());
            assert_ne!(source, s::agent_chat::source_this_chat());
        }
    }
}
