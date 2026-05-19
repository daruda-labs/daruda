//! GPUI rendering for Workspace.
//!
//! Two distinct UI elements live here — keep them straight (see workspace/mod.rs):
//!   • Tab bar  (top of window)   — built inline in `impl Render`.
//!                                   Identifiers: `tab_bar`, `tab_titles`, `TAB_BAR_HEIGHT`.
//!   • Pane header (per pane)     — built by `pane_header()`, only in split mode.
//!                                   Identifiers: `pane_header`, `PANE_HEADER_HEIGHT`.

use super::main_area::render_layout;

use crate::ui::theme;
use gpui::{
    ClickEvent, ClipboardItem, Context, CursorStyle, Focusable as _, IntoElement, KeyContext,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Render, SharedString, Window, div,
    prelude::*, px,
};

use gpui::KeyDownEvent;

use super::command::palette as command_palette;
use super::layout::DockPosition;
use super::layout::{BottomDockSnapshot, DockSnapshot, LeftDockSnapshot, RightDockSnapshot};
use super::main_area::file_view_pane::{CharPos, CharSelection};
use super::main_area::pane::PaneContent;
use super::main_area::pane_tree::{DIVIDER_PX, PaneLayout, SplitDirection};
use super::status_bar::{self, StatusBarData};
use super::{
    FileViewerSearchNext, FileViewerSearchOpen, FileViewerSearchPrev, NewTab, TAB_BAR_HEIGHT,
    TITLE_BAR_HEIGHT, Workspace,
};
#[allow(unused_imports)]
use super::{FocusPaneDown, FocusPaneLeft, FocusPaneRight, FocusPaneUp};

pub(super) const PANE_HEADER_HEIGHT: f32 = theme::PANE_HEADER_HEIGHT;

/// Wrap every clickable [`crate::ui::ContextMenuItem`] so the menu's
/// open state is cleared before the user handler runs. Idempotent: if a
/// handler already calls `close_context_menu` itself the second call is a
/// no-op. Keeps the workspace's `context_menu = Some(...)` backdrop from
/// outliving the action that opened a Dialog or transitioned a view.
fn wrap_items_with_close(
    items: &[crate::ui::ContextMenuItem],
    cx: &gpui::Context<Workspace>,
) -> Vec<crate::ui::ContextMenuItem> {
    use crate::ui::ContextMenuItem;
    let weak = cx.weak_entity();
    items
        .iter()
        .map(|item| match item {
            ContextMenuItem::Separator => ContextMenuItem::Separator,
            ContextMenuItem::Item {
                label,
                disabled,
                tooltip,
                on_click,
            } => {
                let weak = weak.clone();
                let inner = on_click.clone();
                ContextMenuItem::Item {
                    label: label.clone(),
                    disabled: *disabled,
                    tooltip: tooltip.clone(),
                    on_click: std::rc::Rc::new(move |ev, win, app| {
                        if let Some(w) = weak.upgrade() {
                            w.update(app, |this, cx| this.close_context_menu(cx));
                        }
                        (inner)(ev, win, app);
                    }),
                }
            }
        })
        .collect()
}

/// Builds a workspace-scoped context-menu item that:
/// 1. upgrades the weak workspace reference,
/// 2. closes the menu,
/// 3. runs `f`.
///
/// Capturing a `WeakEntity<Workspace>` (rather than `&mut Workspace`
/// directly) keeps the closure `'static` and avoids re-entrancy — the
/// action executes in a new event cycle after the current render is done.
pub(in crate::workspace) fn ws_menu_item(
    ws: gpui::WeakEntity<Workspace>,
    label: &'static str,
    disabled: bool,
    f: impl Fn(&mut Workspace, &mut gpui::Window, &mut gpui::Context<Workspace>) + 'static,
) -> crate::ui::ContextMenuItem {
    crate::ui::ContextMenuItem::new(label, move |_, win, app| {
        if let Some(w) = ws.upgrade() {
            w.update(app, |this, cx| {
                this.close_context_menu(cx);
                f(this, win, cx);
            });
        }
    })
    .disabled(disabled)
}

/// Builds a workspace-scoped context-menu item that closes the menu and
/// writes `text` to the system clipboard.
pub(in crate::workspace) fn ws_clipboard_item(
    ws: gpui::WeakEntity<Workspace>,
    label: &'static str,
    text: String,
) -> crate::ui::ContextMenuItem {
    crate::ui::ContextMenuItem::new(label, move |_, _, app| {
        if let Some(w) = ws.upgrade() {
            w.update(app, |this, cx| {
                this.close_context_menu(cx);
            });
        }
        app.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
    })
}

/// Small icon button in the tab bar for toggling docks.
/// Thin wrapper around [`crate::ui::button_toggle`] so the
/// render tree keeps reading as a local helper while the visual
/// bits live in one place.
fn dock_toggle_icon(
    id: &'static str,
    icon: &'static str,
    is_active: bool,
    cx: &gpui::App,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    crate::ui::button_toggle(id, icon, is_active, cx).on_click(on_click)
}

