//! Flow-definition watcher → GPUI bridge.
//!
//! Mirrors [`super::mcp`]: every 100 ms the pump drains whatever the watcher
//! debounced and hands it to [`Workspace::apply_flows_event`]. Dropping the
//! returned `Task<()>` (held as `_flow_event_pump` on `Workspace`) cancels the
//! loop, the receiver disconnects, and the watcher thread exits.
//!
//! What a change means here is deliberately coarse — see
//! [`crate::hooks::flow_watcher::FlowsEvent`]: every open graph pane reads its
//! file again, and a pane whose bytes did not change does nothing at all. So the
//! cost of "something in this lane's flows changed" is one small read per open
//! graph, and a repaint only where the picture is actually different.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use gpui::{Context, Task, Window};

use crate::hooks::flow_watcher::{self, FlowsEvent};
use crate::workspace::{Workspace, flow_paths};

const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(in crate::workspace) fn spawn(
    events: Receiver<FlowsEvent>,
    cx: &mut Context<Workspace>,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        'outer: loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            loop {
                match events.try_recv() {
                    Ok(FlowsEvent::Changed) => {}
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break 'outer,
                }
                // A window is needed because rebuilding a graph builds a canvas,
                // which is a view. `update_in` fails once the window is gone —
                // that is the same signal as a released entity, so stop.
                if this
                    .update_in(cx, |ws, window, cx| ws.apply_flows_event(window, cx))
                    .is_err()
                {
                    break 'outer;
                }
            }
        }
    })
}

impl Workspace {
    /// Something in this lane's flow directories changed: forget the panel's
    /// cached list, and let every open graph re-read its own file.
    ///
    /// Panes of *every* lane are told, not just the active one's: the three
    /// source directories are shared (a project's and the person's own), so a
    /// change can belong to a graph sitting in a lane that is not on screen.
    /// Telling it now means the tab is right when it comes back rather than
    /// showing a picture of a file that has moved on.
    pub(in crate::workspace) fn apply_flows_event(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.invalidate_flow_list();
        self.reload_flow_graphs(None, window, cx);
        // No blanket notify: a pane that actually changed raised its own, which
        // marks this view dirty through it, and gpui has no partial redraw — a
        // notify here would repaint the whole window for an event that changed
        // nothing (an editor touching a file, our own write coming back). What
        // it *is* needed for is the panel's list, which is rendered from here.
        if self.right_dock_view == daruda_store::project::RightDockView::Flows {
            cx.notify();
        }
    }

    /// (Re)spawn the flow watcher over the active lane's source directories.
    /// Call it wherever the active lane changes, and after the app itself
    /// creates or deletes a flow — creating the first one also creates the
    /// directory the watcher has to be anchored on.
    pub(in crate::workspace) fn respawn_flow_watcher(&mut self, cx: &mut Context<Self>) {
        self._flow_watcher = None;
        self._flow_event_pump = None;

        let Some(sources) = self.flow_sources() else {
            return;
        };
        // The origin is what a *listing* needs; anchoring only needs the paths.
        let dirs = sources.dirs().into_iter().map(|(dir, _)| dir).collect();
        let (events, handle) = flow_watcher::spawn(dirs, flow_paths::FLOW_EXTENSIONS.to_vec());
        self._flow_event_pump = Some(spawn(events, cx));
        self._flow_watcher = Some(handle);
    }
}
