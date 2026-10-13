//! The tab strip above the panes: one cell per tab, its context menu, and the
//! `+` new-tab button.

use crate::ui::theme;
use crate::ui::{DropdownMenu as _, PopupMenuItem, menu_builder};
use gpui::{Context, DragMoveEvent, MouseButton, MouseDownEvent, div, prelude::*, px};

use super::menus::{build_new_tab_menu, ws_popup_clipboard_item, ws_popup_menu_item};
use super::tab_cells::{self, TabCell};
use crate::workspace::main_area::pane_drag_ops::PaneHeaderDrag;
use crate::workspace::main_area::pane_tree::SplitDirection;
use crate::workspace::main_area::tab_drag_ops::{TabDrag, TabDragGhost};
use crate::workspace::main_area::tab_ops::NewPaneKind;
use crate::workspace::root_menu::RootContextMenuExt as _;
use crate::workspace::{TAB_BAR_HEIGHT, Workspace};

impl Workspace {
    /// The tab strip: one cell per tab of the active lane, then the `+` button.
    pub(super) fn build_tab_bar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let t = theme::current(cx);
        let dark = t.is_dark();
        // These tab-strip slots still read raw consts; pick light-aware values
        // so the tab bar doesn't render dark with white text on the light theme.
        let tab_bar_bg = if dark {
            theme::SURFACE_1
        } else {
            theme::LIGHT_SURFACE_1
        };
        let tab_bar_border = if dark {
            theme::HAIRLINE
        } else {
            theme::LIGHT_SURFACE_3
        };
        let tab_active_bg = if dark {
            theme::CANVAS
        } else {
            theme::LIGHT_CANVAS
        };
        // Active file tabs are workspace chrome, so they stay on the Daruda UI
        // theme token. The file-viewer body/editor surface may follow the
        // terminal-mirrored content color, but that should not leak into the
        // tab strip.
        let tab_active_file_chrome_bg = t.file_viewer_bg;
        let tab_active_text = if dark {
            t.text_primary
        } else {
            theme::LIGHT_INK
        };
        let tab_inactive_bg = t.tab_inactive_bg;
        let tab_inactive_text = t.text_muted;
        let tab_inactive_hover_bg = t.tab_inactive_hover_bg;
        let tab_insertion_line_color = t.terminal_drop_target_bg;

        // Pre-collect tab bar data (no entity reads during element construction).
        let tab_cells = self.tab_cells(cx);

        // iTerm2-style tab bar:
        // - Each tab grows to share row width (min 80px, max 220px); text
        //   truncates via overflow_hidden + whitespace_nowrap.
        // - Number prefix matches the cmd-N hotkey for that tab.
        // - Close × is hover-only (group hover) — matches iTerm2's
        //   `tabCloseButtonsAlwaysVisible = NO` default.
        // - Middle-click on a tab closes it (iTerm2 `middleClickClosesTab`).
        // - Active tab gets a 2px bottom accent + brighter bg.
        // ── Tab bar ──────────────────────────────────────
        // Reorder-insertion indicator: `Some(k)` means "insert the dragged
        // tab before slot k" (k == tab_cells.len() means "at the end").
        // Rendered as a border on the adjacent cell (or the "+" button for
        // the end slot) rather than an absolute-positioned overlay, so it
        // never has to re-derive the tab bar's flex-computed x offsets.
        let tab_reorder_preview = self.main_area.tab_reorder_preview.map(|(_, index)| index);
        let tab_count = tab_cells.len();
        div()
            .flex()
            .flex_row()
            .w_full()
            .h(px(TAB_BAR_HEIGHT))
            .relative()
            .bg(tab_bar_bg)
            .items_center()
            .children(tab_cells.into_iter().map(
                |TabCell {
                     index: i,
                     id: tab_id,
                     is_active,
                     label: display,
                     title,
                     file_path,
                     worktree_root,
                     is_scratch,
                     indicator,
                 }| {
                    let is_orchestrator = self
                        .active_runtime()
                        .tabs
                        .get(i)
                        .is_some_and(|tab| self.is_orchestrator_tab(tab));
                    // Stop the left-press from bubbling to the tab cell's
                    // `on_mouse_down(Left, activate_tab)` below — clicking ×
                    // must close the tab without first activating it. The
                    // Button's own `on_click` (mouse-up) does the close.
                    let close_button = crate::ui::button_close(("tab-close", i), cx)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.request_close_tab(i, window, cx);
                        }));
                    let drag_title = display.clone();
                    // Precomputed here (not inside `.context_menu`'s builder) since
                    // every value derives from data already in scope for this tab —
                    // no live `Workspace` read needed at menu-open time.
                    let ws = cx.entity().downgrade();
                    let abs_path = file_path.clone();
                    let rel_path = file_path.as_ref().and_then(|p| {
                        worktree_root.as_ref().and_then(|root| {
                            p.strip_prefix(root)
                                .ok()
                                .map(|r| r.to_string_lossy().into_owned())
                        })
                    });
                    let abs_str = abs_path.as_ref().map(|p| p.to_string_lossy().into_owned());
                    let is_file = abs_path.is_some();
                    let is_last = i + 1 >= tab_count;