/// Resize handle — purely a hit target, absolutely positioned so it
/// occupies NO flex layout space. Centered on the **visible border
/// line**, not on the dock's outer edge: since the 1px dock border is
/// drawn inside the dock (e.g. `border_r_1` at `[dock_size - 1, dock_size]`
/// for the left dock), the line center sits at `dock_size - DIVIDER_PX/2`.
/// Aligning there keeps the hit zone symmetric around the line — same
/// model as `render_layout`'s pane divider, where the overlay sits on
/// top of a 1px flex-child visible line.
fn dock_resize_handle(
    position: DockPosition,
    dock_size: f32,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let hit = theme::RESIZE_HANDLE_HIT_PX;
    let half = hit / 2.0;
    let line_center_offset = dock_size - DIVIDER_PX / 2.0;
    let handle_start = line_center_offset - half;
    let (id_str, cursor) = match position {
        DockPosition::Left => ("dock-resize-left", CursorStyle::ResizeLeftRight),
        DockPosition::Right => ("dock-resize-right", CursorStyle::ResizeLeftRight),
        DockPosition::Bottom => ("dock-resize-bottom", CursorStyle::ResizeUpDown),
    };

    let mut handle = div().id(id_str).absolute().cursor(cursor);
    handle = match position {
        DockPosition::Left => handle.left(px(handle_start)).w(px(hit)).top_0().bottom_0(),
        DockPosition::Right => handle.right(px(handle_start)).w(px(hit)).top_0().bottom_0(),
        DockPosition::Bottom => handle
            .bottom(px(handle_start))
            .h(px(hit))
            .left_0()
            .right_0(),
    };

    handle.on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, ev: &MouseDownEvent, _window, cx| {
            let anchor_px: f32 = match position {
                DockPosition::Left | DockPosition::Right => ev.position.x.into(),
                DockPosition::Bottom => ev.position.y.into(),
            };
            this.begin_dock_drag(position, anchor_px, cx);
        }),
    )
}

/// Full-screen absolute overlay — click-to-dismiss hit target for floating
/// panels (palette, context menu). Chain `.on_mouse_down(...)` and `.child()`
/// to complete the pattern.
fn backdrop() -> gpui::Div {
    div().absolute().size_full().top_0().left_0()
}

