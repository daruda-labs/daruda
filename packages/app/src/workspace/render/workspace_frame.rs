//! The workspace frame: title bar, tab strip, panes and docks, plus the
//! root's key context and action listeners.

use crate::ui::theme;
use gpui::{
    ClipboardItem, Context, Focusable as _, KeyDownEvent, MouseButton, MouseMoveEvent,
    MouseUpEvent, Window, div, prelude::*, px,
};

use super::docks::{DockFrame, dock_resize_handle, dock_toggle_icon};
use super::{center, root_key_context};
use crate::workspace::layout::DockPosition;
use crate::workspace::main_area::pane_drag_ops::PaneHeaderDrag;
use crate::workspace::main_area::pane_tree::SplitDirection;
use crate::workspace::main_area::tab_drag_ops::TabDrag;
use crate::workspace::{
    FileViewerSearchNext, FileViewerSearchOpen, FileViewerSearchPrev, SaveFilePane, Workspace,
};

impl Workspace {
    /// Title bar, tab strip, panes and docks.
    pub(super) fn build_workspace_frame(
        &self,
        docks: DockFrame,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let t = theme::current(cx);
        let title_bar_bg = t.title_bar_bg;
        let DockFrame {
            dragged_dock,
            left_dock_open,
            left_dock_size,
            bottom_dock_open,
            bottom_dock_size,
            right_dock_open,
            right_dock_size,
        } = docks;

        // ── Title bar ──────────────────────────────────────
        // `title_bar::render` owns the row: the left inset (traffic lights on
        // macOS, plain margin elsewhere), the drag strip, and — where the
        // platform left no caption — the window controls.
        let left_dock_toggle = dock_toggle_icon(
            "toggle-left-dock",
            DockPosition::Left,
            left_dock_open,
            cx,
            cx.listener(|this, _, window, cx| {
                this.on_toggle_left_dock(&crate::workspace::ToggleLeftDock, window, cx);
            }),
        );
        let trailing_dock_toggles = div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(theme::DOCK_ICON_GROUP_GAP))
            .mr(px(theme::DOCK_ICON_GROUP_MR))
            .child(dock_toggle_icon(
                "toggle-bottom-dock",
                DockPosition::Bottom,
                bottom_dock_open,
                cx,
                cx.listener(|this, _, window, cx| {
                    this.on_toggle_bottom_dock(&crate::workspace::ToggleBottomDock, window, cx);
                }),
            ))
            .child(dock_toggle_icon(
                "toggle-right-dock",
                DockPosition::Right,
                right_dock_open,
                cx,
                cx.listener(|this, _, window, cx| {
                    this.on_toggle_right_dock(&crate::workspace::ToggleRightDock, window, cx);
                }),
            ));
        let title_bar = crate::title_bar::render(
            crate::title_bar::chrome_for_window(window),
            title_bar_bg,
            Some(left_dock_toggle.into_any_element()),
            Some(trailing_dock_toggles.into_any_element()),
            window,
            cx,
        );

        let tab_bar = self.build_tab_bar(cx);

        // Content area — the active tab's pane layout, the inaccessible-
        // lane empty state, or a blank fallback. Built in `center.rs` so
        // this method stays focused on top-level layout assembly.
        let center_content = center::render_center_content(self, cx);

        // BodyLayout: [LeftDock] [MainArea] [RightDock]
        // Resize handles are absolutely positioned overlays centered
        // on each dock's border (see `dock_resize_handle`) — they don't
        // consume flex space, so toggling docks doesn't reflow layout.
        let pane_area = div()
            .flex_1()
            .w_full()
            .flex()
            .on_drag_move::<PaneHeaderDrag>(cx.listener(Self::update_pane_drag_from_move))
            .on_drag_move::<TabDrag>(cx.listener(Self::update_tab_merge_hover_from_move))
            .child(center_content);
        let main_area = div()
            .flex_1()
            .flex()
            .flex_col()
            .relative()
            .overflow_hidden()
            .when(self.workspace_page.is_none(), |el| el.child(tab_bar))
            .child(pane_area)
            // `.cached()`: when the bottom dock isn't notified (its
            // snapshot was staged unchanged above), GPUI recycles its
            // previous layout + paint instead of re-rendering the macro
            // grid / terminal input on every parent repaint. The
            // self-notifying terminal input entity inside still repaints
            // on its own edits (Pitfall #10).
            .when(bottom_dock_open && self.workspace_page.is_none(), |el| {
                el.child(
                    gpui::AnyView::from(self.docks.bottom.clone()).cached(
                        gpui::StyleRefinement::default()
                            .w_full()
                            .h(gpui::px(bottom_dock_size)),
                    ),
                )
            })
            .when(bottom_dock_open && self.workspace_page.is_none(), |el| {
                el.child(dock_resize_handle(
                    DockPosition::Bottom,
                    bottom_dock_size,
                    dragged_dock,
                    cx,
                ))
            });

