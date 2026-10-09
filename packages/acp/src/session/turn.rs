//! FIFO command dispatch and one prompt turn at a time.

use super::*;

/// The multi-turn pump. Runs prompts strictly one turn at a time, draining a
/// stash of prompts that arrived mid-turn before pulling new commands, and
/// ending when the command channel closes (handle dropped).
///
/// Turns are serialized because a session is single-threaded: a second
/// `session/prompt` cannot be issued until the first turn's stop reason
/// returns. The stash preserves arrival order without losing any prompt.
pub(super) async fn prompt_loop(
    connection: &ConnectionTo<Agent>,
    session_id: SessionId,
    mut command_rx: UnboundedReceiver<Command>,
    event_tx: &UnboundedSender<AcpEvent>,
    mode_tracker: &ModeTracker,
    capabilities: SessionCapabilitiesView,
) -> Result<(), agent_client_protocol::Error> {
    let mut stash: VecDeque<PromptInput> = VecDeque::new();
    loop {
        // Prefer a prompt queued during the previous turn; otherwise block on
        // the channel for the next command.
        let text = match stash.pop_front() {
            Some(text) => text,
            None => match command_rx.next().await {
                Some(Command::Prompt(text)) => text,
                // A cancel with no active turn has nothing to cancel; ignore.
                Some(Command::Cancel) => continue,
                // A mode switch while idle: issue the request and wait for the
                // next command (the agent confirms via CurrentModeUpdate). A
                // rejected switch is non-fatal (Notice), never a session kill.
                Some(Command::SetMode(id)) => {
                    send_set_mode(connection, &session_id, id, event_tx).await;
                    continue;
                }
                // A config option change while idle: issue the request and wait
                // for the next command. The response carries the updated set; a
                // failure is non-fatal (Notice).
                Some(Command::SetConfigOption { config_id, value }) => {
                    send_set_config_option(
                        connection,
                        &session_id,
                        config_id,
                        value,
                        event_tx,
                        mode_tracker,
                    )
                    .await;
                    continue;
                }
                None => return Ok(()),
            },
        };
        if text
            .attachments
            .iter()
            .any(|attachment| !attachment.supported_by(capabilities))
        {
            let _ = event_tx.unbounded_send(AcpEvent::TurnFailed(AcpFailure::unclassified(
                "The agent does not support this prompt attachment",
            )));
            continue;
        }
        let dropped = run_turn(
            connection,
            &session_id,
            text,
            &mut command_rx,
            &mut stash,
            event_tx,
            mode_tracker,
        )
        .await?;
        if dropped {
            // Handle was dropped mid-turn: no more commands will arrive.
            return Ok(());
        }
    }
}

/// Send one `session/prompt` and await its stop reason while servicing commands
/// that arrive mid-turn: a `Cancel` is forwarded as `session/cancel`; a queued
/// `Prompt` is stashed for the outer loop to run next (never dropped). Returns
/// `true` if the command channel closed (handle dropped) during the turn.
pub(super) async fn run_turn(
    connection: &ConnectionTo<Agent>,
    session_id: &SessionId,
    text: PromptInput,
    command_rx: &mut UnboundedReceiver<Command>,
    stash: &mut VecDeque<PromptInput>,
    event_tx: &UnboundedSender<AcpEvent>,
    mode_tracker: &ModeTracker,
) -> Result<bool, agent_client_protocol::Error> {
    let response = connection
        .send_request(PromptRequest::new(session_id.clone(), text.into_blocks()))
        .block_task()
        .fuse();
    // `block_task()`'s future is `!Unpin`; `select!` requires `Unpin`.
    futures::pin_mut!(response);

    let mut handle_dropped = false;
    // The usage rides out of the loop with the stop reason: it is only ever set
    // on the one arm that breaks, so pairing them keeps that obvious.
    let (stop_reason, turn_usage) = loop {
        futures::select! {
            resp = response => match resp {
                Ok(r) => {
                    let usage = r.usage.as_ref().map(crate::model::TurnUsageView::from);
                    break (r.stop_reason, usage);
                }
                Err(e) if agent_client_protocol::is_incoming_transport_closed(&e) => return Err(e),
                Err(e) => {
                    // A `session/prompt` that returns a JSON-RPC error (e.g. the
                    // adapter hit a usage / session limit → `-32603`) is a
                    // TURN-level failure, NOT a connection failure: the error is
                    // a normal response, so the stdio connection stays alive.
                    // Surface it as `TurnFailed` and let `prompt_loop` continue,
                    // so the session stays usable and the user can re-prompt
                    // (e.g. once the limit resets) without reconnecting. This is
                    // the one prompt-error path that must NOT propagate `?` and
                    // tear the whole session down.
                    //
                    // Transport EOF propagates separately: reconnecting, not
                    // re-sending into the dead channel, is the only recovery.
                    let _ = event_tx.unbounded_send(AcpEvent::TurnFailed(AcpFailure::classify(&e)));
                    return Ok(handle_dropped);
                }
            },
            command = command_rx.next() => match command {
                Some(Command::Cancel) => {
                    connection.send_notification(CancelNotification::new(session_id.clone()))?;
                    // Keep awaiting: the agent still returns a (Cancelled) stop
                    // reason and may flush final updates first.
                }
                Some(Command::Prompt(queued)) => {
                    // Sessions are single-turn; run it after this turn ends.
                    stash.push_back(queued);
                }
                Some(Command::SetMode(id)) => {
                    // Mode switch mid-turn: send the request and keep awaiting
                    // the prompt response; the confirmation arrives via
                    // CurrentModeUpdate notification. Non-fatal on rejection.
                    send_set_mode(connection, session_id, id, event_tx).await;
                }
                Some(Command::SetConfigOption { config_id, value }) => {
                    // Config change mid-turn: issue it and keep awaiting the
                    // prompt response; the updated set arrives in the response,
                    // forwarded as ConfigOptionsChanged. Non-fatal on rejection.
                    send_set_config_option(
                        connection,
                        session_id,
                        config_id,
                        value,
                        event_tx,
                        mode_tracker,
                    )
                    .await;
                }
                None => {
                    // Handle dropped mid-turn: cancel and let the turn wind
                    // down. `UnboundedReceiver` is a `FusedStream`, so once it
                    // reports `None` `select!` stops polling it — no spin — and
                    // the loop now waits only on `response`.
                    connection.send_notification(CancelNotification::new(session_id.clone()))?;
                    handle_dropped = true;
                }
            },
        }
    };

    let _ = event_tx.unbounded_send(AcpEvent::TurnEnded {
        completed_normally: turn_completed_normally(&stop_reason),
        stop_reason: format!("{stop_reason:?}"),
        usage: turn_usage,
    });
    Ok(handle_dropped)
}

