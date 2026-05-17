//! Bottom dock tab strip — one tab per `panels.tabs` entry, click to
//! switch the active tab.
//!
//! Built on `crate::ui::{tab, tab_bar}` so the strip matches the
//! sidebar / right-panel underline TabBars. The built-in "Input" tab
//! occupies index 0; user-defined macro tabs follow. The `+` create
//! chip rides on `TabBar::suffix` — the one place the wrapper permits
//! suffix usage (see `crate::ui::tab_bar` doc comment).
//!
//! `gpui_component::Tab` implements the full `InteractiveElement` +
//! `StatefulInteractiveElement` trait suite, so drag-reorder and
//! right-click context menus hang off each macro tab directly via
//! `on_drag` / `drag_over` / `on_drop` / `on_mouse_down`. The built-in
//! Input tab carries no such handlers — click-to-activate is handled
//! at the TabBar level through `on_click(ix)`.

use crate::ui::theme;
use crate::ui::{Tab, tab, tab_bar};
use daruda_store::panels::TabId;
use gpui::{
    AnyElement, App, ClickEvent, Context, Focusable, IntoElement, MouseButton, MouseDownEvent,
    Pixels, Point, Render, SharedString, WeakEntity, Window, div, prelude::*, px,
};

use crate::surface::strings as surface_strings;
use crate::ui::dialog::ButtonVariant;

use crate::ui::ContextMenuItem;
use crate::workspace::Workspace;
use crate::workspace::dock::Dock;
use crate::workspace::dock_snap::BottomDockSnapshot;

/// Row-preset table: dock height → number of macro-tile rows visible.
/// Order matters: `nearest_row_preset` walks midpoints between adjacent
/// entries, so keep heights monotonically increasing.
const ROW_PRESETS: [(u8, f32); 3] = [
    (1, theme::DOCK_BOTTOM_ROW_PRESET_1_H),
    (2, theme::DOCK_BOTTOM_ROW_PRESET_2_H),
    (3, theme::DOCK_BOTTOM_ROW_PRESET_3_H),
];

/// Pick the row preset whose height is closest to the current dock
/// size. Used by the suffix button to label the active preset and by
/// the dropdown to mark the active entry with a check glyph; also
/// consumed by `terminal_input` to swap between the vertical chrome
/// (2/3 rows) and the inline single-row layout (1 row).
pub(in crate::workspace::bottom_panel) fn nearest_row_preset(size: f32) -> u8 {
    let mid_12 = (ROW_PRESETS[0].1 + ROW_PRESETS[1].1) / 2.0;
    let mid_23 = (ROW_PRESETS[1].1 + ROW_PRESETS[2].1) / 2.0;
    if size <= mid_12 {
        ROW_PRESETS[0].0
    } else if size <= mid_23 {
        ROW_PRESETS[1].0
    } else {
        ROW_PRESETS[2].0
    }
}

// ----------------------------------------------------------------
// Drag payload — passed through GPUI's on_drag / on_drop chain.
// ----------------------------------------------------------------

/// Data carried by a panel tab during a drag operation. Stays in
/// scope for the entire drag — `label` powers the floating ghost
/// without re-reading `panels.tabs`.
#[derive(Clone, Debug)]
pub(in crate::workspace) struct DraggedPanelTab {
    pub id: TabId,
    pub label: SharedString,
}

/// Floating ghost element under the cursor while a panel tab is
/// being dragged.
struct DraggedPanelTabGhost {
    label: SharedString,
}

impl Render for DraggedPanelTabGhost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        let text_color = t.sidebar_view_tab_active;
        let bg = t.panel_tab_drop_target_bg;
        div()
            .px(px(theme::SIDEBAR_VIEW_TAB_PAD_X))
            .py(px(theme::WORKTREE_DRAG_GHOST_PAD_Y))
            .text_size(px(theme::SIDEBAR_VIEW_TAB_FONT_SIZE))
            .text_color(text_color)
            .bg(bg)
            .rounded(px(theme::MODAL_BUTTON_RADIUS))
            .child(self.label.clone())
    }
}

