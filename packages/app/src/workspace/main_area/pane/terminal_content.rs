//! Terminal pane runtime: cached OSC title / cwd, resize-to-fit, and dispatch of
//! terminal view events.

use super::*;

/// Live-drag coalescing gate (cmux `shouldApplySurfacePixelSizeChange`
/// analog): whether a freshly-computed grid should be forwarded to the PTY
/// and terminal view, given the grid already applied. A live drag emits a
/// bounds notification per pixel, but daruda's grid is cell-quantized, so most
/// notifications recompute the *same* `(cols, rows)`. Forwarding those fires a
/// redundant PTY SIGWINCH — the child app repaints over itself — plus a
/// ghostty reflow on every frame. Skip when the grid is unchanged; the next
/// real cell-boundary crossing differs and is forwarded.
pub(super) fn grid_resize_needed(current: (u16, u16), computed: (u16, u16)) -> bool {
    current != computed
}

impl TerminalContent {
    pub(in crate::workspace) fn has_exited(&self) -> bool {
        self.exited
    }

    /// Update OSC-derived title / cwd in place. Returns `true` iff a
    /// field actually changed, so the caller can scope `cx.notify` to
    /// real updates and skip idempotent OSC repeats.
    pub(super) fn update_cached(&mut self, new_title: String, new_cwd: Option<PathBuf>) -> bool {
        let mut changed = false;
        if self.cached_title.as_ref() != new_title {
            self.cached_title = SharedString::from(new_title);
            changed = true;
        }
        if self.cached_cwd != new_cwd {
            self.cached_cwd = new_cwd;
            changed = true;
        }
        changed
    }

    pub(super) fn resize_to_fit(
        &self,
        avail_w: f32,
        avail_h: f32,
        pane_header_h: f32,
        cache: &mut std::collections::HashMap<FontMetricsKey, TerminalLayout>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> bool {
        let key = FontMetricsKey::from_view(self.view.read(cx));
        let layout = cache.get(&key).copied().or_else(|| {
            let m = self.view.read(cx).cell_layout(window)?;
            cache.insert(key, m);
            Some(m)
        });

        let Some(layout) = layout else { return false };
        // Reserve the terminal-pane inset (left+right, top+bottom) so the
        // grid matches the painted content area — the element insets the
        // paint origin by the same `state.inset_*` (single source).
        let (inset_x, inset_y) = self.view.read(cx).inset();
        // ghostty_vt render paths are undefined for a 1-column terminal
        // (mirrors Zed's `cell_width * 2` minimum guard).
        let cols = layout.cols((avail_w - inset_x * 2.0).max(1.0)).max(2);
        let rows = layout.rows((avail_h - pane_header_h - inset_y * 2.0).max(1.0));

        // Live-drag coalescing gate: skip when the recomputed grid equals the
        // grid already applied (sub-cell pixel churn during a drag). Avoids a
        // redundant PTY SIGWINCH + ghostty reflow on every bounds notification.
        // Counts as "measured" — the grid is already correct — so the caller
        // does not mark the resize pending.
        let current_grid = {
            let view = self.view.read(cx);
            (view.session().cols(), view.session().rows())
        };
        if !grid_resize_needed(current_grid, (cols, rows)) {
            return true;
        }

        if let Some(master) = &self.master {
            let _ = master.resize(portable_pty::PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
        self.view
            .update(cx, |view, cx| view.resize_terminal(cols, rows, cx));
        true
    }
}

/// Dispatch one `TerminalViewEvent` from `pane_id` into the matching
/// platform call, gated by the `[notifications]` config and by the
/// "skip focused pane" rule (which itself only applies when daruda
/// is the foreground app).
pub(super) fn handle_view_event(
    workspace: &mut Workspace,
    pane_id: PaneId,
    event: &daruda_terminal::TerminalViewEvent,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    use crate::platform;
    use crate::surface::{constants::APP_NAME, strings as s};
    use daruda_terminal::{NotificationRequest, TerminalViewEvent};

    // A pane in view is silenced only when daruda itself is the
    // foreground app — backgrounded notifications always surface
    // because the user, by definition, is not looking at the pane.
    let suppressed_by_focus = workspace.mirrors.notifications.skip_focused_pane
        && platform::attention::is_app_active()
        && workspace.pane_on_screen(pane_id);

    match event {
        TerminalViewEvent::AttentionRequested(kind) => {
            // macOS auto-suppresses dock bounce when the app is
            // foreground, so we don't apply the focused-pane rule
            // here — the kernel of attention is "tell the user
            // something happened in the background".
            if workspace.mirrors.notifications.attention_enabled {
                platform::attention::apply(*kind);
            }
        }
        TerminalViewEvent::NotificationRequested(req) => {
            if suppressed_by_focus {
                return;
            }
            match req {
                NotificationRequest::Osc9 { body } => {
                    if workspace.mirrors.notifications.osc9_enabled {
                        platform::notifications::show(APP_NAME, body);
                    }
                }
                NotificationRequest::Osc777 { title, body } => {
                    if workspace.mirrors.notifications.osc777_enabled {
                        platform::notifications::show(title, body);
                    }
                }
            }
        }
        TerminalViewEvent::CommandFinishedAfter { elapsed } => {
            if !workspace.mirrors.notifications.long_running_enabled {
                return;
            }
            let threshold = std::time::Duration::from_secs(
                workspace.mirrors.notifications.long_running_threshold_secs,
            );
            if *elapsed < threshold {
                return;
            }
            if suppressed_by_focus {
                return;
            }
            let body = s::notification::format_duration_compact(*elapsed);
            let title = s::notification::long_running_title();
            platform::notifications::show(&title, &body);
        }
        TerminalViewEvent::AnnotationDoubleClicked { id } => {
            workspace.open_annotation_dialog_for_edit(pane_id, *id, window, cx);
        }
        TerminalViewEvent::ContextMenuRequested { position, range: _ } => {
            workspace.open_pane_context_menu_at(pane_id, *position, window, cx);
        }
    }
}