        let body_layout = div()
            .flex_1()
            .flex()
            .flex_row()
            .relative()
            .overflow_hidden()
            // `.cached()`: the left dock re-renders only when its staged
            // snapshot content changes. The staging path above diffs each
            // frame's `LeftDockSnapshot` (`content_differs`) and dirties the
            // dock only on a real change, so any workspace `cx.notify()`
            // (source mutations, lane/project/group CRUD, git ops, file-tree
            // ops, claude status) refreshes it for free. On unrelated parent
            // repaints (terminal output) the cached layout + paint is recycled
            // instead of rebuilding the entire Worktrees / Git-changes / Files
            // tree. The status pulse still dirties the dock directly to
            // advance badge animation (its frames aren't in the snapshot).
            // Embedded `git_commit_input` and focus/scroll handles self-notify
            // and dirty the dock as an ancestor (Pitfall #10).
            .child(
                gpui::AnyView::from(self.docks.left.clone()).cached(
                    gpui::StyleRefinement::default()
                        .h_full()
                        .w(gpui::px(left_dock_size)),
                ),
            )
            .child(main_area)
            // `.cached()`: staging diffs `RightDockSnapshot`
            // (`content_differs`) and repaints only on real change, so a
            // workspace `cx.notify()` suffices. Exceptions keeping an
            // explicit `notify_right_dock`: the status pulse and task-live
            // tick, whose changes (animation, `now`) aren't in the snapshot.
            .when(right_dock_open, |el| {
                el.child(
                    gpui::AnyView::from(self.docks.right.clone()).cached(
                        gpui::StyleRefinement::default()
                            .h_full()
                            .w(gpui::px(right_dock_size)),
                    ),
                )
            })
            .child(dock_resize_handle(
                DockPosition::Left,
                left_dock_size,
                dragged_dock,
                cx,
            ))
            .when(right_dock_open, |el| {
                el.child(dock_resize_handle(
                    DockPosition::Right,
                    right_dock_size,
                    dragged_dock,
                    cx,
                ))
            });

        // Key contexts gate search/file-viewer actions on the focused
        // pane's content. Each open file pane carries its own search
        // state; "the file viewer" for action-routing purposes is the
        // one that currently has focus — never while a page hides it.
        let focused_is_file = self.workspace_page.is_none() && self.focused_file_view().is_some();
        let focused_search_open = self
            .focused_file_view()
            .is_some_and(|fv| fv.search.is_some());
        let mut key_ctx = root_key_context();
        if focused_is_file {
            key_ctx.add("FileViewer");
            if focused_search_open {
                key_ctx.add("FileViewerSearch");
            }
        }