/// Render the bottom dock tab strip. Index 0 is always the built-in
/// Input tab; macro tabs follow in `tab_summaries` order. The `+`
/// chip lives in the TabBar's right-edge `suffix` slot.
pub(in crate::workspace) fn render(
    snap: &BottomDockSnapshot,
    cx: &mut Context<Dock>,
) -> AnyElement {
    let active_id = snap.active_tab_id.clone();
    let terminal_input_visible = snap.terminal_input_visible;
    let tabs_sorted = snap.tab_summaries.clone();
    let workspace = snap.workspace.clone();
    let terminal_input = snap.terminal_input.clone();

    let active_ix = if terminal_input_visible {
        0
    } else {
        active_id
            .as_ref()
            .and_then(|active| tabs_sorted.iter().position(|(id, _, _)| id == active))
            .map(|i| i + 1)
            .unwrap_or(0)
    };

    let mut children: Vec<Tab> = Vec::with_capacity(tabs_sorted.len() + 1);
    children.push(builtin_input_tab());
    for (tab_id, name, widget_count) in tabs_sorted.iter().cloned() {
        children.push(macro_tab(tab_id, name, widget_count, &workspace, cx));
    }

    let click_tab_ids: Vec<TabId> = tabs_sorted.iter().map(|(id, _, _)| id.clone()).collect();
    let click_workspace = workspace.clone();
    let click_terminal_input = terminal_input.clone();

    tab_bar("bottom-dock-tabs")
        .w_full()
        .gap(px(0.))
        .selected_index(active_ix)
        .children(children)
        .on_click(move |ix, window, cx| {
            if *ix == 0 {
                if let Some(ws) = click_workspace.upgrade() {
                    let fh = click_terminal_input.focus_handle(cx);
                    ws.update(cx, |ws, cx| ws.activate_bottom_input(cx));
                    fh.focus(window, cx);
                }
            } else if let Some(tab_id) = click_tab_ids.get(*ix - 1).cloned()
                && let Some(ws) = click_workspace.upgrade()
            {
                ws.update(cx, |ws, cx| ws.set_active_panel_tab(tab_id, cx));
            }
        })
        .suffix(
            // Two chip-style buttons sit in the suffix gutter: the
            // always-visible `+` (panel-tab create) and, when a macro
            // tab is active, the row-preset chip (`1`/`2`/`3` opening
            // a dropdown). The chip chrome (outline + compact) keeps
            // them readable as discrete buttons rather than a run-on
            // glyph sequence. Input-tab activation hides the row-preset
            // chip because the Input panel has no rows to fit.
            div()
                .flex()
                .items_center()
                .h_full()
                .gap(px(theme::PANEL_BODY_GAP))
                .px(px(theme::SIDEBAR_VIEW_TAB_PAD_X))
                .child(add_tab_button(snap, cx))
                .when(!terminal_input_visible, |el| {
                    el.child(row_preset_button(snap, cx))
                }),
        )
        .into_any_element()
}

/// Built-in "Input" tab. No drag / right-click — click-to-activate is
/// driven from the TabBar's `on_click(ix=0)` branch.
fn builtin_input_tab() -> Tab {
    tab(surface_strings::BOTTOM_INPUT_TAB_LABEL)
}

/// Row-preset chip rendered next to `[+]` in the TabBar suffix. Shows
/// the nearest preset to the current dock height (e.g. `2`) and opens
/// a context menu listing the three presets when clicked.
fn row_preset_button(snap: &BottomDockSnapshot, cx: &mut Context<Dock>) -> impl IntoElement {
    let current_preset = nearest_row_preset(snap.bottom_dock_size);
    let workspace = snap.workspace.clone();
    let label = SharedString::from(current_preset.to_string());
    crate::ui::button_chip("bottom-dock-row-preset", label).on_click(cx.listener(
        move |_dock, ev: &ClickEvent, _window, cx| {
            cx.stop_propagation();
            let Some(ws) = workspace.upgrade() else {
                return;
            };
            let position = ev.position();
            let items = build_row_preset_menu(current_preset, workspace.clone());
            ws.update(cx, |ws, cx| {
                // Anchor by bottom-right so the menu expands up-and-left
                // from the chip — the chip sits at the right edge of
                // the tab bar, and opening down-and-right would clip.
                ws.open_context_menu_at_corner(
                    position,
                    items,
                    crate::ui::ContextMenuCorner::BottomRight,
                    cx,
                )
            });
        },
    ))
}

fn build_row_preset_menu(
    current_preset: u8,
    workspace: WeakEntity<Workspace>,
) -> Vec<ContextMenuItem> {
    ROW_PRESETS
        .iter()
        .map(|(rows, height)| row_preset_item(*rows, *height, current_preset, workspace.clone()))
        .collect()
}

fn row_preset_item(
    rows: u8,
    preset_h: f32,
    current_preset: u8,
    workspace: WeakEntity<Workspace>,
) -> ContextMenuItem {
    let prefix = if rows == current_preset {
        surface_strings::ROW_PRESET_CHECK_PREFIX
    } else {
        surface_strings::ROW_PRESET_UNCHECK_PREFIX
    };
    let body = match rows {
        1 => surface_strings::ROW_PRESET_1_LABEL,
        2 => surface_strings::ROW_PRESET_2_LABEL,
        _ => surface_strings::ROW_PRESET_3_LABEL,
    };
    let label = format!("{prefix}{body}");
    ContextMenuItem::new(label, move |_ev, window, app_cx| {
        let Some(ws) = workspace.upgrade() else {
            return;
        };
        ws.update(app_cx, |ws, cx| {
            ws.close_context_menu(cx);
            ws.set_bottom_dock_row_preset(preset_h, window, cx);
        });
    })
}

