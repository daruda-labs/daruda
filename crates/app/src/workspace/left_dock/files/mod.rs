//! Files view — file tree rooted at the active lane path.
//!
//! W-7e: virtual scroll via `uniform_list`. The visible-row list is
//! built once per cache invalidation (see
//! `workspace::file_tree_ops`), wrapped in `Arc`, and passed into the
//! `uniform_list` body closure. The closure renders only the rows in
//! the current viewport range — directories with thousands of
//! children pay the same per-frame cost as small ones.

use crate::ui::theme;
use daruda_config::IconColorMode;
use daruda_store::project::{LaneId, LaneRef};
use gpui::{
    AnyElement, ClickEvent, Context, Hsla, IntoElement, div, img, prelude::*, px, svg, uniform_list,
};

use crate::files::icons::icon_path;
use crate::files::tree::EntryKind;
use crate::surface::strings;
use crate::ui::{ButtonVariants as _, Icon, IconName, SectionHeader, Sizable as _, button_bare};
use crate::workspace::layout::Dock;
use crate::workspace::layout::LeftDockSnapshot;
use crate::workspace::left_dock::file_tree_ops::VisibleEntry;
use crate::workspace::left_dock::git_ops::{git_status_color, git_status_symbol};
use crate::workspace::path_drag::PathDrag;
use gpui::UniformListScrollHandle;

pub(in crate::workspace) fn render(snap: &LeftDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    let active_id = snap.active.lane;
    let active_ref = snap.active;
    let visible = snap.cached_visible.clone();
    let count = visible.len();
    let root_kind = snap.root_kind;

    let show_loading = count == 0 && !matches!(root_kind, Some(EntryKind::Dir));
    let show_empty = count == 0 && matches!(root_kind, Some(EntryKind::Dir));

    // `track_focus` + `key_context("FilesPanel")` route arrow keys to
    // FilesSelectNext / Prev / Expand / Collapse only when the panel
    // has focus — otherwise they fall through to terminal panes.
    let panel_focus = snap.files_panel_focus.clone();
    let workspace = snap.workspace.clone();
    let mut body = crate::workspace::left_dock::left_panel_body()
        .key_context("FilesPanel")
        .track_focus(&panel_focus);
    body = body.child(view_header(active_id, snap, cx));

    if show_loading {
        body = body.child(loading_placeholder(cx));
    } else if show_empty {
        body = body.child(empty_dir_placeholder(cx));
    } else {
        let color_mode = snap.files_icon_color_mode.clone();
        let scroll_handle = snap.files_scroll_handle.clone();
        // The row renderer runs inside Context<Dock> but needs Workspace
        // state (lane root path, per-range VisibleEntry slice). Pull
        // everything needed from the snapshot at closure-capture time.
        let visible_for_renderer = visible.clone();
        let workspace_for_renderer = workspace.clone();
        let worktree_root = snap
            .lanes
            .iter()
            .find(|w| w.id == active_id)
            .map(|w| w.path.clone())
            .unwrap_or_default();
        let row_renderer = cx.processor(
            move |_dock: &mut Dock,
                  range: std::ops::Range<usize>,
                  _window: &mut gpui::Window,
                  cx: &mut Context<Dock>| {
                // Give each row a `WeakEntity<Workspace>` for its event handlers.
                // The snap's lane root is already captured above.
                let _ = workspace_for_renderer.clone(); // keep alive in closure
                range
                    .filter_map(|i| visible_for_renderer.get(i).cloned())
                    .map(|v| {
                        render_row(
                            &v,
                            active_ref,
                            &worktree_root,
                            &color_mode,
                            workspace_for_renderer.clone(),
                            cx,
                        )
                    })
                    .collect::<Vec<_>>()
            },
        );
        let scrollbar = build_files_scrollbar(&scroll_handle, count, cx);
        let list_area = div()
            .flex_1()
            .relative()
            .overflow_hidden()
            .w_full()
            .child(
                uniform_list("files-tree", count, row_renderer)
                    .track_scroll(&scroll_handle)
                    .size_full(),
            )
            .when_some(scrollbar, |d, sb| d.child(sb));
        body = body.child(list_area);
    }

    body.into_any_element()
}