                    div()
                        .id(("tab", i))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(theme::TAB_GAP))
                        .pl(px(theme::TAB_PAD_LEFT))
                        .pr(px(theme::TAB_PAD_RIGHT))
                        .py(px(theme::TAB_PAD_Y))
                        .mx(px(theme::TAB_MARGIN_X))
                        .min_w(px(theme::TAB_MIN_WIDTH))
                        .max_w(px(theme::TAB_MAX_WIDTH))
                        .flex_grow()
                        .flex_basis(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(theme::TAB_FONT_SIZE))
                        .cursor_pointer()
                        .when(Some(i) == tab_reorder_preview, |d| {
                            d.border_l_2().border_color(tab_insertion_line_color)
                        })
                        .when(is_active, |d| {
                            let active_bg = if file_path.is_some() {
                                tab_active_file_chrome_bg
                            } else {
                                tab_active_bg
                            };
                            d.bg(active_bg).text_color(tab_active_text)
                        })
                        .when(!is_active, |d| {
                            d.bg(tab_inactive_bg)
                                .text_color(tab_inactive_text)
                                .hover(move |d| d.bg(tab_inactive_hover_bg))
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                this.on_tab_press(i, tab_id, event.click_count, window, cx);
                            }),
                        )
                        .tooltip(crate::ui::tooltip::text(match indicator {
                            Some(indicator) => {
                                crate::surface::strings::tab_strip::tooltip_with_status(
                                    &title,
                                    indicator.label(),
                                )
                                .into()
                            }
                            None => title,
                        }))
                        .on_mouse_down(
                            MouseButton::Middle,
                            cx.listener(move |this, _, window, cx| {
                                this.request_close_tab(i, window, cx);
                            }),
                        )
                        .when(!is_orchestrator, |el| {
                            el.on_drag(
                                TabDrag {
                                    tab_id,
                                    title: drag_title,
                                },
                                |d, offset, _window, cx| {
                                    cx.new(|_| TabDragGhost {
                                        title: d.title.clone(),
                                        offset,
                                    })
                                },
                            )
                        })
                        .on_drag_move::<TabDrag>(cx.listener(
                            move |this, event: &DragMoveEvent<TabDrag>, window, cx| {
                                this.update_tab_drag_from_move(tab_id, i, event, window, cx);
                            },
                        ))
                        .on_drop::<TabDrag>(cx.listener(move |this, d: &TabDrag, window, cx| {
                            this.drop_tab_onto_bar(d.tab_id, window, cx);
                        }))
                        .root_context_menu(ws.clone(), move |menu, _window, _cx| {
                            use crate::surface::strings as s;

                            let mut items: Vec<PopupMenuItem> = Vec::new();
                            if !is_orchestrator {
                                items.push(ws_popup_menu_item(
                                    ws.clone(),
                                    s::ctx::rename_tab(),
                                    false,
                                    move |this, win, cx| {
                                        this.open_rename_tab_dialog(tab_id, win, cx)
                                    },
                                ));
                                items.push(PopupMenuItem::separator());
                            }
                            items.extend([
                                ws_popup_menu_item(
                                    ws.clone(),
                                    s::common::close_tab(),
                                    false,
                                    move |this, win, cx| {
                                        this.mutate_durable_in(win, cx, |ws, win, cx| {
                                            ws.request_close_tab(i, win, cx);
                                        });
                                    },
                                ),
                                ws_popup_menu_item(
                                    ws.clone(),
                                    s::ctx::close_other_tabs(),
                                    tab_count <= 1,
                                    move |this, win, cx| {
                                        this.mutate_durable_in(win, cx, |ws, win, cx| {
                                            ws.close_other_tabs(i, win, cx);
                                        });
                                    },
                                ),
                                ws_popup_menu_item(
                                    ws.clone(),
                                    s::ctx::close_tabs_to_right(),
                                    is_last,
                                    move |this, win, cx| {
                                        this.mutate_durable_in(win, cx, |ws, win, cx| {
                                            ws.close_tabs_to_right(i, win, cx);
                                        });
                                    },
                                ),
                                PopupMenuItem::separator(),
                                ws_popup_menu_item(
                                    ws.clone(),
                                    s::common::move_tab_left(),
                                    i == 0,
                                    move |this, _win, cx| {
                                        this.mutate_durable(cx, |ws, cx| ws.move_tab_left(i, cx));
                                    },
                                ),
                                ws_popup_menu_item(
                                    ws.clone(),
                                    s::common::move_tab_right(),
                                    is_last,
                                    move |this, _win, cx| {
                                        this.mutate_durable(cx, |ws, cx| ws.move_tab_right(i, cx));
                                    },
                                ),
                            ]);

                            // Split + New Tab — terminal tabs only.
                            if !is_file {
                                items.extend([
                                    PopupMenuItem::separator(),
                                    ws_popup_menu_item(
                                        ws.clone(),
                                        s::common::split_terminal_horizontal(),
                                        false,
                                        move |this, win, cx| {
                                            this.mutate_durable_in(win, cx, |ws, win, cx| {
                                                ws.split_from_tab(
                                                    i,
                                                    NewPaneKind::Terminal,
                                                    SplitDirection::Horizontal,
                                                    win,
                                                    cx,
                                                );
                                            });
                                        },
                                    ),
                                    ws_popup_menu_item(
                                        ws.clone(),
                                        s::common::split_terminal_vertical(),
                                        false,
                                        move |this, win, cx| {
                                            this.mutate_durable_in(win, cx, |ws, win, cx| {
                                                ws.split_from_tab(
                                                    i,
                                                    NewPaneKind::Terminal,
                                                    SplitDirection::Vertical,
                                                    win,
                                                    cx,
                                                );
                                            });
                                        },
                                    ),
                                    PopupMenuItem::separator(),
                                    ws_popup_menu_item(
                                        ws.clone(),
                                        s::common::split_agent_chat_horizontal(),
                                        false,
                                        move |this, win, cx| {
                                            this.mutate_durable_in(win, cx, |ws, win, cx| {
                                                ws.split_from_tab(
                                                    i,
                                                    NewPaneKind::AgentChat,
                                                    SplitDirection::Horizontal,
                                                    win,
                                                    cx,
                                                );
                                            });
                                        },
                                    ),
                                    ws_popup_menu_item(
                                        ws.clone(),
                                        s::common::split_agent_chat_vertical(),
                                        false,
                                        move |this, win, cx| {
                                            this.mutate_durable_in(win, cx, |ws, win, cx| {
                                                ws.split_from_tab(
                                                    i,
                                                    NewPaneKind::AgentChat,
                                                    SplitDirection::Vertical,
                                                    win,
                                                    cx,
                                                );
                                            });
                                        },
                                    ),
                                    PopupMenuItem::separator(),
                                    ws_popup_menu_item(
                                        ws.clone(),
                                        crate::surface::strings::common::new_tab(),
                                        false,
                                        |this, win, cx| {
                                            this.mutate_durable_in(win, cx, |ws, win, cx| {
                                                ws.add_tab(win, cx);
                                            });
                                        },
                                    ),
                                ]);
                            }

                            // File-pane-specific items
                            if is_file {
                                items.push(PopupMenuItem::separator());
                                if let Some(abs) = abs_str.clone() {
                                    items.push(ws_popup_clipboard_item(
                                        s::ctx::copy_file_path(),
                                        abs,
                                    ));
                                }
                                if let Some(rel) = rel_path.clone() {
                                    items.push(ws_popup_clipboard_item(
                                        s::ctx::copy_relative_path(),
                                        rel,
                                    ));
                                }
                                items.push(ws_popup_menu_item(
                                    ws.clone(),
                                    s::ctx::close_file_viewer(),
                                    false,
                                    move |this, win, cx| {
                                        this.mutate_durable_in(win, cx, |ws, win, cx| {
                                            ws.request_close_tab(i, win, cx);
                                        });
                                    },
                                ));
                            }

                            items.into_iter().fold(menu, |m, item| m.item(item))
                        })
                        .when_some(
                            indicator.map(|i| tab_cells::indicator_color(i, cx)),
                            |cell, color| {
                                cell.child(
                                    div()
                                        .flex_none()
                                        .mr(px(theme::TAB_STATUS_DOT_GAP))
                                        .size(px(theme::TAB_STATUS_DOT_SIZE))
                                        .rounded_full()
                                        .bg(color),
                                )
                            },
                        )
                        .child({
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                // Italic marks the scratch tab — the one the
                                // next left-dock preview replaces. Same
                                // convention as VS Code and zed, and it drops
                                // the moment the tab stops being replaceable.
                                .when(is_scratch, |d| d.italic())
                                .child(display)
                        })
                        .child(close_button)
                },
            ))
            .child({
                // "+" opens a dropdown so a new tab can be a terminal or an Agent
                // chat pane under a chosen agent. Left-click dropdowns attach to a
                // Button (`impl DropdownMenu for Button`), so this is a ghost button
                // sized to match the tab bar via the NEW_TAB_* metrics, not a div.
                let ws = cx.entity().downgrade();
                // Snapshot the catalog as owned (id, name) pairs for the
                // 'static menu closure.
                let agents: Vec<(String, String)> = self
                    .mirrors
                    .agents
                    .iter()
                    .map(|a| (a.id.clone(), a.name.clone()))
                    .collect();
                crate::ui::button_icon("new-tab-btn", crate::ui::icons::ADD, cx)
                    .tooltip(crate::surface::strings::common::new_tab())
                    .mx(px(theme::NEW_TAB_MARGIN_X))
                    .text_size(px(theme::NEW_TAB_FONT_SIZE))
                    // Reorder-insertion indicator for "drop at the very
                    // end" — the counterpart of the per-cell border above
                    // when `tab_reorder_preview` points past the last tab.
                    .when(tab_reorder_preview == Some(tab_count), |d| {
                        d.border_l_2().border_color(tab_insertion_line_color)
                    })
                    .dropdown_menu(menu_builder(move |menu, window, cx| {
                        build_new_tab_menu(menu, &ws, &agents, window, cx)
                    }))
            })
            .child(
                div()
                    .absolute()
                    .left_0()
                    .bottom_0()
                    .size_full()
                    .border_b_1()
                    .border_color(tab_bar_border),
            )
            // Dropping a dragged pane header onto the tab bar itself (rather
            // than onto another pane's content) detaches it into a new tab,
            // appended at the end — a positional insert by drop x-offset is
            // out of scope for v1.
            .on_drop::<PaneHeaderDrag>(cx.listener(|this, d: &PaneHeaderDrag, window, cx| {
                this.mutate_durable_in(window, cx, |ws, window, cx| {
                    let at = ws.active_runtime().tabs.len();
                    ws.detach_pane_to_new_tab(d.dragged, at, window, cx);
                });
            }))
            // Fallback for a `TabDrag` released on the tab bar but off every
            // individual cell's own hitbox (e.g. over the "+" new-tab
            // button, which is exactly where the "insert at the end"
            // indicator renders — the cells resolve to `Hold` there, so the
            // end slot survives to be committed here). A per-cell
            // `on_drop::<TabDrag>` handling the drop first calls
            // `cx.stop_propagation()`, so this container-level listener only
            // runs when no cell caught it.
            .on_drop::<TabDrag>(cx.listener(|this, d: &TabDrag, window, cx| {
                this.drop_tab_onto_bar(d.tab_id, window, cx);
            }))
    }
}