/// `[+]` button rendered in the TabBar's right-edge `suffix` slot.
/// Uses the same chip chrome as `row_preset_button` so the two
/// adjacent buttons read as discrete actions rather than a single
/// glyph sequence.
fn add_tab_button(snap: &BottomDockSnapshot, cx: &mut Context<Dock>) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    crate::ui::button_chip("panel-tab-add", "+").on_click(cx.listener(
        move |_dock, _: &ClickEvent, window, cx| {
            if let Some(ws) = workspace.upgrade() {
                let callback_ws = workspace.clone();
                ws.update(cx, |ws, cx| {
                    let _ = ws;
                    crate::workspace::dialog_helpers::open_single_field_dialog(
                        callback_ws.clone(),
                        surface_strings::CREATE_PANEL_TAB_MODAL_TITLE,
                        surface_strings::CREATE_PANEL_TAB_PLACEHOLDER,
                        None,
                        |ws, value, _window, cx| {
                            if let Some(name) = value {
                                ws.add_panel_tab(name, cx);
                            }
                        },
                        window,
                        cx,
                    );
                });
            }
        },
    ))
}

fn macro_tab(
    tab_id: TabId,
    label: String,
    widget_count: usize,
    workspace: &WeakEntity<Workspace>,
    cx: &mut Context<Dock>,
) -> Tab {
    let label_shared = SharedString::from(label.clone());

    let on_right_click = {
        let tab_id = tab_id.clone();
        let label = label.clone();
        let ws = workspace.clone();
        cx.listener(move |_dock, ev: &MouseDownEvent, _window, cx| {
            // Stop ancestors from interpreting the right-click before
            // the menu can render (mirrors the worktrees row pattern).
            cx.stop_propagation();
            let position: Point<Pixels> = ev.position;
            if let Some(w) = ws.upgrade() {
                let items =
                    build_tab_context_menu(tab_id.clone(), label.clone(), widget_count, ws.clone());
                w.update(cx, |ws, cx| ws.open_context_menu(position, items, cx));
            }
        })
    };

    let drag_payload = DraggedPanelTab {
        id: tab_id.clone(),
        label: label_shared.clone(),
    };

    let drop_workspace = workspace.clone();
    tab(label_shared)
        .on_mouse_down(MouseButton::Right, on_right_click)
        .on_drag(drag_payload, |dragged, _offset, _window, cx| {
            cx.new(|_| DraggedPanelTabGhost {
                label: dragged.label.clone(),
            })
        })
        .drag_over::<DraggedPanelTab>(|style, _dragged, _window, cx| {
            style.bg(theme::current(cx).panel_tab_drop_target_bg)
        })
        .on_drop({
            let drop_target_id = tab_id.clone();
            cx.listener(move |_dock, dragged: &DraggedPanelTab, _window, cx| {
                if let Some(w) = drop_workspace.upgrade() {
                    w.update(cx, |ws, cx| {
                        ws.reorder_panel_tab(dragged.id.clone(), drop_target_id.clone(), cx)
                    });
                }
            })
        })
}

/// Build the context menu for a tab right-click — Rename + Delete.
fn build_tab_context_menu(
    tab_id: TabId,
    current_name: String,
    widget_count: usize,
    workspace: WeakEntity<Workspace>,
) -> Vec<ContextMenuItem> {
    vec![
        rename_item(tab_id.clone(), current_name.clone(), workspace.clone()),
        delete_item(tab_id, current_name, widget_count, workspace),
    ]
}

fn rename_item(
    tab_id: TabId,
    current_name: String,
    workspace: WeakEntity<Workspace>,
) -> ContextMenuItem {
    ContextMenuItem::new(
        surface_strings::CTX_PANEL_TAB_RENAME,
        move |_ev: &MouseDownEvent, window: &mut Window, app_cx: &mut App| {
            let Some(ws) = workspace.upgrade() else {
                return;
            };
            let tab_id = tab_id.clone();
            let initial = current_name.clone();
            let callback_ws = workspace.clone();
            ws.update(app_cx, |ws, cx| {
                ws.close_context_menu(cx);
                crate::workspace::dialog_helpers::open_single_field_dialog(
                    callback_ws.clone(),
                    surface_strings::RENAME_PANEL_TAB_MODAL_TITLE,
                    surface_strings::RENAME_PANEL_TAB_PLACEHOLDER,
                    Some(&initial),
                    {
                        let tab_id = tab_id.clone();
                        move |ws, value, _window, cx| {
                            if let Some(name) = value {
                                ws.rename_panel_tab(tab_id.clone(), name, cx);
                            }
                        }
                    },
                    window,
                    cx,
                );
            });
        },
    )
}