/// Whether a turn's stop reason represents a normal completion rather than a
/// client-initiated cancellation. Every stop reason except `Cancelled` (a normal
/// `EndTurn`, hitting `MaxTokens` / `MaxTurnRequests`, or a `Refusal`) is a turn
/// that ran to its own conclusion.
pub(super) fn turn_completed_normally(sr: &StopReason) -> bool {
    !matches!(sr, StopReason::Cancelled)
}

/// Send a `session/set_mode`, downgrading a rejected switch to a non-fatal
/// [`AcpEvent::Notice`]. A mode switch is a user-initiated optional action, so
/// an adapter that rejects it must NOT tear down the live session (unlike a
/// `session/prompt` failure, which is terminal). Infallible by design: the
/// connection keeps running whatever the adapter answers. On success the agent
/// confirms the switch via a `CurrentModeUpdate` notification
/// ([`AcpEvent::ModeChanged`]).
pub(super) async fn send_set_mode(
    connection: &ConnectionTo<Agent>,
    session_id: &SessionId,
    mode_id: String,
    event_tx: &UnboundedSender<AcpEvent>,
) {
    if let Err(e) = connection
        .send_request(SetSessionModeRequest::new(
            session_id.clone(),
            mode_id.clone(),
        ))
        .block_task()
        .await
    {
        let _ = event_tx.unbounded_send(AcpEvent::Notice(format!(
            "set_mode({mode_id}) failed — the session stays in its current mode: {e:?}"
        )));
    }
}

/// Send a `session/set_config_option` and broadcast the agent's updated option
/// set as [`AcpEvent::ConfigOptionsChanged`]. The protocol returns the full
/// option list in the response, so the host replaces its cache wholesale (no
/// separate notification, unlike `set_mode` which confirms via
/// `CurrentModeUpdate`).
///
/// Infallible like [`send_set_mode`]: a rejected config change is a user-driven
/// optional action, downgraded to a non-fatal [`AcpEvent::Notice`] rather than
/// propagated as an error that would end the connection.
pub(super) async fn send_set_config_option(
    connection: &ConnectionTo<Agent>,
    session_id: &SessionId,
    config_id: String,
    value: ConfigValueView,
    event_tx: &UnboundedSender<AcpEvent>,
    mode_tracker: &ModeTracker,
) {
    // The wire form is kind-specific: a select's value id serializes as a bare
    // string (`ValueId` is the untagged variant), a boolean as a tagged
    // `{"type":"boolean","value":…}`. Sending the wrong one is rejected.
    let protocol_value = match &value {
        ConfigValueView::Id(id) => SessionConfigOptionValue::value_id(id.clone()),
        ConfigValueView::Bool(b) => SessionConfigOptionValue::boolean(*b),
    };
    match connection
        .send_request(SetSessionConfigOptionRequest::new(
            session_id.clone(),
            config_id.clone(),
            protocol_value,
        ))
        .block_task()
        .await
    {
        Ok(resp) => {
            send_config_options_fold(
                mode_tracker,
                config_options_from_protocol(&resp.config_options),
                event_tx,
            );
        }
        Err(e) => {
            // Both: the `Notice` is what a chat pane already shows, and the
            // typed event is what a consumer that required the change acts
            // on. Emitting only the typed one would silently drop the
            // wording the agent panel puts in front of a user.
            let reason = format!("{e:?}");
            let _ = event_tx.unbounded_send(AcpEvent::Notice(format!(
                "set_config_option({config_id}={value:?}) failed — the session keeps its \
                 current value: {reason}"
            )));
            let _ = event_tx.unbounded_send(AcpEvent::ConfigOptionRejected { config_id, reason });
        }
    }
}