/// Scrollbar overlay for the file list. Returns `None` when the list
/// fits the viewport or `uniform_list` has not yet captured a layout
/// (first frame), so the panel does not flicker an empty thumb.
///
/// Scrollability uses `max_offset().y` rather than a manual
/// `count * row_h` estimate: the previous calculation could keep a
/// stale row height from before the panel resized and produced a
/// `content_h > viewport_h` even after the list fit again. `max_offset`
/// is computed by GPUI from the layout in the just-finished frame and
/// is exactly zero when the content fits.
fn build_files_scrollbar(
    handle: &UniformListScrollHandle,
    item_count: usize,
    cx: &gpui::App,
) -> Option<AnyElement> {
    if item_count == 0 {
        return None;
    }
    // `UniformListScrollHandle` exposes no public API for geometry; `.0`
    // reaches the internal `ListState` which is the only stable source.
    let state = handle.0.borrow();
    let viewport_h = state.base_handle.bounds().size.height;
    // `max_offset` returns `Size<Pixels>` in this gpui version — the
    // height component is the y-axis overflow (0 when content fits).
    let max_offset_h = state.base_handle.max_offset().height;
    let offset_y = state.base_handle.offset().y;
    drop(state);
    if viewport_h <= px(0.0) || max_offset_h <= px(0.0) {
        return None;
    }

    let content_h = viewport_h + max_offset_h;
    let thumb_ratio = (viewport_h / content_h).min(1.0_f32);
    let raw_thumb_h = viewport_h * thumb_ratio;
    let thumb_h = raw_thumb_h.max(px(theme::SCROLLBAR_MIN_THUMB_H));
    let track_h = viewport_h - thumb_h;
    let scroll_frac = ((-offset_y) / max_offset_h).clamp(0.0_f32, 1.0_f32);
    let thumb_top = track_h * scroll_frac;
    let thumb_w = px(theme::SCROLLBAR_W);
    let t = theme::current(cx);
    let thumb_bg = t.dock_scrollbar_thumb;
    let thumb_hover_bg = t.dock_scrollbar_thumb_hover;

    Some(
        div()
            .id("files-scrollbar")
            .absolute()
            .top(thumb_top)
            .right(px(theme::SCROLLBAR_MARGIN_R))
            .w(thumb_w)
            .h(thumb_h)
            .rounded(thumb_w / 2.0)
            .bg(thumb_bg)
            .hover(move |d| d.bg(thumb_hover_bg))
            .into_any_element(),
    )
}

// ----------------------------------------------------------------
// Header
// ----------------------------------------------------------------

fn view_header(
    _lane_id: LaneId,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    let refresh = button_bare("files-refresh")
        .xsmall()
        .ghost()
        .icon(IconName::Refresh)
        .on_click(cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.refresh_files_root(cx));
            }
        }));

    SectionHeader::new(strings::files_header_label())
        .padding(theme::GIT_HEADER_PAD_X, theme::GIT_HEADER_PAD_Y)
        .truncate_label(true)
        .actions(refresh)
}

// ----------------------------------------------------------------
// Row rendering
// ----------------------------------------------------------------

