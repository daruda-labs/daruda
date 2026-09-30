//! Fold one ACP event from a pane's session into the workspace: the view's
//! transcript, the docks, persistence, and the task and phone side effects.

use gpui::Context;

use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

/// What the event pump does after one event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum PumpStep {
    Continue,
    /// The pane let go of its session at `Connected` — a read-only snapshot
    /// keeps only the replayed history — so the pump ends without treating
    /// the closing stream as a failure.
    Release,
    /// The pane is gone.
    Stop,
}

impl Workspace {
    pub(in crate::workspace) fn fold_agent_chat_event(
        &mut self,
        pane_id: PaneId,
        event: daruda_acp::AcpEvent,
        cx: &mut Context<Self>,
    ) -> PumpStep {
        let is_connected = matches!(&event, daruda_acp::AcpEvent::Connected { .. });
        let (syntax_theme, is_light) = self.agent_chat_theme_params(cx);
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return PumpStep::Stop;
        };
        // A mirror answers nothing; a replayed permission is cancelled on the
        // spot so the adapter is never left waiting on it.
        if view.read(cx).is_read_only()
            && let daruda_acp::AcpEvent::PermissionRequested { id, .. } = &event
        {
            if let Some(handle) = view.read(cx).any_handle() {
                handle.respond_permission(*id, daruda_acp::PermissionDecision::Cancelled);
            }
            return PumpStep::Continue;
        }
        // The displayed session status (Working / Idle /
        // NeedsAttention / …) feeds the cached per-lane
        // dock badge. `apply_event` only self-notifies the
        // view, so dirty the docks when the status actually
        // changes — including for a *parked* lane, whose
        // badge would otherwise freeze at its last animating
        // frame once the pulse stops. Gated on change so
        // token-streaming events don't repaint the docks.
        let before = view.read(cx).to_session_status();
        // Capture the persisted session identity before the
        // event: `Connected` establishes the live session id
        // and `SessionInfoChanged` sets the title. Both are
        // persisted (the id lets a later launch resume via
        // `session/load`; the title names the tab), so a save
        // is triggered below when either changes.
        let session_id_before = view.read(cx).session_id.clone();
        let title_before = view.read(cx).session_title.clone();
        // Also persisted (see `last_known_mode_id`'s doc) —
        // reapplied on the next resume to work around
        // `claude-agent-acp` not restoring it itself.
        let mode_id_before = view.read(cx).last_known_mode_id.clone();
        // Capture current mode before the event so we can
        // detect `Connected` (modes arriving) and
        // `ModeChanged` (current switching) and refresh the
        // bottom-input placeholder when either fires.
        let mode_before = view
            .read(cx)
            .session_config
            .current_mode_id()
            .map(str::to_string);
        // Desktop notification for a permission wait, gated by
        // focus. Must borrow `&event` before the move into
        // `apply_event` below. Turn *completion* fires later,
        // at the activity-settle edge (see the reconcile below).
        self.maybe_notify_agent_event(pane_id, &event, cx);
        self.report_agent_notice(pane_id, &event, cx);
        // Refresh the persisted option vocabularies from
        // what this agent just advertised. Also borrows
        // `&event` before the move below.
        self.record_agent_vocabulary(pane_id, &event, cx);
        let telegram_first_response = view.update(cx, |v, cx| {
            v.apply_event(event, &syntax_theme, is_light, cx)
        });
        self.relay_phone_ack_effect(pane_id, telegram_first_response, cx);
        if is_connected {
            self.record_task_chat_session(pane_id, cx);
            self.finish_cli_chat_load(pane_id, cx);
        }
        // Advance the activity span now that the event folded
        // in. When this event drove the last busy→idle
        // transition (the turn ended and no subagent is still
        // running), `tick_activity` returns the captured
        // outcome and the completion signals fire exactly once.
        // A still-running subagent leaves the pane busy, so the
        // firing defers to the pulse tick that catches the
        // quiescence settle. AgentChat-surfaced tasks reconcile
        // off this edge (they never write the status-file hooks
        // the Terminal surface uses).
        let edge = view.update(cx, |v, cx| v.tick_activity(std::time::Instant::now(), cx));
        if let Some(outcome) = edge {
            self.fire_activity_completion(pane_id, outcome, cx);
        }
        if view.read(cx).to_session_status() != before {
            self.notify_status_docks(cx);
        }
        // Persist when the session id is newly established
        // (or changed) or the title changed. Both change
        // rarely — once at connect, then on the occasional
        // `SessionInfoChanged` — so this never thrashes on
        // token-streaming events.
        {
            let v = view.read(cx);
            if v.session_id != session_id_before
                || v.session_title != title_before
                || v.last_known_mode_id != mode_id_before
            {
                self.mutate_durable(cx, |_, _| {});
            }
        }
        // Refresh placeholder when the active mode changed or
        // modes became available (Connected). Only fires for
        // the focused pane to avoid redundant work on parked
        // lane vieself.
        let mode_after = view
            .read(cx)
            .session_config
            .current_mode_id()
            .map(str::to_string);
        let focused_id = self.active_runtime().focused_pane_id;
        if mode_before != mode_after && focused_id == pane_id {
            self.refresh_terminal_input_placeholder(cx);
        }
        if is_connected && view.read(cx).is_read_only() {
            return PumpStep::Release;
        }
        PumpStep::Continue
    }
}