/// Default alpha for the dim overlay drawn on top of inactive panes.
/// Runtime value lives on `Workspace::dim_alpha` so future theme/config
/// loading can override it without touching render code.
pub(super) const DEFAULT_INACTIVE_PANE_DIM_ALPHA: f32 = 0.35;

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.main_area.pending_resize {
            self.resize_all_tabs(window, cx);
        }

        let t = theme::current(cx);
        let title_bar_bg = t.title_bar_bg;
        let tab_bar_bg = t.tab_bar_bg;
        let tab_bar_border = t.tab_bar_border;
        let tab_active_bg = t.tab_active_bg;
        let tab_active_text = t.tab_active_text;
        let tab_inactive_bg = t.tab_inactive_bg;
        let tab_inactive_text = t.tab_inactive_text;
        let tab_inactive_hover_bg = t.tab_inactive_hover_bg;
        let muted_text = t.muted_text;
        let close_button_hover_bg = t.close_button_hover_bg;

        // Pre-collect tab bar data (no entity reads during element construction).
        // User-set label (Window > Edit Tab Title…) wins; otherwise fall
        // back to cwd basename, then PTY title — iTerm2's "Show profile
        // name → working directory" preference.
        //
        // Each entry: (index, is_active, display_label, file_abs_path, worktree_root)
        // file_abs_path / worktree_root are Some only for File panes and drive the
        // right-click "Copy File Path" / "Copy Relative Path" items.
        #[allow(clippy::type_complexity)]
        let tab_titles: Vec<(
            usize,
            bool,
            SharedString,
            Option<std::path::PathBuf>,
            Option<std::path::PathBuf>,
        )> = self
            .main_area
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                let pane = self
                    .main_area
                    .panes
                    .iter()
                    .find(|p| p.id == tab.last_focused_pane);
                let base_label = tab
                    .user_label
                    .clone()
                    .or_else(|| {
                        pane.and_then(|p| match &p.content {
                            // File panes: filename is the tab identity; the parent
                            // directory is shown in the toolbar, not the tab.
                            PaneContent::File(_) => None,
                            _ => p.display_cwd(),
                        })
                    })
                    .or_else(|| pane.map(|p| p.title()))
                    .unwrap_or_else(|| "shell".into());
                // Prefix the dirty dot (R-25) so the user can spot
                // unsaved TaskEdit panes in the tab bar at a glance.
                // Terminal / File panes always read `false` here.
                let label: SharedString = if pane.map(|p| p.tab_dirty_dot(cx)).unwrap_or(false) {
                    SharedString::from(format!(
                        "{}{}",
                        crate::surface::strings::TAB_TITLE_DIRTY_DOT,
                        base_label
                    ))
                } else {
                    base_label
                };
                let (file_path, worktree_root) = match pane.and_then(|p| match &p.content {
                    PaneContent::File(f) => Some((f.view.path.clone(), f.view.worktree_id)),
                    PaneContent::Terminal(_) | PaneContent::TaskEditPane(_) => None,
                }) {
                    Some((path, wt_id)) => {
                        let root = self
                            .active_worktrees()
                            .iter()
                            .find(|wt| wt.id == wt_id)
                            .map(|wt| wt.path.clone());
                        (Some(path), root)
                    }
                    None => (None, None),
                };
                (
                    i,
                    i == self.main_area.active_tab_index,
                    label,
                    file_path,
                    worktree_root,
                )
            })
            .collect();

        // Window title — user override (Window > Edit Window Title…) wins;
        // otherwise show `<project> · <branch>` for the active worktree
        // (active project only, no aggregate count). Welcome state
        // (no projects) leaves the title untouched.
        if let Some(label) = self.window_user_label.as_ref() {
            window.set_window_title(label.as_ref());
        } else if let Some(title) = self.window_title_label() {
            window.set_window_title(&title);
        }

        // --- Stage dock snapshots before GPUI descends into dock entities ---
        //
        // Each snapshot is a plain-data copy of the Workspace fields the
        // dock's render needs.  Written here (Context<Workspace>) so the
        // dock render closure runs inside Context<Dock> without reaching
        // back through WeakEntity<Workspace>.

        // Ensure the file tree is primed before snapshotting its state.
        let active_ref = self.active_ref();
        if !self.file_tree.file_trees.contains_key(&active_ref) {
            self.ensure_file_tree(active_ref, cx);
        }

        let left_snap = LeftDockSnapshot {
            left_dock_view: self.left_dock_view,
            active_project_name: self
                .active_project()
                .map(|p| gpui::SharedString::from(p.name.clone())),
            worktrees: self.active_worktrees().to_vec(),
            projects: {
                let mut projects: Vec<crate::workspace::layout::snap::ProjectSnapshot> = self
                    .projects
                    .iter()
                    .map(|p| crate::workspace::layout::snap::ProjectSnapshot {
                        id: p.id,
                        name: gpui::SharedString::from(p.name.clone()),
                        group_id: p.group_id,
                        color: p
                            .color
                            .as_ref()
                            .map(|c| gpui::SharedString::from(c.clone())),
                        tab_order: p.tab_order,
                        worktrees: p.worktrees.clone(),
                        last_active_worktree_id: p.last_active_worktree_id,
                    })
                    .collect();
                projects.sort_by_key(|p| p.tab_order);
                projects
            },
            groups: {
                let mut groups: Vec<crate::workspace::layout::snap::GroupSnapshot> = self
                    .groups
                    .iter()
                    .map(|g| crate::workspace::layout::snap::GroupSnapshot {
                        id: g.id,
                        name: gpui::SharedString::from(g.name.clone()),
                        color: g
                            .color
                            .as_ref()
                            .map(|c| gpui::SharedString::from(c.clone())),
                        tab_order: g.tab_order,
                        is_collapsed: g.is_collapsed,
                    })
                    .collect();
                groups.sort_by_key(|g| g.tab_order);
                groups
            },
            active: self.active,
            active_tab_count: self.main_area.tabs.len(),
            git_status_cache: self.git_status_cache.clone(),
            git_stage_in_flight: self.git_stage_in_flight,
            git_op_in_flight: self.git_op_in_flight,
            git_collapsed_dirs: self
                .git_collapsed_dirs
                .get(&self.active)
                .cloned()
                .unwrap_or_default(),
            git_changes_cursor: self.git_changes_cursor.get(&self.active).cloned(),
            git_changes_panel_focus: self.git_changes_panel_focus.clone(),
            focused_file_selection: self
                .focused_file_view()
                .map(|fv| (fv.worktree_id, fv.path.clone(), fv.staged)),
            git_changes_scroll_handle: self.git_changes_scroll_handle.clone(),
            git_commit_input: self.git_commit_input.clone(),
            files_panel_focus: self.file_tree.files_panel_focus.clone(),
            files_scroll_handle: self.file_tree.files_scroll_handle.clone(),
            files_icon_color_mode: self.file_tree.files_icon_color_mode.clone(),
            cached_visible: self.cached_or_rebuild_visible(active_ref),
            root_kind: self
                .file_tree
                .file_trees
                .get(&active_ref)
                .and_then(|t| t.entry(t.root_id))
                .map(|e| e.kind),
            // Sessions are only surfaced once the PtyTracker has
            // confirmed a live PID for them. A session_id can sit in
            // `claude_status` (hook write or jsonl tail) without
            // belonging to any process daruda owns — e.g. a stale
            // status file from a crashed run, or `claude` running
            // outside daruda in another terminal. Hiding those keeps
            // the indicator a faithful "this Workspace owns a live
            // claude here" signal rather than a "we have a status
            // record for this cwd somewhere on disk".
            //
            // Trade-off: there's a ~3 s window after `claude` starts
            // before PtyTracker's next poll lands the binding, so the
            // indicator appears slightly later than the first hook
            // event. Acceptable for the correctness it buys.
            claude_status_per_worktree: {
                let live: std::collections::HashSet<&str> = self
                    .claude
                    .pty_claude_bindings
                    .values()
                    .map(|b| b.session_id.as_str())
                    .collect();
                let mut map = std::collections::HashMap::new();
                for wt in self.active_worktrees() {
                    let live_sessions = self
                        .claude
                        .claude_status
                        .per_session_states_for_cwd(&wt.path)
                        .into_iter()
                        .filter(|(sid, _)| live.contains(sid.as_str()));
                    if let Some(state) = live_sessions.map(|(_, s)| s).max_by_key(|s| s.priority())
                    {
                        map.insert(wt.id, state);
                    }
                }
                map
            },
            claude_per_session_per_worktree: {
                let live: std::collections::HashSet<&str> = self
                    .claude
                    .pty_claude_bindings
                    .values()
                    .map(|b| b.session_id.as_str())
                    .collect();
                let mut map = std::collections::HashMap::new();
                for wt in self.active_worktrees() {
                    let sessions: Vec<_> = self
                        .claude
                        .claude_status
                        .per_session_states_for_cwd(&wt.path)
                        .into_iter()
                        .filter(|(sid, _)| live.contains(sid.as_str()))
                        .collect();
                    // Only worktrees with ≥ 2 PID-confirmed sessions
                    // get a sub-row; single-session worktrees are
                    // fully described by the leading indicator.
                    if sessions.len() >= 2 {
                        map.insert(wt.id, sessions);
                    }
                }
                map
            },
            claude_active_session_id: self
                .claude
                .pty_claude_bindings
                .get(&self.main_area.focused_pane_id)
                .map(|b| b.session_id.clone()),
            claude_install_banner_visible: self.claude.claude_status_enabled
                && !self.claude.claude_hooks_installed,
            workspace: self.left_dock.read(cx).workspace.clone(),
        };
        self.left_dock
            .update(cx, |d, _| d.snap = DockSnapshot::Left(Box::new(left_snap)));

        let bottom_snap = {
            let active_tab_id = self.panels.active_tab_id.clone();
            let tab_summaries: Vec<(daruda_store::panels::TabId, String, usize)> = {
                let mut v: Vec<_> = self
                    .panels
                    .tabs
                    .iter()
                    .map(|t| (t.order, t.id.clone(), t.name.clone(), t.widgets.len()))
                    .collect();
                v.sort_by_key(|(order, _, _, _)| *order);
                v.into_iter()
                    .map(|(_, id, name, count)| (id, name, count))
                    .collect()
            };
            let active_tab_widgets = self
                .panels
                .active_tab_id
                .as_ref()
                .and_then(|id| self.panels.tabs.iter().find(|t| &t.id == id))
                .map(|t| t.widgets.clone())
                .unwrap_or_default();
            // `shell_program` is None when neither the user nor project
            // config sets `[shell] program` — drag-drop quoting falls back
            // to Posix, which is right for daruda's macOS target where the
            // PTY inherits `$SHELL` (zsh / bash / sh).
            let shell = self
                .shell_program
                .as_deref()
                .map(crate::shell_quote::Shell::detect_from_program)
                .unwrap_or_default();
            let bottom_dock_size = self.bottom_dock.read(cx).size;
            BottomDockSnapshot {
                terminal_input_visible: self.terminal_input_visible,
                active_tab_id,
                tab_summaries,
                active_tab_widgets,
                grid_columns: self.panels_grid_columns,
                bottom_dock_size,
                terminal_input: self.terminal_input.clone(),
                shell,
                workspace: self.bottom_dock.read(cx).workspace.clone(),
            }
        };
        self.bottom_dock
            .update(cx, |d, _| d.snap = DockSnapshot::Bottom(bottom_snap));

        let claude_status_per_path = cx
            .global::<crate::agent::tasks_global::GlobalTasks>()
            .tasks
            .iter()
            .filter_map(|t| t.state.worktree_path().cloned())
            .filter_map(|p| {
                self.claude
                    .claude_status
                    .aggregate_for_cwd(&p)
                    .map(|s| (p, s))
            })
            .collect();
        // Per-session status keyed by the `session_id` so the Tasks
        // tab's row renderer can paint a `⟳ / ● / ⚠` glyph next to
        // each row's session-id badge (R-23) without dipping into
        // the workspace.
        let claude_status_per_session: std::collections::HashMap<
            String,
            daruda_claude::SessionStatus,
        > = self
            .claude
            .claude_status
            .iter()
            .map(|(sid, file)| (sid.to_string(), file.status))
            .collect();
        // Mirror the per-session failure counters too — only entries
        // that have actually accumulated failures travel into the
        // snap so an idle session with 0 failures stays absent from
        // the map (cheaper hash, no false-positive lookups).
        let tool_use_failure_counts: std::collections::HashMap<String, u32> = self
            .claude
            .tool_use_failure_counts
            .iter()
            .filter(|&(_, &n)| n > 0)
            .map(|(sid, &n)| (sid.clone(), n))
            .collect();
        let right_snap = RightDockSnapshot {
            right_dock_view: self.right_dock_view,
            workspace: self.right_dock.read(cx).workspace.clone(),
            usage: self.claude.usage.clone(),
            usage_pricing: self.claude.usage_pricing.clone(),
            plan_limits: self.claude.plan_limits.clone(),
            service_status: self.claude.service_status.clone(),
            usage_window: self.claude.usage_window,
            usage_select: self.claude.usage_select.clone(),
            skills: cx
                .global::<crate::agent::skills::SkillsState>()
                .snapshot_for(self.active_worktree_root().as_deref()),
            skill_search_input: self.skill_search_input.clone(),
            skill_search_query: self.skill_search_input.read(cx).value().to_string(),
            skill_plugin_expanded: self.skill_plugin_expanded.clone(),
            tasks: cx
                .global::<crate::agent::tasks_global::GlobalTasks>()
                .0
                .clone(),
            task_search_input: self.task_search_input.clone(),
            task_search_query: self.task_search_input.read(cx).value().to_string(),
            task_filter: self.task_filter,
            claude_status_per_path,
            claude_status_per_session,
            tool_use_failure_counts,
            now: chrono::Utc::now(),
            right_panel_scroll_handle: self.right_panel_scroll_handle.clone(),
            mcp: cx
                .global::<crate::agent::mcp::McpState>()
                .snapshot_for(self.active_worktree_root().as_deref()),
        };
        self.right_dock.update(cx, |d, _| {
            d.snap = DockSnapshot::Right(Box::new(right_snap))
        });

        // Read dock display state after staging snapshots.
        let (left_dock_open, left_dock_size) = {
            let d = self.left_dock.read(cx);
            (d.is_open, d.size)
        };
        let (bottom_dock_open, bottom_dock_size) = {
            let d = self.bottom_dock.read(cx);
            (d.is_open, d.size)
        };
        let (right_dock_open, right_dock_size) = {
            let d = self.right_dock.read(cx);
            (d.is_open, d.size)
        };

        // iTerm2-style tab bar:
        // - Each tab grows to share row width (min 80px, max 220px); text
        //   truncates via overflow_hidden + whitespace_nowrap.
        // - Number prefix matches the cmd-N hotkey for that tab.
        // - Close × is hover-only (group hover) — matches iTerm2's
        //   `tabCloseButtonsAlwaysVisible = NO` default.
        // - Middle-click on a tab closes it (iTerm2 `middleClickClosesTab`).
        // - Active tab gets a 2px bottom accent + brighter bg.
        // ── Title bar ──────────────────────────────────────
        // Traffic lights sit in the left 70px. Dock toggle
        // icons are pushed to the right. The area is
        // draggable so the user can move the window.
        let title_spacer = div().flex_1();
        let dock_toggles = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::DOCK_ICON_GROUP_GAP))
            .mr(px(theme::DOCK_ICON_GROUP_MR))
            .child(dock_toggle_icon(
                "toggle-left-dock",
                "◧",
                left_dock_open,
                cx,
                cx.listener(|this, _, window, cx| {
                    this.on_toggle_left_dock(&super::ToggleLeftDock, window, cx);
                }),
            ))
            .child(dock_toggle_icon(
                "toggle-bottom-dock",
                "⬓",
                bottom_dock_open,
                cx,
                cx.listener(|this, _, window, cx| {
                    this.on_toggle_bottom_dock(&super::ToggleBottomDock, window, cx);
                }),
            ))
            .child(dock_toggle_icon(
                "toggle-right-dock",
                "◨",
                right_dock_open,
                cx,
                cx.listener(|this, _, window, cx| {
                    this.on_toggle_right_dock(&super::ToggleRightDock, window, cx);
                }),
            ));
        let title_bar = div()
            .flex()
            .flex_row()
            .w_full()
            .h(px(TITLE_BAR_HEIGHT))
            .bg(title_bar_bg)
            .items_center()
            .child(div().flex_none().w(px(theme::TRAFFIC_LIGHT_WIDTH)))
            .child(title_spacer)
            .child(dock_toggles);

        // ── Tab bar ──────────────────────────────────────
        let tab_bar = div()
            .flex()
            .flex_row()
            .w_full()
            .h(px(TAB_BAR_HEIGHT))
            .relative()
            .bg(tab_bar_bg)
            .items_center()
            .children(tab_titles.into_iter().map(
                |(i, is_active, display, file_path, worktree_root)| {
                    let group_name = SharedString::from(format!("tab-{}", i));

                    let close_button = div()
                        .id(("tab-close", i))
                        .flex_none()
                        .w(px(theme::TAB_CLOSE_W))
                        .h(px(theme::TAB_CLOSE_W))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(theme::TAB_CLOSE_RADIUS))
                        .text_size(px(theme::TAB_CLOSE_FONT_SIZE))
                        .text_color(muted_text)
                        .invisible()
                        .group_hover(group_name.clone(), |d| d.visible())
                        .hover(move |d| d.text_color(tab_active_text).bg(close_button_hover_bg))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                this.request_close_tab(i, window, cx);
                            }),
                        )
                        .child("×");

                    div()
                        .id(("tab", i))
                        .group(group_name)
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
                        .when(is_active, |d| {
                            d.bg(tab_active_bg).text_color(tab_active_text)
                        })
                        .when(!is_active, |d| {
                            d.bg(tab_inactive_bg)
                                .text_color(tab_inactive_text)
                                .hover(move |d| d.bg(tab_inactive_hover_bg))
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                this.activate_tab(i, window, cx);
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Middle,
                            cx.listener(move |this, _, window, cx| {
                                this.request_close_tab(i, window, cx);
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, ev: &MouseDownEvent, _window, cx| {
                                use crate::surface::strings as s;
                                use crate::ui::ContextMenuItem as CItem;

                                let tab_count = this.main_area.tabs.len();
                                let ws = cx.entity().downgrade();
                                let abs_path = file_path.clone();
                                let rel_path = file_path.as_ref().and_then(|p| {
                                    worktree_root.as_ref().and_then(|root| {
                                        p.strip_prefix(root)
                                            .ok()
                                            .map(|r| r.to_string_lossy().into_owned())
                                    })
                                });
                                let abs_str =
                                    abs_path.as_ref().map(|p| p.to_string_lossy().into_owned());
                                let is_file = abs_path.is_some();
                                let is_last = i + 1 >= tab_count;

                                let mut items: Vec<CItem> = vec![
                                    ws_menu_item(
                                        ws.clone(),
                                        s::CTX_CLOSE_TAB,
                                        false,
                                        move |this, win, cx| {
                                            this.request_close_tab(i, win, cx);
                                            this.mark_dirty_and_save(cx);
                                        },
                                    ),
                                    ws_menu_item(
                                        ws.clone(),
                                        s::CTX_CLOSE_OTHER_TABS,
                                        tab_count <= 1,
                                        move |this, win, cx| {
                                            let indices: Vec<usize> =
                                                (0..this.main_area.tabs.len())
                                                    .rev()
                                                    .filter(|&j| j != i)
                                                    .collect();
                                            this.request_close_tabs_bulk(indices, win, cx);
                                            this.mark_dirty_and_save(cx);
                                        },
                                    ),
                                    ws_menu_item(
                                        ws.clone(),
                                        s::CTX_CLOSE_TABS_TO_RIGHT,
                                        is_last,
                                        move |this, win, cx| {
                                            let indices: Vec<usize> =
                                                (i + 1..this.main_area.tabs.len()).rev().collect();
                                            this.request_close_tabs_bulk(indices, win, cx);
                                            this.mark_dirty_and_save(cx);
                                        },
                                    ),
                                    CItem::separator(),
                                    ws_menu_item(
                                        ws.clone(),
                                        s::CTX_MOVE_TAB_LEFT,
                                        i == 0,
                                        move |this, _win, cx| {
                                            if i > 0 {
                                                this.move_tab(i, i - 1, cx);
                                                this.mark_dirty_and_save(cx);
                                            }
                                        },
                                    ),
                                    ws_menu_item(
                                        ws.clone(),
                                        s::CTX_MOVE_TAB_RIGHT,
                                        is_last,
                                        move |this, _win, cx| {
                                            if i + 1 < this.main_area.tabs.len() {
                                                this.move_tab(i, i + 1, cx);
                                                this.mark_dirty_and_save(cx);
                                            }
                                        },
                                    ),
                                ];

                                // Split + New Tab — terminal tabs only.
                                if !is_file {
                                    items.extend([
                                        CItem::separator(),
                                        ws_menu_item(
                                            ws.clone(),
                                            s::CTX_SPLIT_RIGHT,
                                            false,
                                            move |this, win, cx| {
                                                if this.main_area.active_tab_index != i {
                                                    this.activate_tab(i, win, cx);
                                                }
                                                this.split_focused_pane(
                                                    SplitDirection::Horizontal,
                                                    win,
                                                    cx,
                                                );
                                                this.mark_dirty_and_save(cx);
                                            },
                                        ),
                                        ws_menu_item(
                                            ws.clone(),
                                            s::CTX_SPLIT_DOWN,
                                            false,
                                            move |this, win, cx| {
                                                if this.main_area.active_tab_index != i {
                                                    this.activate_tab(i, win, cx);
                                                }
                                                this.split_focused_pane(
                                                    SplitDirection::Vertical,
                                                    win,
                                                    cx,
                                                );
                                                this.mark_dirty_and_save(cx);
                                            },
                                        ),
                                        CItem::separator(),
                                        ws_menu_item(
                                            ws.clone(),
                                            s::CTX_NEW_TAB,
                                            false,
                                            |this, win, cx| {
                                                this.add_tab(win, cx);
                                                this.mark_dirty_and_save(cx);
                                            },
                                        ),
                                    ]);
                                }

                                // File-pane-specific items
                                if is_file {
                                    items.push(CItem::separator());
                                    if let Some(abs) = abs_str {
                                        items.push(ws_clipboard_item(
                                            ws.clone(),
                                            s::CTX_COPY_FILE_PATH,
                                            abs,
                                        ));
                                    }
                                    if let Some(rel) = rel_path {
                                        items.push(ws_clipboard_item(
                                            ws.clone(),
                                            s::CTX_COPY_RELATIVE_PATH,
                                            rel,
                                        ));
                                    }
                                    items.push(ws_menu_item(
                                        ws.clone(),
                                        s::CTX_CLOSE_FILE_VIEWER,
                                        false,
                                        move |this, win, cx| {
                                            this.request_close_tab(i, win, cx);
                                            this.mark_dirty_and_save(cx);
                                        },
                                    ));
                                }

                                this.open_context_menu(ev.position, items, cx);
                            }),
                        )
                        .child({
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .child(display)
                        })
                        .child(close_button)
                },
            ))
            .child(
                div()
                    .id("new-tab-btn")
                    .px(px(theme::NEW_TAB_PAD_X))
                    .py(px(theme::NEW_TAB_PAD_Y))
                    .mx(px(theme::NEW_TAB_MARGIN_X))
                    .rounded(px(theme::NEW_TAB_RADIUS))
                    .text_size(px(theme::NEW_TAB_FONT_SIZE))
                    .cursor_pointer()
                    .text_color(muted_text)
                    .hover(move |d| d.text_color(tab_active_text).bg(tab_active_bg))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.on_new_tab(&NewTab, window, cx);
                        }),
                    )
                    .child("+"),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .bottom_0()
                    .size_full()
                    .border_b_1()
                    .border_color(tab_bar_border),
            );

        // Content area — render active tab's pane layout. The walker
        // dispatches per-pane on PaneContent: terminal panes embed
        // TerminalView, file panes embed `render_pane_file_viewer`
        // driven by their own per-pane scroll handle and find input.
        //
        // When a pane is zoomed, render only that leaf at full size.
        // has_splits = true so the pane header is visible, giving the
        // user access to the right-click Unzoom menu. The dim overlay
        // is suppressed (dim_alpha = 0.0) because is_focused is always
        // true for the sole zoomed leaf.
        let center_content =
            if let Some(tab) = self.main_area.tabs.get(self.main_area.active_tab_index) {
                let actual_has_splits = tab.layout.leaf_count() > 1;
                if let Some(zoomed_id) = self.main_area.zoomed_pane_id {
                    if tab.layout.pane_ids().contains(&zoomed_id) {
                        let leaf = PaneLayout::Pane(zoomed_id);
                        render_layout(
                            &leaf,
                            &self.main_area.panes,
                            zoomed_id,
                            true,
                            0.0,
                            SharedString::from(self.font_family.clone()),
                            None,
                            cx,
                        )
                    } else {
                        render_layout(
                            &tab.layout,
                            &self.main_area.panes,
                            self.main_area.focused_pane_id,
                            actual_has_splits,
                            self.dim_alpha,
                            SharedString::from(self.font_family.clone()),
                            self.main_area.zoomed_pane_id,
                            cx,
                        )
                    }
                } else {
                    render_layout(
                        &tab.layout,
                        &self.main_area.panes,
                        self.main_area.focused_pane_id,
                        actual_has_splits,
                        self.dim_alpha,
                        SharedString::from(self.font_family.clone()),
                        None,
                        cx,
                    )
                }
            } else {
                div().flex_1().w_full().into_any_element()
            };

        // BodyLayout: [LeftDock] [MainArea] [RightDock]
        // Resize handles are absolutely positioned overlays centered
        // on each dock's border (see `dock_resize_handle`) — they don't
        // consume flex space, so toggling docks doesn't reflow layout.
        let pane_area = div().flex_1().w_full().flex().child(center_content);
        let main_area = div()
            .flex_1()
            .flex()
            .flex_col()
            .relative()
            .overflow_hidden()
            .child(tab_bar)
            .child(pane_area)
            .when(bottom_dock_open, |el| el.child(self.bottom_dock.clone()))
            .when(bottom_dock_open, |el| {
                el.child(dock_resize_handle(
                    DockPosition::Bottom,
                    bottom_dock_size,
                    cx,
                ))
            });

        let body_layout = div()
            .flex_1()
            .flex()
            .flex_row()
            .relative()
            .overflow_hidden()
            .when(left_dock_open, |el| el.child(self.left_dock.clone()))
            .child(main_area)
            .when(right_dock_open, |el| el.child(self.right_dock.clone()))
            .when(left_dock_open, |el| {
                el.child(dock_resize_handle(DockPosition::Left, left_dock_size, cx))
            })
            .when(right_dock_open, |el| {
                el.child(dock_resize_handle(DockPosition::Right, right_dock_size, cx))
            });

        // Status bar
        let focused_pane = self
            .main_area
            .panes
            .iter()
            .find(|p| p.id == self.main_area.focused_pane_id);
        // Cheap stat per render — the path is a single file under the
        // user config dir and renders only fire on `cx.notify()`, not
        // in a tight loop.
        let has_project_config = self
            .active_project()
            .and_then(|p| daruda_config::project_config_path(&p.root))
            .is_some_and(|path: std::path::PathBuf| path.exists());
        let status_data = StatusBarData {
            project_branch: self.active_project_branch_label().map(Into::into),
            is_detached: matches!(self.active_branch_status(), super::BranchStatus::Detached),
            title: focused_pane
                .map(|p| p.title())
                .unwrap_or_else(|| "shell".into()),
            error: self.last_error.clone(),
            has_project_config,
        };
        let status_bar = status_bar::StatusBar(status_data);

        // Key contexts gate search/file-viewer actions on the focused
        // pane's content. Each open file pane carries its own search
        // state; "the file viewer" for action-routing purposes is the
        // one that currently has focus.
        let focused_is_file = self.focused_file_view().is_some();
        let focused_search_open = self
            .focused_file_view()
            .is_some_and(|fv| fv.search.is_some());
        let mut key_ctx = KeyContext::default();
        key_ctx.add("Workspace");
        if focused_is_file {
            key_ctx.add("FileViewer");
            if focused_search_open {
                key_ctx.add("FileViewerSearch");
            }
        }

        let workspace_root = div()
            // Toast overlay (and other absolute-positioned children added
            // later) anchor inside the workspace, not the entire window.
            .relative()
            .key_context(key_ctx)
            .track_focus(&self.focus_handle)
            // Search actions — context-gated via KeyBinding context strings in main.rs.
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
                if let Some(fv) = this.focused_file_view_mut() {
                    fv.search_next_match();
                }
                this.scroll_file_viewer_to_focused_match();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FileViewerSearchPrev, _window, cx| {
                if let Some(fv) = this.focused_file_view_mut() {
                    fv.search_prev_match();
                }
                this.scroll_file_viewer_to_focused_match();
                cx.notify();
            }))
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
                            let pane_id = this.main_area.focused_pane_id;
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
                            let n = this
                                .focused_file_view()
                                .map(|fv| fv.visible_row_count())
                                .unwrap_or(0);
                            if n > 0
                                && let Some(fv) = this.focused_file_view_mut()
                            {
                                fv.char_selection = Some(CharSelection {
                                    anchor: CharPos { row: 0, byte: 0 },
                                    active: CharPos {
                                        row: n - 1,
                                        byte: usize::MAX,
                                    },
                                });
                                cx.notify();
                            }
                        }
                        _ => {}
                    }
                }))
            })
            // Intercept key events when the command palette is open.
            .when(self.command_palette.is_open, |el| {
                el.on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                    if !this.command_palette.is_open {
                        return;
                    }
                    let key = ev.keystroke.key.as_str();
                    match key {
                        "escape" => {
                            this.command_palette.close();
                            cx.notify();
                        }
                        "enter" => {
                            this.execute_palette_action(window, cx);
                        }
                        "up" => {
                            this.command_palette.move_up();
                            cx.notify();
                        }
                        "down" => {
                            let max = this.command_palette.filtered_entries().len();
                            this.command_palette.move_down(max);
                            cx.notify();
                        }
                        "backspace" => {
                            this.command_palette.backspace();
                            cx.notify();
                        }
                        _ => {
                            if let Some(ch) = ev
                                .keystroke
                                .key_char
                                .as_deref()
                                .and_then(|s| s.chars().next())
                                && (ch.is_ascii_graphic() || ch == ' ')
                            {
                                this.command_palette.append(ch);
                                cx.notify();
                            }
                        }
                    }
                }))
            })
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, cx| {
                if let Some(drag) = this.dock_drag {
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
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _window, cx| {
                    this.end_divider_drag(cx);
                    this.end_dock_drag(cx);
                    if let Some(fv) = this.focused_file_view_mut()
                        && fv.is_drag_selecting
                    {
                        fv.is_drag_selecting = false;
                        cx.notify();
                    }
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
        // Cmd+1..9 tab quick-switch + Cmd+Ctrl+1..9 worktree quick-switch —
        // each slot is one macro line in `slot_actions.rs`.
        let workspace_root = crate::tab_slot_table!(@register_listeners cx, workspace_root);
        let workspace_root = crate::worktree_slot_table!(@register_listeners cx, workspace_root);
        workspace_root
            .on_action(cx.listener(Self::on_toggle_left_dock))
            .on_action(cx.listener(Self::on_toggle_bottom_dock))
            .on_action(cx.listener(Self::on_toggle_right_dock))
            .on_action(cx.listener(Self::on_toggle_command_palette))
            .on_action(cx.listener(Self::on_show_left_dock_worktrees))
            .on_action(cx.listener(Self::on_show_left_dock_git))
            .on_action(cx.listener(Self::on_show_left_dock_files))
            .on_action(cx.listener(Self::on_switch_right_panel_usage))
            .on_action(cx.listener(Self::on_switch_right_panel_skills))
            .on_action(cx.listener(Self::on_switch_right_panel_tools))
            .on_action(cx.listener(Self::on_switch_right_panel_tasks))
            .on_action(cx.listener(Self::on_new_skill))
            .on_action(cx.listener(Self::on_new_task))
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
            .on_action(cx.listener(Self::on_open_settings))
            .on_action(cx.listener(Self::on_open_project_config))
            .on_action(cx.listener(Self::on_install_claude_hooks))
            .on_action(cx.listener(Self::on_uninstall_claude_hooks))
            .on_action(cx.listener(Self::on_run_macro_by_shortcut))
            .on_action(cx.listener(Self::on_minimize_window))
            .on_action(cx.listener(Self::on_zoom_window))
            .on_action(cx.listener(Self::on_toggle_full_screen))
            .on_action(cx.listener(Self::on_edit_window_title))
            .on_action(cx.listener(Self::on_open_command_history))
            .on_action(cx.listener(Self::on_close_other_tabs))
            .on_action(cx.listener(Self::on_close_tabs_to_right))
            .on_action(cx.listener(Self::on_toggle_zoom_pane))
            .on_action(cx.listener(Self::on_new_group))
            .on_action(cx.listener(Self::on_rename_active_project))
            .on_action(cx.listener(Self::on_move_active_project_to_group))
            .size_full()
            .flex()
            .flex_col()
            .child(title_bar)
            .child(body_layout)
            .child(status_bar)
            // Toast overlay paints last so it floats above the status
            // bar. ToastLayer owns its queue, expiry sweep, and render;
            // it notifies only itself when toasts change, sparing the
            // full Workspace repaint.
            .child(self.toast_layer.clone())
            .child(command_palette::CommandPaletteOverlay::new(
                self.command_palette.clone(),
                cx.listener(|this, _, _, cx| {
                    this.command_palette.close();
                    cx.notify();
                }),
            ))
            // Context menu overlay — backdrop dismisses on click-outside;
            // the ContextMenu widget itself stops propagation on item click.
            // Each item's on_click is wrapped so the menu state is cleared
            // before the user handler runs. Without this, a handler that
            // opens a Dialog (e.g. discard / amend confirms) leaves the
            // backdrop layered behind the Dialog and steals later clicks.
            .when_some(
                self.main_area
                    .context_menu
                    .as_ref()
                    .map(|m| (m.position, m.corner, wrap_items_with_close(&m.items, cx))),
                |el, (position, corner, items)| {
                    // Backdrop is `size_full()` mounted on the workspace
                    // root, which fills the entire window. `position`
                    // is window-local (`ClickEvent::position()`), so a
                    // `BottomRight` anchor must convert against the
                    // window viewport — not `last_viewport`, which
                    // tracks only the pane area (window minus open
                    // docks) and would offset the menu by the dock
                    // sizes.
                    let parent_size = window.viewport_size();
                    el.child(
                        backdrop()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.close_context_menu(cx);
                                }),
                            )
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(|this, _, _, cx| {
                                    this.close_context_menu(cx);
                                }),
                            )
                            .child(
                                crate::ui::ContextMenu::new("workspace-ctx-menu", position)
                                    .anchor(corner, parent_size)
                                    .items(items),
                            ),
                    )
                },
            )
            // gpui_component overlay layers. The window root is
            // `gpui_component::Root` (Phase 3), but `Root::render` only
            // renders the inner view — Dialog/Sheet/Notification layers
            // must be rendered by the inner view explicitly. Without
            // these, `window.open_dialog(...)` registers a dialog into
            // `Root.active_dialogs` but nothing ever paints it. All
            // daruda modals route through these layers post Phase 4.d.
            .children(gpui_component::Root::render_sheet_layer(window, cx))
            .children(gpui_component::Root::render_dialog_layer(window, cx))
            .children(gpui_component::Root::render_notification_layer(window, cx))
    }
}