fn render_row(
    v: &VisibleEntry,
    wt_ref: LaneRef,
    worktree_root: &std::path::Path,
    color_mode: &IconColorMode,
    workspace: gpui::WeakEntity<crate::workspace::Workspace>,
    cx: &mut Context<Dock>,
) -> AnyElement {
    let lane_id = wt_ref.lane;
    let entry_id = v.entry_id;
    let kind = v.kind;
    let path = v.path.clone();
    let worktree_root_buf = worktree_root.to_path_buf();
    // chevron element built below after we have the row's text colors.
    let icon_asset = icon_path(kind, v.is_symlink, v.is_expanded, &v.name);
    let is_keyboard_focused = v.is_keyboard_focused;
    let is_ignored = v.is_ignored;
    let git_status = v.git_status;

    let t = theme::current(cx);
    let faint = t.faint_text;
    let muted = t.muted_text;
    let row_selected_bg = t.git_file_row_selected_bg;
    let row_hover_bg = t.git_file_row_hover_bg;

    let indent_px = (v.depth as f32) * theme::FILES_INDENT_PX;
    let row_text_color = if is_ignored { faint } else { muted };

    let mut row = div()
        .id(("files-row", entry_id.0 as usize))
        .flex()
        .flex_row()
        .items_center()
        .h(px(theme::FILES_ROW_HEIGHT))
        .pr(px(theme::FILES_ROW_PAD_X))
        .pl(px(theme::FILES_ROW_PAD_X + indent_px))
        .gap(px(theme::FILES_ROW_GAP))
        .text_size(px(theme::FILES_ROW_FONT_SIZE))
        .text_color(row_text_color)
        .cursor_pointer()
        .when(is_keyboard_focused, move |d| d.bg(row_selected_bg))
        .when(!is_keyboard_focused, move |d| {
            d.hover(move |d| d.bg(row_hover_bg))
        })
        // on_click fires only when mousedown + mouseup happen at the same
        // position (no drag past DRAG_THRESHOLD), so it coexists safely with
        // on_drag — a drag away from the row no longer also opens the file.
        .on_click(cx.listener(move |_dock, ev: &ClickEvent, window, cx| {
            if let Some(ws) = workspace.upgrade() {
                let click_count = ev.click_count();
                let alt = ev.modifiers().alt;
                ws.update(cx, |ws, cx| {
                    // Steal focus to the Files panel so subsequent arrow
                    // keys reach FilesPanel-bound actions.
                    let panel = ws.file_tree.files_panel_focus.clone();
                    panel.focus(window, cx);
                    ws.file_tree.files_selection = Some(entry_id);
                    ws.invalidate_visible_files_cache(wt_ref);

                    if click_count >= 2 {
                        if !kind.is_dir() {
                            ws.open_file_externally(lane_id, path.clone(), cx);
                        }
                        return;
                    }
                    // Alt+click on an expanded dir collapses the entire
                    // subtree in one shot.
                    if kind.is_dir() && alt {
                        ws.collapse_files_subtree(wt_ref, entry_id, cx);
                        return;
                    }
                    if kind.is_dir() {
                        ws.toggle_files_expand(wt_ref, entry_id, cx);
                    } else {
                        ws.open_files_entry(wt_ref, worktree_root_buf.join(&path), window, cx);
                    }
                });
            }
        }))
        .child(chevron_element(kind, v.is_expanded, faint))
        .child(file_icon(icon_asset, row_text_color, color_mode))
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(v.name.clone()),
        );

    if let Some(ch) = git_status {
        // Files view shows the working-tree (unstaged) colour because
        // its `staged=false` semantics match what the user sees on disk.
        let colour = git_status_color(ch, /* staged = */ false, cx);
        let symbol = git_status_symbol(ch);
        row = row.child(
            div()
                .flex_none()
                .w(px(theme::FILES_CHEVRON_W))
                .text_color(colour)
                .child(symbol),
        );
    }

    let abs_path = worktree_root.join(&v.path);
    row.on_drag(
        PathDrag {
            path: abs_path,
            offset: gpui::Point::default(),
        },
        |drag, pos, _window, cx| {
            cx.new(|_| PathDrag {
                path: drag.path.clone(),
                offset: pos,
            })
        },
    )
    .into_any_element()
}

/// Leading slot for a file row: chevron icon for directories, the
/// pending glyph while a directory is loading, or an empty slot for
/// regular files (preserves indent alignment).
fn chevron_element(kind: EntryKind, is_expanded: bool, color: Hsla) -> AnyElement {
    let slot = div()
        .flex_none()
        .w(px(theme::FILES_CHEVRON_W))
        .flex()
        .items_center()
        .justify_center()
        .text_color(color);
    match kind {
        EntryKind::Dir | EntryKind::UnloadedDir => {
            let icon = if is_expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            };
            slot.child(Icon::new(icon).xsmall().text_color(color))
                .into_any_element()
        }
        EntryKind::PendingDir => slot
            .child(strings::FILES_CHEVRON_PENDING)
            .into_any_element(),
        EntryKind::File => slot.into_any_element(),
    }
}

fn file_icon(path: &'static str, mono_color: Hsla, mode: &IconColorMode) -> AnyElement {
    match mode {
        IconColorMode::Color => img(path)
            .flex_none()
            .w(px(theme::FILES_ICON_W))
            .h(px(theme::FILES_ICON_W))
            .into_any_element(),
        IconColorMode::Monochrome => svg()
            .flex_none()
            .w(px(theme::FILES_ICON_W))
            .h(px(theme::FILES_ICON_W))
            .path(path)
            .text_color(mono_color)
            .into_any_element(),
    }
}

// ----------------------------------------------------------------
// Placeholders
// ----------------------------------------------------------------

fn loading_placeholder(cx: &gpui::App) -> AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
        .text_color(theme::current(cx).dock_placeholder_text)
        .child(div().w_full().text_center().child(strings::files_loading()))
        .into_any_element()
}

fn empty_dir_placeholder(cx: &gpui::App) -> AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
        .text_color(theme::current(cx).dock_placeholder_text)
        .child(
            div()
                .w_full()
                .text_center()
                .child(strings::files_empty_dir()),
        )
        .into_any_element()
}