fn delete_item(
    tab_id: TabId,
    current_name: String,
    widget_count: usize,
    workspace: WeakEntity<Workspace>,
) -> ContextMenuItem {
    ContextMenuItem::new(
        surface_strings::CTX_PANEL_TAB_DELETE,
        move |_ev: &MouseDownEvent, window: &mut Window, app_cx: &mut App| {
            let Some(ws) = workspace.upgrade() else {
                return;
            };
            let tab_id = tab_id.clone();
            let body = format_delete_body(&current_name, widget_count);
            let callback_ws = workspace.clone();
            ws.update(app_cx, |ws, cx| {
                ws.close_context_menu(cx);
            });
            crate::workspace::dialog_helpers::open_confirm_dialog(
                surface_strings::DELETE_PANEL_TAB_MODAL_TITLE,
                body,
                surface_strings::DELETE_PANEL_TAB_CONFIRM_LABEL,
                ButtonVariant::Danger,
                move |_, _window, app_cx| {
                    if let Some(ws) = callback_ws.upgrade() {
                        let tab_id = tab_id.clone();
                        ws.update(app_cx, |ws, cx| {
                            ws.delete_panel_tab(tab_id, cx);
                        });
                    }
                },
                window,
                app_cx,
            );
        },
    )
}

/// Build "Delete tab 'X'? N widgets will be removed." style body.
/// Singular when widget_count == 1, omits the count clause when 0.
fn format_delete_body(name: &str, widget_count: usize) -> String {
    match widget_count {
        0 => format!("Delete tab \u{201c}{name}\u{201d}?"),
        1 => format!("Delete tab \u{201c}{name}\u{201d}? 1 widget will be removed."),
        n => format!("Delete tab \u{201c}{name}\u{201d}? {n} widgets will be removed."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_body_zero_widgets() {
        assert_eq!(
            format_delete_body("AI", 0),
            "Delete tab \u{201c}AI\u{201d}?"
        );
    }

    #[test]
    fn delete_body_one_widget() {
        assert_eq!(
            format_delete_body("Build", 1),
            "Delete tab \u{201c}Build\u{201d}? 1 widget will be removed."
        );
    }

    #[test]
    fn delete_body_many_widgets() {
        assert_eq!(
            format_delete_body("AI", 7),
            "Delete tab \u{201c}AI\u{201d}? 7 widgets will be removed."
        );
    }

    #[test]
    fn nearest_preset_at_anchors() {
        assert_eq!(nearest_row_preset(theme::DOCK_BOTTOM_ROW_PRESET_1_H), 1);
        assert_eq!(nearest_row_preset(theme::DOCK_BOTTOM_ROW_PRESET_2_H), 2);
        assert_eq!(nearest_row_preset(theme::DOCK_BOTTOM_ROW_PRESET_3_H), 3);
    }

    #[test]
    fn nearest_preset_handles_default_and_extremes() {
        // Default dock height equals the 1-row preset — fresh projects
        // open compact.
        assert_eq!(nearest_row_preset(theme::DOCK_BOTTOM_DEFAULT_H), 1);
        // Min height collapses onto 1.
        assert_eq!(nearest_row_preset(theme::DOCK_BOTTOM_MIN_H), 1);
        // Max height (and anything beyond the 2↔3 midpoint) saturates
        // at 3.
        assert_eq!(nearest_row_preset(theme::DOCK_BOTTOM_MAX_H), 3);
    }

    #[test]
    fn nearest_preset_uses_midpoints() {
        let mid_12 = (theme::DOCK_BOTTOM_ROW_PRESET_1_H + theme::DOCK_BOTTOM_ROW_PRESET_2_H) / 2.0;
        let mid_23 = (theme::DOCK_BOTTOM_ROW_PRESET_2_H + theme::DOCK_BOTTOM_ROW_PRESET_3_H) / 2.0;
        // Exact midpoints round down to the smaller preset.
        assert_eq!(nearest_row_preset(mid_12), 1);
        assert_eq!(nearest_row_preset(mid_23), 2);
        // Just above each midpoint moves to the next preset.
        assert_eq!(nearest_row_preset(mid_12 + 1.0), 2);
        assert_eq!(nearest_row_preset(mid_23 + 1.0), 3);
    }
}
