//! Raw notification compatibility and forwarding into host events.

use super::*;

/// `session/update`, received as a raw payload instead of a typed one.
///
/// The typed `SessionNotification` rejects any `sessionUpdate` this schema
/// version has no variant for — and the SDK logs a rejected notification and
/// moves on (`jsonrpc/incoming_actor.rs`: "Notification errors are logged
/// without replying"), so the update is not an error the host ever sees, it
/// simply never happened. A draft-protocol update — a native subagent session's
/// traffic — would therefore vanish in silence. Taking `update` as JSON lets
/// [`NativeSubagentRouter`] decide what it is; every standard update still ends
/// up in the same typed [`SessionUpdate`] it always did.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, JsonRpcNotification)]
#[notification(method = "session/update")]
#[serde(rename_all = "camelCase")]
pub(super) struct CompatSessionNotification {
    /// Which session this update belongs to. The typed handler discarded this;
    /// with native subagents it is the only thing distinguishing a child's tool
    /// call from the main agent's.
    pub(super) session_id: SessionId,
    pub(super) update: serde_json::Value,
    #[serde(default, rename = "_meta")]
    #[allow(dead_code, reason = "Part of the wire shape; nothing reads it yet.")]
    pub(super) meta: Option<agent_client_protocol::schema::v1::Meta>,
}

/// Fold one standard `session/update` into the host's event stream.
///
/// The two mode-bearing updates go through the tracker (0..2 events out); the
/// rest map 1:1. Shared by root traffic and by the native subagent router's
/// normalized output, so a subagent's tool calls take exactly the path the main
/// agent's take.
pub(super) fn forward_session_update(
    update: SessionUpdate,
    mode_tracker: &ModeTracker,
    tx: &UnboundedSender<AcpEvent>,
) {
    let update = match update {
        SessionUpdate::CurrentModeUpdate(u) => {
            if let Some(state) = mode_tracker.apply_current_mode(u.current_mode_id.to_string()) {
                let _ = tx.unbounded_send(AcpEvent::ModeChanged { state });
            }
            return;
        }
        // The agent pushed a config-option change it made itself (e.g. a
        // fast-mode toggle, or effort reconciliation after a mode downgrade) —
        // not a reply to our `set_config_option`. Carries the full option set,
        // so reuse the same ConfigOptionsChanged full-replace the request path
        // emits; without this the model/effort chips show a stale value after
        // any agent-driven change.
        SessionUpdate::ConfigOptionUpdate(u) => {
            send_config_options_fold(
                mode_tracker,
                config_options_from_protocol(&u.config_options),
                tx,
            );
            return;
        }
        other => other,
    };
    let event = match update {
        SessionUpdate::AvailableCommandsUpdate(u) => AcpEvent::AvailableCommandsChanged(
            u.available_commands
                .iter()
                .map(crate::model::SlashCommand::from)
                .collect(),
        ),
        SessionUpdate::Plan(p) => AcpEvent::PlanChanged(
            p.entries
                .iter()
                .map(crate::model::PlanEntryView::from)
                .collect(),
        ),
        // Title and last-activity timestamp. The adapter pushes both together at
        // turn-end, but each field is mapped independently so a title-only or
        // timestamp-only update (per the protocol's per-field `MaybeUndefined`)
        // is also handled correctly.
        SessionUpdate::SessionInfoUpdate(u) => AcpEvent::SessionInfoChanged {
            title: u.title.into(),
            updated_at: u.updated_at.into(),
        },
        // Live context-window / cost accounting. Surfaced as a typed event (like
        // mode / plan / config) rather than raw `Update` so the host renders a
        // context meter without parsing protocol types. Distinct from the CLI's
        // cumulative Usage tab: this is the current context fill.
        SessionUpdate::UsageUpdate(u) => AcpEvent::UsageChanged(crate::model::UsageView::from(&u)),
        update => AcpEvent::Update(Box::new(update)),
    };
    let _ = tx.unbounded_send(event);
}
