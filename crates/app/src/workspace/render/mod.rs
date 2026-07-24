//! GPUI rendering for Workspace.
//!
//! Two distinct UI elements live here — keep them straight (see workspace/mod.rs):
//!   • Tab bar  (top of window)   — built inline in `impl Render`.
//!                                   Identifiers: `tab_bar`, `tab_titles`, `TAB_BAR_HEIGHT`.
//!   • Pane header (per pane)     — built by `pane_header()`, only in split mode.
//!                                   Identifiers: `pane_header`, `PANE_HEADER_HEIGHT`.

use crate::ui::theme;
use crate::ui::{
    ButtonVariants as _, DropdownMenu as _, PopupMenu, PopupMenuItem, button, menu_builder,
};
use gpui::{
    ClickEvent, ClipboardItem, Context, CursorStyle, DragMoveEvent, Focusable as _, IntoElement,
    KeyContext, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Render, SharedString,
    Window, div, prelude::*, px,
};

use gpui::KeyDownEvent;

use super::command::lane_switcher;
use super::command::palette as command_palette;
use super::layout::DockPosition;
use super::layout::DockSnapshot;
use super::main_area::pane::PaneContent;
use super::main_area::pane_drag_ops::PaneHeaderDrag;
use super::main_area::pane_tree::{DIVIDER_PX, SplitDirection};
use super::main_area::tab_drag_ops::{TabDrag, TabDragGhost};
use super::main_area::tab_ops::NewPaneKind;
use super::status_bar::{self, StatusBarData};
use super::{
    FileViewerSearchNext, FileViewerSearchOpen, FileViewerSearchPrev, SaveFilePane, TAB_BAR_HEIGHT,
    TITLE_BAR_HEIGHT, Workspace,
};
#[allow(unused_imports)]
use super::{FocusPaneDown, FocusPaneLeft, FocusPaneRight, FocusPaneUp};

mod center;
mod snapshots;

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
    label: impl Into<gpui::SharedString>,
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
    label: impl Into<gpui::SharedString>,
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

/// `true` when the `+` menu lists agent entries flat; `false` when it should
/// fold them into a `New Agent Chat` submenu (too many to list inline).
fn agent_menu_is_flat(agent_count: usize) -> bool {
    agent_count <= crate::ui::theme::AGENT_MENU_FLAT_MAX
}

/// Whether a `+`-menu agent entry should be disabled: the agent's launch
/// needs a remote working directory substituted in (see
/// [`daruda_config::AgentLaunch::needs_remote_cwd`]) but the active lane
/// has no `remote_cwd` set — there is nothing to substitute the remote path
/// with, so the entry is pre-disabled rather than left to fail after
/// the user picks it (`resolve_new_pane_cwd`'s error path remains a fallback
/// for call sites that bypass this menu, e.g. programmatic pane creation).
fn agent_menu_entry_disabled(needs_remote_cwd: bool, lane_has_remote_cwd: bool) -> bool {
    needs_remote_cwd && !lane_has_remote_cwd
}

/// Label for a `+`-menu agent entry, appending the disabled-reason suffix
/// when [`agent_menu_entry_disabled`] is true (there is no tooltip API on
/// `PopupMenuItem`, so the reason has to live in the label text itself).
fn agent_menu_entry_label(base_label: String, disabled: bool) -> String {
    if disabled {
        format!(
            "{base_label}{}",
            crate::surface::strings::agent_needs_remote_cwd_suffix()
        )
    } else {
        base_label
    }
}