        let workspace_root = self
            .frame_root(key_ctx, cx)
            // Search actions — context-gated via KeyBinding context strings in main.rs.
            .on_action(cx.listener(|this, _: &SaveFilePane, window, cx| {
                this.save_focused_file_pane(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FileViewerSearchOpen, window, cx| {
                if let Some(fv) = this.focused_file_view_mut() {
                    fv.search_open();
                }
                if let Some(fc) = this.focused_file_content() {
                    let fh = fc.search_input.read(cx).focus_handle(cx);
                    fh.focus(window, cx);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &FileViewerSearchNext, _window, cx| {
                this.file_view_search_next(cx);
            }))
            .on_action(cx.listener(|this, _: &FileViewerSearchPrev, _window, cx| {
                this.file_view_search_prev(cx);
            }))
            .when(self.workspace_page.is_some(), |el| {
                el.on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                    this.on_page_key_down(ev, window, cx)
                }))
            })
            // Keyboard shortcuts when the focused pane is a file viewer.
            // The per-pane Input handles its own typing; this `on_key_down`
            // owns the panel-level shortcuts (close pane, search close,
            // copy / select-all when no input is focused).
            .when(focused_is_file, |el| {
                el.on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                    let search_open = this
                        .focused_file_view()
                        .is_some_and(|fv| fv.search.is_some());
                    match ev.keystroke.key.as_str() {
                        // Escape while the search panel is open closes it +
                        // clears the query and restores pane focus.
                        // `gpui_component::Input` doesn't emit Escape via
                        // `InputEvent`, so the per-pane subscription can't
                        // see it; the panel-level handler picks it up.
                        "escape" if search_open => {
                            let pane_id = this.active_runtime().focused_pane_id;
                            if let Some(fc) = this.focused_file_content() {
                                let input = fc.search_input.clone();
                                input.update(cx, |inp, cx_state| {
                                    inp.set_value("", window, cx_state)
                                });
                            }
                            if let Some(fv) = this.focused_file_view_mut() {
                                fv.search_close();
                            }
                            this.focus_pane(pane_id, window, cx);
                            cx.notify();
                            cx.stop_propagation();
                        }
                        "escape" => {
                            this.close_focused_file_pane(window, cx);
                        }
                        "c" if ev.keystroke.modifiers.platform && !search_open => {
                            let text = this
                                .focused_file_view()
                                .map(|fv| fv.selected_text_for_copy())
                                .unwrap_or_default();
                            if !text.is_empty() {
                                cx.write_to_clipboard(ClipboardItem::new_string(text));
                            }
                        }
                        "a" if ev.keystroke.modifiers.platform && !search_open => {
                            if let Some(fv) = this.focused_file_view_mut()
                                && fv.select_all()
                            {
                                cx.notify();
                            }
                        }
                        _ => {}
                    }
                }))
            })
            // The three overlay key interceptors below all capture rather than
            // bubble: key events dispatch to the focused element first, and
            // while an overlay is open that is still the terminal — which
            // forwards every arrow and character to its PTY before the root
            // ever sees them. Capture runs root → leaf, so these intercept,
            // and each handler's `stop_propagation` keeps the keystroke out of
            // the shell.
            .when(self.overlays.palette.is_open, |el| {
                el.capture_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                    this.on_palette_key(ev, window, cx)
                }))
            })
            .when(self.overlays.lane_switcher.is_open, |el| {
                el.capture_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                    this.on_lane_switcher_key(ev, window, cx)
                }))
            })
            .when(self.overlays.flow_picker.is_open(), |el| {
                el.capture_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                    this.on_flow_picker_key(ev, window, cx)
                }))
            })
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, cx| {
                // A mouse-up released outside the window never reaches the
                // bubble-phase on_mouse_up below — it fails the root div's
                // hit-test — so any drag begun inside stays "live". The first
                // re-entry move carries pressed_button: None: treat that as
                // the missed release and settle every live drag (dock/divider
                // resize + file-view text/block selection) instead of
                // continuing it. The root move handler spans the whole window,
                // so it catches the release wherever the cursor re-enters.
                if !ev.dragging() {
                    this.end_stale_resize_drags(cx);
                    this.end_file_selection_drag(cx);
                    this.clear_pane_drop_hover(cx);
                    this.finish_tab_drag(false, cx);
                    return;
                }
                if let Some(drag) = this.docks.drag {
                    let cursor_px: f32 = match drag.position {
                        DockPosition::Left | DockPosition::Right => ev.position.x.into(),
                        DockPosition::Bottom => ev.position.y.into(),
                    };
                    this.update_dock_drag(cursor_px, window, cx);
                    return;
                }
                let Some(drag) = this.main_area.drag_state else {
                    return;
                };
                let cursor_px: f32 = match drag.direction {
                    SplitDirection::Horizontal => ev.position.x.into(),
                    SplitDirection::Vertical => ev.position.y.into(),
                };
                this.update_divider_drag(cursor_px, window, cx);
            }))
            // Bubble phase, so every `on_drop` has already had its shot and a
            // consumed drop stopped propagation (gpui `div.rs`). Reaching
            // here means the release hit no drop target at all.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _window, cx| {
                    this.end_divider_drag(cx);
                    this.end_dock_drag(cx);
                    this.end_file_selection_drag(cx);
                    this.clear_pane_drop_hover(cx);
                    this.finish_tab_drag(false, cx);
                }),
            )
            .on_action(cx.listener(Self::on_new_tab))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_close_pane))
            .on_action(cx.listener(Self::on_next_tab))
            .on_action(cx.listener(Self::on_prev_tab))
            .on_action(cx.listener(Self::on_split_right))
            .on_action(cx.listener(Self::on_split_down))
            .on_action(cx.listener(Self::on_focus_next_pane))
            .on_action(cx.listener(Self::on_focus_prev_pane))
            .on_action(cx.listener(Self::on_focus_pane_left))
            .on_action(cx.listener(Self::on_focus_pane_right))
            .on_action(cx.listener(Self::on_focus_pane_up))
            .on_action(cx.listener(Self::on_focus_pane_down))
            .on_action(cx.listener(Self::on_move_tab_left))
            .on_action(cx.listener(Self::on_move_tab_right));
        // Cmd+1..9 tab quick-switch + Cmd+Ctrl+1..9 lane quick-switch —
        // each slot is one macro line in `slot_actions.rs`.
        let workspace_root = crate::tab_slot_table!(@register_listeners cx, workspace_root);
        let workspace_root = crate::lane_slot_table!(@register_listeners cx, workspace_root);
        workspace_root
            .on_action(cx.listener(Self::on_toggle_left_dock))
            .on_action(cx.listener(Self::on_reset_left_dock_width))
            .on_action(cx.listener(Self::on_toggle_git_changes_focus))
            .on_action(cx.listener(Self::on_toggle_files_focus))
            .on_action(cx.listener(Self::on_toggle_bottom_dock))
            .on_action(cx.listener(Self::on_toggle_right_dock))
            .on_action(cx.listener(Self::on_toggle_command_palette))
            .on_action(cx.listener(Self::on_toggle_lane_switcher))
            .on_action(cx.listener(Self::on_run_flow))
            .on_action(cx.listener(Self::on_validate_flow))
            .on_action(cx.listener(Self::on_show_flow_graph))
            .on_action(cx.listener(Self::on_reload_flow_graph))
            .on_action(cx.listener(Self::on_show_left_dock_worktrees))
            .on_action(cx.listener(Self::on_show_left_dock_git))
            .on_action(cx.listener(Self::on_show_left_dock_files))
            .on_action(cx.listener(Self::on_switch_right_panel_usage))
            .on_action(cx.listener(Self::on_switch_right_panel_skills))
            .on_action(cx.listener(Self::on_switch_right_panel_tools))
            .on_action(cx.listener(Self::on_switch_right_panel_tasks))
            .on_action(cx.listener(Self::on_switch_right_panel_flows))
            .on_action(cx.listener(Self::on_new_skill))
            .on_action(cx.listener(Self::on_new_task))
            .on_action(cx.listener(Self::on_open_agent_chat))
            .on_action(cx.listener(Self::on_edit_task))
            .on_action(cx.listener(Self::on_focus_skill_search))
            .on_action(cx.listener(Self::on_invoke_skill_palette))
            .on_action(cx.listener(Self::on_refresh_git_status_action))
            .on_action(cx.listener(Self::on_commit_changes))
            .on_action(cx.listener(Self::on_commit_amend_action))
            .on_action(cx.listener(Self::on_push_changes))
            .on_action(cx.listener(Self::on_fetch_action))
            .on_action(cx.listener(Self::on_pull_action))
            .on_action(cx.listener(Self::on_files_toggle_hidden))
            .on_action(cx.listener(Self::on_files_select_next))
            .on_action(cx.listener(Self::on_files_select_prev))
            .on_action(cx.listener(Self::on_files_activate))
            .on_action(cx.listener(Self::on_files_expand))
            .on_action(cx.listener(Self::on_files_collapse))
            .on_action(cx.listener(Self::on_files_refresh))
            .on_action(cx.listener(Self::on_git_changes_select_next))
            .on_action(cx.listener(Self::on_git_changes_select_prev))
            .on_action(cx.listener(Self::on_git_changes_toggle_stage))
            .on_action(cx.listener(Self::on_git_changes_activate))
            .on_action(cx.listener(Self::on_switch_pane_account))
            .on_action(cx.listener(Self::on_open_project_config))
            .on_action(cx.listener(Self::on_install_agent_hooks))
            .on_action(cx.listener(Self::on_uninstall_agent_hooks))
            .on_action(cx.listener(Self::on_run_macro_by_shortcut))
            .on_action(cx.listener(Self::on_edit_window_title))
            .on_action(cx.listener(Self::on_open_command_history))
            .on_action(cx.listener(Self::on_close_other_tabs))
            .on_action(cx.listener(Self::on_close_tabs_to_right))
            .on_action(cx.listener(Self::on_toggle_zoom_pane))
            .on_action(cx.listener(Self::on_new_group))
            .on_action(cx.listener(Self::on_rename_active_project))
            .on_action(cx.listener(Self::on_move_active_project_to_group))
            .child(title_bar)
            .child(body_layout)
    }
}
