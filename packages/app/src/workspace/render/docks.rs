//! Dock chrome for the workspace frame: title-bar toggles, resize handles, and
//! staging dock snapshots before a frame is drawn.

use crate::ui::cursor::{CursorReach, CursorReachExt as _};
use crate::ui::theme;
use gpui::{
    ClickEvent, Context, CursorStyle, IntoElement, MouseButton, MouseDownEvent, Window, div,
    prelude::*, px,
};

use crate::workspace::Workspace;
use crate::workspace::layout::{DockPosition, DockSnapshot};
use crate::workspace::main_area::pane_tree::DIVIDER_PX;

// Material names the large region's docking direction, so the narrow
// panel appears on the opposite side for the left/right icons.
pub(super) fn dock_toggle_icon_path(position: DockPosition, is_open: bool) -> &'static str {
    match (position, is_open) {
        (DockPosition::Left, false) => "icons/ui/dock-to-right.svg",
        (DockPosition::Left, true) => "icons/ui/dock-to-right-fill.svg",
        (DockPosition::Bottom, false) => "icons/ui/dock-to-bottom.svg",
        (DockPosition::Bottom, true) => "icons/ui/dock-to-bottom-fill.svg",
        (DockPosition::Right, false) => "icons/ui/dock-to-left.svg",
        (DockPosition::Right, true) => "icons/ui/dock-to-left-fill.svg",
    }
}

/// Title-bar dock toggle; the filled panel denotes an open dock.
pub(super) fn dock_toggle_icon(
    id: &'static str,
    position: DockPosition,
    is_active: bool,
    cx: &gpui::App,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let icon = dock_toggle_icon_path(position, is_active);
    crate::ui::button_toggle_icon(id, icon, is_active, cx).on_click(on_click)
}

/// Resize handle — purely a hit target, absolutely positioned so it
/// occupies NO flex layout space. Centered on the **visible border
/// line**, not on the dock's outer edge: since the 1px dock border is
/// drawn inside the dock (e.g. `border_r_1` at `[dock_size - 1, dock_size]`
/// for the left dock), the line center sits at `dock_size - DIVIDER_PX/2`.
/// Aligning there keeps the hit zone symmetric around the line — same
/// model as `render_layout`'s pane divider, where the overlay sits on
/// top of a 1px flex-child visible line.
pub(super) fn dock_resize_handle(
    position: DockPosition,
    dock_size: f32,
    dragged_dock: Option<DockPosition>,
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

    // Not `.cursor()`: a hitbox-bound style stops applying the moment the drag
    // pulls the pointer off these few pixels, which is immediately.
    let reach = CursorReach::while_dragging(cursor, dragged_dock == Some(position));
    let mut handle = div().id(id_str).absolute().cursor_reach(Some(reach));
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
        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
            this.on_dock_resize_press(position, ev, window, cx)
        }),
    )
}

/// What `Workspace::render` needs from the dock entities: the snapshots staged
/// into them, and the geometry read back out.
pub(super) struct DockFrame {
    pub(super) dragged_dock: Option<DockPosition>,
    pub(super) left_dock_open: bool,
    pub(super) left_dock_size: f32,
    pub(super) bottom_dock_open: bool,
    pub(super) bottom_dock_size: f32,
    pub(super) right_dock_open: bool,
    pub(super) right_dock_size: f32,
}

impl Workspace {
    /// Stage each dock's snapshot and read its geometry back.
    ///
    /// Called only when the docks are actually on screen — see the call site
    /// for why touching them otherwise is not merely wasted work.
    pub(super) fn stage_docks(&mut self, cx: &mut Context<Self>) -> DockFrame {
        // --- Stage dock snapshots before GPUI descends into dock entities ---
        //
        // Each snapshot is a plain-data copy of the Workspace fields the
        // dock's render needs.  Written here (Context<Workspace>) so the
        // dock render closure runs inside Context<Dock> without reaching
        // back through WeakEntity<Workspace>.

        // Ensure the file tree is primed before snapshotting its state.
        let active_ref = self.active_ref();
        if self.docks.left.read(cx).is_open
            && self.docks.left_view == daruda_store::project::LeftDockView::Files
            && self.lane_file_tree(active_ref).is_none()
        {
            self.ensure_file_tree(active_ref, cx);
        }

        // — Build dock snapshots ————————————————————————————————————————
        let left_snap = self.prepare_left_dock_snapshot(cx);
        let bottom_snap = self.prepare_bottom_dock_snapshot(cx);
        let right_snap = self.prepare_right_dock_snapshot(cx);
        self.stage_flow_page(cx);

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
        self.docks.left.update(cx, |d, cx| {
            d.stage_for_render(DockSnapshot::Left(Box::new(left_snap)), cx)
        });
        // Bottom dock is wrapped in `.cached()` (see `body`/`main_area`
        // below), so it must be marked dirty only when its snapshot
        // content actually changes — otherwise the cached view shows
        // stale data, and an unconditional notify would defeat the cache
        // by repainting on every 250 ms status-pulse tick (which leaves
        // this snapshot identical). Per root AGENTS.md Pitfall #10.
        self.docks.bottom.update(cx, |d, cx| {
            d.stage_for_render(DockSnapshot::Bottom(Box::new(bottom_snap)), cx)
        });
        self.docks.right.update(cx, |d, cx| {
            d.stage_for_render(DockSnapshot::Right(Box::new(right_snap)), cx)
        });

        // Read dock display state after staging snapshots.
        // Which handle is being held, so its cursor can reach past itself.
        let dragged_dock = self.docks.drag.map(|drag| drag.position);
        let (left_dock_open, left_dock_size) = {
            let d = self.docks.left.read(cx);
            (d.is_open, d.display_size())
        };
        let (bottom_dock_open, bottom_dock_size) = {
            let d = self.docks.bottom.read(cx);
            (d.is_open, self.bottom_dock_display_size(cx))
        };
        let (right_dock_open, right_dock_size) = {
            let d = self.docks.right.read(cx);
            (d.is_open, d.size)
        };
        DockFrame {
            dragged_dock,
            left_dock_open,
            left_dock_size,
            bottom_dock_open,
            bottom_dock_size,
            right_dock_open,
            right_dock_size,
        }
    }
}