/// Build the `+` tab-add dropdown: New Terminal, then one agent-chat entry per
/// configured agent — flat when `agents.len() <= AGENT_MENU_FLAT_MAX`, else
/// folded into a `New Agent Chat` submenu. All handlers dispatch into
/// `Workspace` (one-way data flow) and wrap in `mutate_durable_in` so the new
/// tab is persisted. `agents` is `(id, name, needs_remote_cwd)`;
/// `lane_has_remote_cwd` is the active lane's `remote_cwd.is_some()` at
/// snapshot time — together they decide, per entry, whether
/// [`agent_menu_entry_disabled`] disables it.
fn build_new_tab_menu(
    menu: PopupMenu,
    ws: &gpui::WeakEntity<Workspace>,
    agents: &[(String, String, bool)],
    lane_has_remote_cwd: bool,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<PopupMenu>,
) -> PopupMenu {
    // New Terminal.
    let menu = {
        let ws = ws.clone();
        menu.item(
            PopupMenuItem::new(crate::surface::strings::ctx_new_terminal()).on_click(
                move |_, window, app| {
                    if let Some(w) = ws.upgrade() {
                        w.update(app, |this, cx| {
                            this.mutate_durable_in(window, cx, |ws, w, cx| ws.add_tab(w, cx));
                        });
                    }
                },
            ),
        )
    }
    .separator();

    if agent_menu_is_flat(agents.len()) {
        // Flat: New {name} Chat per agent.
        agents.iter().fold(menu, |m, (id, name, needs_remote_cwd)| {
            let ws = ws.clone();
            let agent_id = id.clone();
            let disabled = agent_menu_entry_disabled(*needs_remote_cwd, lane_has_remote_cwd);
            let label = agent_menu_entry_label(
                crate::surface::strings::new_agent_chat_named(name),
                disabled,
            );
            m.item(
                PopupMenuItem::new(label)
                    .disabled(disabled)
                    .on_click(move |_, window, app| {
                        if let Some(w) = ws.upgrade() {
                            let agent_id = agent_id.clone();
                            w.update(app, |this, cx| {
                                this.mutate_durable_in(window, cx, |ws, w, cx| {
                                    ws.open_agent_chat_pane_with_agent(agent_id, w, cx)
                                });
                            });
                        }
                    }),
            )
        })
    } else {
        // Submenu: New Agent Chat ▸ { agent display name per item }.
        let agents: Vec<(String, String, bool)> = agents.to_vec();
        let ws = ws.clone();
        menu.submenu(
            crate::surface::strings::ctx_new_agent_chat(),
            window,
            cx,
            move |sub, _w, _c| {
                agents.iter().fold(sub, |m, (id, name, needs_remote_cwd)| {
                    let ws = ws.clone();
                    let agent_id = id.clone();
                    let disabled =
                        agent_menu_entry_disabled(*needs_remote_cwd, lane_has_remote_cwd);
                    let label = agent_menu_entry_label(name.clone(), disabled);
                    m.item(
                        PopupMenuItem::new(gpui::SharedString::from(label))
                            .disabled(disabled)
                            .on_click(move |_, window, app| {
                                if let Some(w) = ws.upgrade() {
                                    let agent_id = agent_id.clone();
                                    w.update(app, |this, cx| {
                                        this.mutate_durable_in(window, cx, |ws, w, cx| {
                                            ws.open_agent_chat_pane_with_agent(agent_id, w, cx)
                                        });
                                    });
                                }
                            }),
                    )
                })
            },
        )
    }
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

/// Centered main-area placeholder shown when the active lane's root
/// directory is inaccessible (Missing / AccessDenied). The render gate
/// in `center_content` keys off `availability` before any tab lookup,
/// so this is shown whenever the active lane is non-`Present` — whether
/// its pane spawn was suppressed (empty `tabs`) or its runtime entry
/// still carries tabs from when the lane was last `Present`. It fills the center
/// with the state message and a Remove affordance. The Remove button is
/// a one-line dispatch into `request_remove_inaccessible_active`
/// (one-way data flow).
/// Amount each inactive split pane's terminal colors blend toward
/// mid-gray (iTerm2's default dim, `colorDimmedBy:0.4`). Pushed onto the
/// pane's `TerminalView` by `refresh_pane_dimming`; alpha is preserved so
/// a transparent background stays equally transparent, just duller.
pub(in crate::workspace) const INACTIVE_PANE_DIM_AMOUNT: f32 = 0.4;

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.main_area.pending_resize {
            self.resize_all_tabs(window, cx);
        }

        let t = theme::current(cx);
        let dark = t.is_dark();
        let title_bar_bg = t.title_bar_bg;
        // These tab-strip slots still read raw consts (the deferred Phase-3
        // migration tail); pick light-aware values so the tab bar doesn't
        // render dark with white text on the light theme.
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
        // A File/diff pane renders on the editor surface, not canvas. The
        // active tab is meant to read as continuous with the content below
        // it, so a file tab's active background must match that surface —
        // otherwise it shows as a darker pure-black notch over the lifted
        // editor body. Terminal tabs stay on `tab_active_bg` (canvas).
        let tab_active_file_bg = t.file_viewer_bg;
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
        // User-set label (Window > Edit Tab Title…) wins; otherwise fall
        // back to cwd basename, then PTY title — iTerm2's "Show profile
        // name → working directory" preference.
        //
        // Each entry: (index, tab_id, is_active, display_label, file_abs_path, worktree_root)
        // tab_id is the stable TabEntry id (drag payload identity, survives
        // reorder). file_abs_path / worktree_root are Some only for File
        // panes and drive the right-click "Copy File Path" / "Copy Relative
        // Path" items.
        #[allow(clippy::type_complexity)]
        let tab_titles: Vec<(
            usize,
            u64,
            bool,
            SharedString,
            Option<std::path::PathBuf>,
            Option<std::path::PathBuf>,
        )> = self
            .active_runtime()
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                let pane = self
                    .active_runtime()
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
                // Prefix the dirty dot so the user can spot unsaved
                // TaskEdit panes in the tab bar at a glance. Terminal /
                // File panes always read `false` here.
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
                    PaneContent::File(f) => Some((f.view.path.clone(), f.view.lane_id)),
                    PaneContent::Terminal(_)
                    | PaneContent::TaskEditPane(_)
                    | PaneContent::AgentChat(_) => None,
                }) {
                    Some((path, wt_id)) => {
                        let root = self
                            .active_lanes()
                            .iter()
                            .find(|wt| wt.id == wt_id)
                            .map(|wt| wt.path.clone());
                        (Some(path), root)
                    }
                    None => (None, None),
                };
                (
                    i,
                    tab.id,
                    i == self.active_runtime().active_tab_index,
                    label,
                    file_path,
                    worktree_root,
                )
            })
            .collect();

        // Window title — user override (Window > Edit Window Title…) wins;
        // otherwise show `<project> · <branch>` for the active lane
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

        // — Build dock snapshots ————————————————————————————————————————
        let left_snap = self.prepare_left_dock_snapshot(cx);
        let bottom_snap = self.prepare_bottom_dock_snapshot(cx);
        let right_snap = self.prepare_right_dock_snapshot(cx);

        // — Publish snapshots to docks ————————————————————————————————
        // Left dock is wrapped in `.cached()` too (see `body` below), so it
        // is marked dirty only when its snapshot content actually changes —
        // an absent / non-Left prior snapshot counts as changed. This is the
        // sole left-dock invalidation path: any workspace render re-stages
        // the snapshot and `content_differs` decides whether to dirty the
        // dock, so source mutations only need a workspace `cx.notify()` (no
        // manual per-site `notify_left_dock()`). The lone exception is the
        // status pulse, which advances badge animation frames not present in
        // the snapshot and so keeps its explicit notify. Per Pitfall #10.
        self.left_dock.update(cx, |d, cx| {
            let changed =
                !matches!(&d.snap, DockSnapshot::Left(old) if !left_snap.content_differs(old));
            if changed {
                d.snap = DockSnapshot::Left(Box::new(left_snap));
                cx.notify();
            }
        });
        // Bottom dock is wrapped in `.cached()` (see `body`/`main_area`
        // below), so it must be marked dirty only when its snapshot
        // content actually changes — otherwise the cached view shows
        // stale data, and an unconditional notify would defeat the cache
        // by repainting on every 250 ms status-pulse tick (which leaves
        // this snapshot identical). Per root CLAUDE.md Pitfall #10.
        self.bottom_dock.update(cx, |d, cx| {
            let unchanged = matches!(&d.snap, DockSnapshot::Bottom(old) if **old == bottom_snap);
            if !unchanged {
                d.snap = DockSnapshot::Bottom(Box::new(bottom_snap));
                cx.notify();
            }
        });
        self.right_dock.update(cx, |d, cx| {
            let changed =
                !matches!(&d.snap, DockSnapshot::Right(old) if !right_snap.content_differs(old));
            if changed {
                d.snap = DockSnapshot::Right(Box::new(right_snap));
                cx.notify();
            }
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
        // Reorder-insertion indicator: `Some(k)` means "insert the dragged
        // tab before slot k" (k == tab_titles.len() means "at the end").
        // Rendered as a border on the adjacent cell (or the "+" button for
        // the end slot) rather than an absolute-positioned overlay, so it
        // never has to re-derive the tab bar's flex-computed x offsets.
        let tab_reorder_preview = self.main_area.tab_reorder_preview;
        let tab_count = tab_titles.len();
        let tab_bar = div()
            .flex()
            .flex_row()
            .w_full()
            .h(px(TAB_BAR_HEIGHT))
            .relative()
            .bg(tab_bar_bg)
            .items_center()
            .children(tab_titles.into_iter().map(
                |(i, tab_id, is_active, display, file_path, worktree_root)| {
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
                                tab_active_file_bg
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

                                let tab_count = this.active_runtime().tabs.len();
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
                                        s::ctx_close_tab(),
                                        false,
                                        move |this, win, cx| {
                                            this.mutate_durable_in(win, cx, |ws, win, cx| {
                                                ws.request_close_tab(i, win, cx);
                                            });
                                        },
                                    ),
                                    ws_menu_item(
                                        ws.clone(),
                                        s::ctx_close_other_tabs(),
                                        tab_count <= 1,
                                        move |this, win, cx| {
                                            this.mutate_durable_in(win, cx, |ws, win, cx| {
                                                let indices: Vec<usize> =
                                                    (0..ws.active_runtime().tabs.len())
                                                        .rev()
                                                        .filter(|&j| j != i)
                                                        .collect();
                                                ws.request_close_tabs_bulk(indices, win, cx);
                                            });
                                        },
                                    ),
                                    ws_menu_item(
                                        ws.clone(),
                                        s::ctx_close_tabs_to_right(),
                                        is_last,
                                        move |this, win, cx| {
                                            this.mutate_durable_in(win, cx, |ws, win, cx| {
                                                let indices: Vec<usize> = (i + 1
                                                    ..ws.active_runtime().tabs.len())
                                                    .rev()
                                                    .collect();
                                                ws.request_close_tabs_bulk(indices, win, cx);
                                            });
                                        },
                                    ),
                                    CItem::separator(),
                                    ws_menu_item(
                                        ws.clone(),
                                        s::ctx_move_tab_left(),
                                        i == 0,
                                        move |this, _win, cx| {
                                            if i > 0 {
                                                this.mutate_durable(cx, |ws, cx| {
                                                    ws.move_tab(i, i - 1, cx);
                                                });
                                            }
                                        },
                                    ),
                                    ws_menu_item(
                                        ws.clone(),
                                        s::ctx_move_tab_right(),
                                        is_last,
                                        move |this, _win, cx| {
                                            if i + 1 < this.active_runtime().tabs.len() {
                                                this.mutate_durable(cx, |ws, cx| {
                                                    ws.move_tab(i, i + 1, cx);
                                                });
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
                                            s::ctx_split_terminal_horizontal(),
                                            false,
                                            move |this, win, cx| {
                                                this.mutate_durable_in(win, cx, |ws, win, cx| {
                                                    if ws.active_runtime().active_tab_index != i {
                                                        ws.activate_tab(i, win, cx);
                                                    }
                                                    ws.split_focused_pane_kind(
                                                        NewPaneKind::Terminal,
                                                        SplitDirection::Horizontal,
                                                        win,
                                                        cx,
                                                    );
                                                });
                                            },
                                        ),
                                        ws_menu_item(
                                            ws.clone(),
                                            s::ctx_split_terminal_vertical(),
                                            false,
                                            move |this, win, cx| {
                                                this.mutate_durable_in(win, cx, |ws, win, cx| {
                                                    if ws.active_runtime().active_tab_index != i {
                                                        ws.activate_tab(i, win, cx);
                                                    }
                                                    ws.split_focused_pane_kind(
                                                        NewPaneKind::Terminal,
                                                        SplitDirection::Vertical,
                                                        win,
                                                        cx,
                                                    );
                                                });
                                            },
                                        ),
                                        CItem::separator(),
                                        ws_menu_item(
                                            ws.clone(),
                                            s::ctx_split_agent_chat_horizontal(),
                                            false,
                                            move |this, win, cx| {
                                                this.mutate_durable_in(win, cx, |ws, win, cx| {
                                                    if ws.active_runtime().active_tab_index != i {
                                                        ws.activate_tab(i, win, cx);
                                                    }
                                                    ws.split_focused_pane_kind(
                                                        NewPaneKind::AgentChat,
                                                        SplitDirection::Horizontal,
                                                        win,
                                                        cx,
                                                    );
                                                });
                                            },
                                        ),
                                        ws_menu_item(
                                            ws.clone(),
                                            s::ctx_split_agent_chat_vertical(),
                                            false,
                                            move |this, win, cx| {
                                                this.mutate_durable_in(win, cx, |ws, win, cx| {
                                                    if ws.active_runtime().active_tab_index != i {
                                                        ws.activate_tab(i, win, cx);
                                                    }
                                                    ws.split_focused_pane_kind(
                                                        NewPaneKind::AgentChat,
                                                        SplitDirection::Vertical,
                                                        win,
                                                        cx,
                                                    );
                                                });
                                            },
                                        ),
                                        CItem::separator(),
                                        ws_menu_item(
                                            ws.clone(),
                                            crate::surface::strings::ctx_new_tab(),
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
                                    items.push(CItem::separator());
                                    if let Some(abs) = abs_str {
                                        items.push(ws_clipboard_item(
                                            ws.clone(),
                                            s::ctx_copy_file_path(),
                                            abs,
                                        ));
                                    }
                                    if let Some(rel) = rel_path {
                                        items.push(ws_clipboard_item(
                                            ws.clone(),
                                            s::ctx_copy_relative_path(),
                                            rel,
                                        ));
                                    }
                                    items.push(ws_menu_item(
                                        ws.clone(),
                                        s::ctx_close_file_viewer(),
                                        false,
                                        move |this, win, cx| {
                                            this.mutate_durable_in(win, cx, |ws, win, cx| {
                                                ws.request_close_tab(i, win, cx);
                                            });
                                        },
                                    ));
                                }

                                this.open_context_menu(ev.position, items, cx);
                            }),
                        )
                        .on_drag(
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
                        .on_drag_move::<TabDrag>(cx.listener(
                            move |this, event: &DragMoveEvent<TabDrag>, window, cx| {
                                this.update_tab_drag_from_move(tab_id, i, event, window, cx);
                            },
                        ))
                        .on_drop::<TabDrag>(cx.listener(move |this, d: &TabDrag, window, cx| {
                            this.drop_tab_onto_bar(d.tab_id, window, cx);
                        }))
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
            .child({
                // "+" opens a dropdown so a new tab can be a terminal or an Agent
                // chat pane under a chosen agent. Left-click dropdowns attach to a
                // Button (`impl DropdownMenu for Button`), so this is a ghost button
                // sized to match the tab bar via the NEW_TAB_* metrics, not a div.
                let ws = cx.entity().downgrade();
                // Snapshot the catalog as owned (id, name, needs_remote_cwd)
                // triples for the 'static menu closure, plus whether the
                // active lane has a remote_cwd to disable remote-only agent
                // entries against.
                let agents: Vec<(String, String, bool)> = self
                    .agents
                    .iter()
                    .map(|a| (a.id.clone(), a.name.clone(), a.launch.needs_remote_cwd()))
                    .collect();
                let lane_has_remote_cwd = self
                    .active_lane()
                    .is_some_and(|lane| lane.remote_cwd.is_some());
                button("new-tab-btn", "+")
                    .ghost()
                    .px(px(theme::NEW_TAB_PAD_X))
                    .py(px(theme::NEW_TAB_PAD_Y))
                    .mx(px(theme::NEW_TAB_MARGIN_X))
                    .rounded(px(theme::NEW_TAB_RADIUS))
                    .text_size(px(theme::NEW_TAB_FONT_SIZE))
                    // Reorder-insertion indicator for "drop at the very
                    // end" — the counterpart of the per-cell border above
                    // when `tab_reorder_preview` points past the last tab.
                    .when(tab_reorder_preview == Some(tab_count), |d| {
                        d.border_l_2().border_color(tab_insertion_line_color)
                    })
                    .dropdown_menu(menu_builder(move |menu, window, cx| {
                        build_new_tab_menu(menu, &ws, &agents, lane_has_remote_cwd, window, cx)
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
            // indicator renders). A per-cell `on_drop::<TabDrag>` handling
            // the drop first calls `cx.stop_propagation()`, so this
            // container-level listener only runs when no cell caught it —
            // without it, that drop would silently no-op and leave
            // `tab_reorder_preview` / `tab_hover_switch` stuck showing a
            // stale indicator (only an outside-window release is caught by
            // the root `clear_tab_drag_state` call).
            .on_drop::<TabDrag>(cx.listener(|this, d: &TabDrag, window, cx| {
                this.drop_tab_onto_bar(d.tab_id, window, cx);
            }));

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
            .child(tab_bar)
            .child(pane_area)
            // `.cached()`: when the bottom dock isn't notified (its
            // snapshot was staged unchanged above), GPUI recycles its
            // previous layout + paint instead of re-rendering the macro
            // grid / terminal input on every parent repaint. The
            // self-notifying terminal input entity inside still repaints
            // on its own edits (Pitfall #10).
            .when(bottom_dock_open, |el| {
                el.child(
                    gpui::AnyView::from(self.bottom_dock.clone()).cached(
                        gpui::StyleRefinement::default()
                            .w_full()
                            .h(gpui::px(bottom_dock_size)),
                    ),
                )
            })
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
            .when(left_dock_open, |el| {
                el.child(
                    gpui::AnyView::from(self.left_dock.clone()).cached(
                        gpui::StyleRefinement::default()
                            .h_full()
                            .w(gpui::px(left_dock_size)),
                    ),
                )
            })
            .child(main_area)
            // `.cached()`: staging diffs `RightDockSnapshot`
            // (`content_differs`) and repaints only on real change, so a
            // workspace `cx.notify()` suffices. Exceptions keeping an
            // explicit `notify_right_dock`: the status pulse and task-live
            // tick, whose changes (animation, `now`) aren't in the snapshot.
            .when(right_dock_open, |el| {
                el.child(
                    gpui::AnyView::from(self.right_dock.clone()).cached(
                        gpui::StyleRefinement::default()
                            .h_full()
                            .w(gpui::px(right_dock_size)),
                    ),
                )
            })
            .when(left_dock_open, |el| {
                el.child(dock_resize_handle(DockPosition::Left, left_dock_size, cx))
            })
            .when(right_dock_open, |el| {
                el.child(dock_resize_handle(DockPosition::Right, right_dock_size, cx))
            });

        // Status bar. Resolve the focused pane's title eagerly into an owned
        // value so the `active_runtime()` borrow doesn't span the
        // `cached_project_config` write below (it borrows all of `self`).
        let focused_title = self
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == self.active_runtime().focused_pane_id)
            .map(|p| p.title())
            .unwrap_or_else(|| "shell".into());
        // Same focused-pane lookup for the account slot: Terminal/AgentChat
        // panes track an `account_id` override (`None` = provider default);
        // File/TaskEdit panes don't track an account at all, so the slot
        // is hidden (`None`) rather than showing a misleading "System".
        let focused_pane_id = self.active_runtime().focused_pane_id;
        let weak_workspace = cx.entity().downgrade();
        let login_unavailable = self.active_agent_login_unavailable();
        let login_pending = self.is_login_pending();
        let focused_account = self
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == focused_pane_id)
            .and_then(|p| p.account_id())
            .map(|account_id| {
                status_bar::AccountSlot::resolve(
                    focused_pane_id,
                    account_id,
                    &self.accounts,
                    weak_workspace.clone(),
                    login_unavailable,
                    login_pending,
                )
            });
        // `project_config_path` (canonicalize) + `Path::exists` are
        // filesystem stats, and `render()` re-runs on every animation frame
        // (status badges request frames without `cx.notify`). Memoize the
        // flag keyed by the active project root so the stat only fires when
        // the active project changes; `reload_config` clears the memo so a
        // freshly created project layer surfaces immediately.
        let active_root = self.active_project().map(|p| p.root.clone());
        let has_project_config = match &active_root {
            Some(root) => {
                if self.cached_project_config.as_ref().map(|(r, _)| r) != Some(root) {
                    let exists = daruda_config::project_config_path(root)
                        .is_some_and(|path: std::path::PathBuf| path.exists());
                    self.cached_project_config = Some((root.clone(), exists));
                }
                self.cached_project_config
                    .as_ref()
                    .map(|(_, exists)| *exists)
                    .unwrap_or(false)
            }
            None => false,
        };
        let status_data = StatusBarData {
            project_branch: self.active_project_branch_label().map(Into::into),
            is_detached: matches!(self.active_branch_status(), super::BranchStatus::Detached),
            title: focused_title,
            error: self.last_error.clone(),
            has_project_config,
            account: focused_account,
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
            .on_action(cx.listener(|this, _: &SaveFilePane, _window, cx| {
                this.save_focused_file_pane(cx);
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
            // Intercept key events when the Lane switcher is open.
            .when(self.lane_switcher.is_open, |el| {
                el.on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                    if !this.lane_switcher.is_open {
                        return;
                    }
                    let key = ev.keystroke.key.as_str();
                    match key {
                        "escape" => {
                            this.lane_switcher.close();
                            cx.notify();
                        }
                        "enter" => {
                            this.execute_lane_switcher_selection(window, cx);
                        }
                        "up" => {
                            this.lane_switcher.move_up();
                            cx.notify();
                        }
                        "down" => {
                            let max = this.lane_switcher.filtered().len();
                            this.lane_switcher.move_down(max);
                            cx.notify();
                        }
                        "backspace" => {
                            this.lane_switcher.backspace();
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
                                this.lane_switcher.append(ch);
                                cx.notify();
                            }
                        }
                    }
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
                    this.clear_tab_drag_state(cx);
                    return;
                }
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
                    this.end_file_selection_drag(cx);
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
            .on_action(cx.listener(Self::on_toggle_bottom_dock))
            .on_action(cx.listener(Self::on_toggle_right_dock))
            .on_action(cx.listener(Self::on_toggle_command_palette))
            .on_action(cx.listener(Self::on_toggle_lane_switcher))
            .on_action(cx.listener(Self::on_show_left_dock_worktrees))
            .on_action(cx.listener(Self::on_show_left_dock_git))
            .on_action(cx.listener(Self::on_show_left_dock_files))
            .on_action(cx.listener(Self::on_switch_right_panel_usage))
            .on_action(cx.listener(Self::on_switch_right_panel_skills))
            .on_action(cx.listener(Self::on_switch_right_panel_tools))
            .on_action(cx.listener(Self::on_switch_right_panel_tasks))
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
            .on_action(cx.listener(Self::on_open_settings))
            .on_action(cx.listener(Self::on_switch_pane_account))
            .on_action(cx.listener(Self::on_add_managed_account))
            .on_action(cx.listener(Self::on_reauthenticate_account))
            .on_action(cx.listener(Self::on_open_project_config))
            .on_action(cx.listener(Self::on_install_agent_hooks))
            .on_action(cx.listener(Self::on_uninstall_agent_hooks))
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
            .child(lane_switcher::LaneSwitcherOverlay::new(
                self.lane_switcher.clone(),
                cx.listener(|this, _, _, cx| {
                    this.lane_switcher.close();
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
            // `gpui_component::Root`, but `Root::render` only
            // renders the inner view — Dialog/Sheet/Notification layers
            // must be rendered by the inner view explicitly. Without
            // these, `window.open_dialog(...)` registers a dialog into
            // `Root.active_dialogs` but nothing ever paints it.
            .children(gpui_component::Root::render_sheet_layer(window, cx))
            .children(gpui_component::Root::render_dialog_layer(window, cx))
            .children(gpui_component::Root::render_notification_layer(window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::{agent_menu_entry_disabled, agent_menu_entry_label, agent_menu_is_flat};

    #[test]
    fn agent_menu_flat_boundary() {
        // 0..=5 flat, 6+ submenu (AGENT_MENU_FLAT_MAX = 5).
        assert!(agent_menu_is_flat(1));
        assert!(agent_menu_is_flat(5));
        assert!(!agent_menu_is_flat(6));
        assert!(!agent_menu_is_flat(20));
    }

    #[test]
    fn agent_menu_entry_disabled_only_when_remote_needed_and_unset() {
        // Local agent: never disabled, regardless of lane state.
        assert!(!agent_menu_entry_disabled(false, false));
        assert!(!agent_menu_entry_disabled(false, true));
        // Remote agent: disabled unless the active lane has a remote_cwd.
        assert!(agent_menu_entry_disabled(true, false));
        assert!(!agent_menu_entry_disabled(true, true));
    }

    #[test]
    fn agent_menu_entry_label_appends_suffix_only_when_disabled() {
        let enabled_label = agent_menu_entry_label("New Claude".to_string(), false);
        assert_eq!(enabled_label, "New Claude");

        let disabled_label = agent_menu_entry_label("New Claude".to_string(), true);
        assert_ne!(disabled_label, "New Claude");
        assert!(disabled_label.starts_with("New Claude"));
    }
}
