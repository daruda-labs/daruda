//! What to do with an update Telegram sent us.
//!
//! The long-poll loop and everything it dispatches into: routing an inbound
//! update through `BridgeCore`, giving a targetless one the app's own active
//! lane, and carrying out whichever action came back. The outbound half —
//! composing and posting pings — stays in [`super`].
//!
//! Split from `mod.rs` because the two halves share only the `TelegramBridge`
//! global and the bot token: they read different state, fail differently, and
//! neither one's changes should conflict with the other's.

use gpui::App;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use super::super::bridge::{
    BotPermissionOutcome, CallbackEdit, InboundAction, PermissionDecision, RouteResult,
};
use super::super::client;
use super::super::command;
use super::super::keychain;
use super::super::trace;
use super::control::{
    answer_only, log_unauthorized_inbound, persist_offset, render_outcome, report_target_gone,
    run_command, select_target, send_command_reply,
};
use super::{IDLE_RECHECK, POLL_TIMEOUT_SECS, TelegramBridge};
use crate::control::resolve as control_resolve;
use crate::control::result::ControlError;
use crate::platform::attention::is_app_active;
use crate::settings_store::SettingsStore;
use crate::surface::strings as s;
use crate::window_registry::WindowRegistry;
use crate::workspace::Workspace;

/// The inbound long-poll loop. Every iteration re-syncs `enabled` /
/// `authorized_chat_id` from the live `SettingsStore` so a
/// Settings-window toggle takes effect without any extra plumbing —
/// the same pattern `workspace/sync/limits.rs` uses for its poll
/// cadence.
pub(super) fn spawn_poll_task(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            let (enabled, chat_id, token, offset) = cx.update(|cx| {
                let cfg = SettingsStore::global(cx).user_arc();
                let bridge = cx.global_mut::<TelegramBridge>();
                bridge.core.set_enabled(cfg.telegram.enabled);
                bridge
                    .core
                    .set_authorized_chat_id(cfg.telegram.authorized_chat_id);
                (
                    bridge.core.is_enabled(),
                    cfg.telegram.authorized_chat_id,
                    keychain::read_token(),
                    bridge.core.current_offset(),
                )
            });
            // Why an idle bridge writes no line of its own: the gate is
            // resynced every iteration, so only its transitions are traced.
            trace::gate_change(enabled, chat_id, token.is_some());

            if !enabled {
                cx.background_executor().timer(IDLE_RECHECK).await;
                continue;
            }
            let Some(token) = token else {
                cx.background_executor().timer(IDLE_RECHECK).await;
                continue;
            };

            // Run the blocking `ureq` long-poll off the foreground thread —
            // it can hang for up to POLL_TIMEOUT_SECS and must never stall
            // the GPUI event loop.
            let fetch_token = token.clone();
            let fetched = cx
                .background_executor()
                .spawn(async move { client::get_updates(&fetch_token, offset, POLL_TIMEOUT_SECS) })
                .await;

            let updates = match fetched {
                Ok(updates) => updates,
                Err(e) => {
                    trace::delivery("poll.failed", || format!("offset={offset} error={e}"));
                    LogWriter::log(
                        ErrorReport::new("Telegram getUpdates failed")
                            .severity(ErrorSeverity::Info)
                            .from_error(&e)
                            .at(file!(), line!())
                            .dedup("telegram.get_updates")
                            .build(),
                    );
                    cx.background_executor().timer(IDLE_RECHECK).await;
                    continue;
                }
            };

            // Read after the fetch returns, not before it: the long-poll can
            // block for POLL_TIMEOUT_SECS, so a presence reading taken at the
            // top of the iteration would describe the wrong moment entirely.
            let app_active = trace::is_on().then(|| cx.update(|_cx| is_app_active()));
            trace::delivery("poll", || {
                format!(
                    "offset={offset} count={} app_active={}",
                    updates.len(),
                    trace::opt(app_active)
                )
            });

            // One update at a time, routed and then acted on before the next
            // is routed. A batch can hold `/use 2` and a plain message that
            // means it: routing them together would resolve the second
            // against the target the first had not yet moved.
            //
            // The global is held only for the routing step and released
            // before any side effect (HTTP calls, cross-workspace dispatch)
            // — routing is the only step that touches `BridgeCore` directly.
            for update in updates {
                let update_id = update.update_id;
                trace::delivery("inbound", || {
                    format!(
                        "update_id={update_id} kind={} chat_id={} app_active={} \
                         payload={}",
                        trace::update_kind_name(&update.kind),
                        trace::opt(trace::update_chat_id(&update.kind)),
                        trace::opt(app_active),
                        trace::update_payload(&update.kind)
                    )
                });
                let RouteResult {
                    action,
                    answer_callback_id,
                    callback_edit,
                } = cx.update(|cx| cx.global_mut::<TelegramBridge>().core.route(update));
                trace::delivery("routed", || {
                    format!(
                        "update_id={update_id} action={} answers_callback={}",
                        trace::action_name(&action),
                        answer_callback_id.is_some()
                    )
                });
                let action = adopt_fallback_target(action, cx);

                match (answer_callback_id, action) {
                    // A permission button tap: apply the decision FIRST so the
                    // feedback is accurate, then answer the callback (toast) and
                    // rewrite the message (drop the buttons + append the outcome).
                    (
                        Some(callback_id),
                        InboundAction::RespondPermission {
                            pane,
                            perm_id,
                            decision,
                        },
                    ) => {
                        let mut outcome = BotPermissionOutcome::Gone;
                        dispatch_to_workspace(cx, pane.workspace, |ws, cx| {
                            outcome =
                                ws.respond_bot_permission(pane.pane, perm_id, decision.clone(), cx);
                        });
                        trace::delivery("permission", || {
                            format!(
                                "pane={} perm_id={perm_id} decision={decision:?} \
                                 outcome={outcome:?}",
                                trace::pane(pane)
                            )
                        });
                        let label = permission_feedback(&decision, outcome);
                        answer_and_edit(cx, &token, callback_id, callback_edit, &label).await;
                    }
                    // A listing button tap: point the target at that pane and
                    // toast which one. The message keeps its buttons — tapping
                    // another row is ordinary use, not a second decision.
                    (Some(callback_id), InboundAction::SelectTarget { pane }) => {
                        let label = cx.update(|cx| select_target(pane, cx));
                        answer_only(cx, &token, callback_id, &label).await;
                    }
                    // An approval button tap. The card keeps its buttons: the
                    // tool call may still be waiting, and a second tap is
                    // ordinary use rather than a second decision.
                    (Some(callback_id), InboundAction::ResolveApproval { id, choice }) => {
                        // The label follows what *happened*, not what was
                        // tapped: the card keeps its buttons while it is live,
                        // so a second, contradictory tap must not be told it
                        // took effect. Same rule as `permission_feedback`.
                        let settled =
                            cx.update(|cx| crate::control::approval::resolve(id, choice, cx));
                        let label = match (settled, choice) {
                            (false, _) => s::control_approval_already_answered(),
                            (true, crate::control::approval::ApprovalChoice::Approved) => {
                                s::control_approval_allowed()
                            }
                            (true, crate::control::approval::ApprovalChoice::Refused) => {
                                s::control_approval_refused()
                            }
                        };
                        trace::delivery("approval.tap", || {
                            format!("id={id:?} choice={choice:?} settled={settled}")
                        });
                        answer_only(cx, &token, callback_id, &label).await;
                    }
                    // A tap on a superseded listing. Answered with its own
                    // wording and *not* edited — `answer_and_edit` drops the
                    // message's keyboard, which would strip the rows off a
                    // listing the user is still reading.
                    (Some(callback_id), InboundAction::StaleListing) => {
                        answer_only(cx, &token, callback_id, &s::control_listing_stale()).await;
                    }
                    // A callback whose token is unknown / already consumed: still
                    // tell the user it was already handled (never leave the tap
                    // silent).
                    (Some(callback_id), _) => {
                        let label = s::telegram_permission_stale();
                        answer_and_edit(cx, &token, callback_id, callback_edit, &label).await;
                    }
                    // A command: resolve, run, fold the outcome back into the
                    // adapter's ordinal table, answer.
                    (None, InboundAction::RunCommand { command }) => {
                        let reply = run_command(command, cx);
                        send_command_reply(cx, &token, reply).await;
                    }
                    // Never swallowed: a mistyped slash command that produced
                    // silence is the defect this replaces.
                    (None, InboundAction::ReportParseError { error }) => {
                        let reply = command::render_parse_error(&error);
                        send_command_reply(cx, &token, reply).await;
                    }
                    // Reached only when `adopt_fallback_target` found nothing
                    // to aim at. `text` is dropped here deliberately — there
                    // is no pane to put it on — which is exactly why this arm
                    // must never be reached with the adopt step skipped.
                    (None, InboundAction::Unaimed { text }) => {
                        trace::delivery("text.unaimed", || {
                            format!("kind=text len={}", text.chars().count())
                        });
                        let reply = cx
                            .update(|cx| render_outcome(&Err(ControlError::NoTargetSelected), cx));
                        send_command_reply(cx, &token, reply).await;
                    }
                    // A plain message routed to a remembered pane. The pane
                    // can be gone — a selection and a last-pinged target both
                    // outlive the pane they name — so the delivery is checked
                    // rather than assumed.
                    (None, InboundAction::InjectPrompt { pane, text }) => {
                        deliver_prompt(cx, &token, pane, text).await;
                    }
                    // A slash daruda does not own. Which vocabulary it belongs
                    // to is the *pane's* to answer — it advertises its own
                    // commands — and `route` is GPUI-free, so the question is
                    // settled here and then rejoins the two paths that already
                    // handle each outcome.
                    (
                        None,
                        InboundAction::UnknownSlash {
                            pane,
                            name,
                            text,
                            suggestion,
                        },
                    ) => {
                        // Defaults to the agent's: a window that is gone
                        // answers nothing, and the delivery attempt below
                        // reports `target_gone` far better than a typo answer
                        // guessed from silence would.
                        let mut agents = true;
                        dispatch_to_workspace(cx, pane.workspace, |ws, cx| {
                            agents = ws.agent_takes_slash_command(pane.pane, &name, cx);
                        });
                        trace::delivery("slash.unowned", || {
                            format!("pane={} name={name} to_agent={agents}", trace::pane(pane))
                        });
                        if agents {
                            deliver_prompt(cx, &token, pane, text).await;
                        } else {
                            let reply = command::render_parse_error(
                                &crate::control::spec::ParseError::Unknown {
                                    input: name,
                                    suggestion,
                                },
                            );
                            send_command_reply(cx, &token, reply).await;
                        }
                    }
                    // The same slash with nothing to aim it at. Having no
                    // target says nothing about whose command the name is, so
                    // the vocabularies are still asked — and only a name every
                    // one of them rules out is answered as a typo. Otherwise
                    // the missing target is the real reason it went nowhere.
                    (
                        None,
                        InboundAction::UnaimedSlash {
                            name,
                            text,
                            suggestion,
                        },
                    ) => {
                        let ours =
                            cx.update(|cx| control_resolve::slash_claim(cx, &name).rules_out());
                        // `text` is the message as sent, arguments and all,
                        // and both answers below drop it — there is no pane
                        // to put it on. Its length is traced for the same
                        // reason the plain arm traces its own: a body that
                        // went nowhere should leave a mark.
                        trace::delivery("slash.unaimed", || {
                            format!("name={name} ours={ours} len={}", text.chars().count())
                        });
                        let reply = if ours {
                            command::render_parse_error(
                                &crate::control::spec::ParseError::Unknown {
                                    input: name,
                                    suggestion,
                                },
                            )
                        } else {
                            cx.update(|cx| render_outcome(&Err(ControlError::NoTargetSelected), cx))
                        };
                        send_command_reply(cx, &token, reply).await;
                    }
                    // A message-origin action (pairing, ignore, unsupported).
                    (None, action) => dispatch_action(action, cx),
                }

                // After acting, not before: a crash in between re-delivers
                // this one update rather than losing it, and writing first
                // would drop the command outright. Per update rather than per
                // batch so the replay is bounded to one.
                persist_offset(cx);
            }

            // No extra sleep on success — the long-poll `timeout_s` itself
            // paces the loop (Telegram returns immediately on data, or
            // after ~timeout_s seconds when idle).
        }
    })
    .detach();
}

/// Put `text` on `pane`, answering the sender when the pane is gone.
///
/// The pane can be: a selection and a last-pinged target both outlive the pane
/// they name, so the delivery is checked rather than assumed. Shared by the
/// plain-message path and by a slash the agent claimed — the two differ in how
/// they were routed, not in how they are delivered.
async fn deliver_prompt(
    cx: &mut gpui::AsyncApp,
    token: &str,
    pane: crate::telegram::bridge::PaneRef,
    text: String,
) {
    let mut delivered = false;
    dispatch_to_workspace(cx, pane.workspace, |ws, cx| {
        delivered = ws.inject_bot_reply(pane.pane, text.clone(), cx);
    });
    trace::delivery("inject", || {
        format!(
            "pane={} delivered={delivered} text={}",
            trace::pane(pane),
            trace::preview(&text)
        )
    });
    if !delivered {
        let reply = cx.update(|cx| report_target_gone(pane, cx));
        send_command_reply(cx, token, reply).await;
    }
}

/// Apply one routed [`InboundAction`]'s side effect. Pure dispatch —
/// no routing policy here, `bridge.rs` already decided what to do.
fn dispatch_action(action: InboundAction, cx: &mut gpui::AsyncApp) {
    match action {
        InboundAction::Ignore => log_unauthorized_inbound(),
        InboundAction::Paired { chat_id } => {
            trace::state("paired", || format!("chat_id={chat_id}"));
            // `BridgeCore::route`'s `Paired` branch already updated the
            // in-memory `authorized_chat_id` — this persists it so pairing
            // survives a restart.
            cx.update(|cx| {
                let result = cx.global_mut::<SettingsStore>().apply_patch(
                    daruda_config::SettingsPatch::TelegramAuthorizedChatId(Some(chat_id)),
                );
                if let Err(e) = result {
                    LogWriter::log(
                        ErrorReport::new("Telegram pairing failed to persist")
                            .severity(ErrorSeverity::Warning)
                            .message(e)
                            .at(file!(), line!())
                            .dedup("telegram.pair.persist")
                            .build(),
                    );
                }
            });
        }
        InboundAction::InjectPrompt { .. } => {
            // Handled inline in the poll loop, which is the only place that
            // can await the "that chat is gone" answer.
        }
        InboundAction::UnknownSlash { .. } | InboundAction::UnaimedSlash { .. } => {
            // Same: settled in the poll loop, which can both read the
            // advertised command lists and await whichever answer that
            // settles on.
        }
        InboundAction::RespondPermission { .. }
        | InboundAction::SelectTarget { .. }
        | InboundAction::ResolveApproval { .. }
        | InboundAction::StaleListing => {
            // All three always arrive as callbacks and are handled inline in
            // the poll loop, where their feedback can be accurate; they never
            // reach this message-origin dispatch path.
        }
        InboundAction::Unsupported => {
            // Nothing to do by construction — it reached `route` only so its
            // id could advance the `getUpdates` offset.
        }
        InboundAction::RunCommand { .. }
        | InboundAction::ReportParseError { .. }
        | InboundAction::Unaimed { .. } => {
            // Answered inline in the poll loop, which is the only place that
            // can await the reply's send.
        }
    }
}

/// Answer a tapped callback with a toast, then rewrite the tapped message to
/// drop its now-consumed buttons and append the outcome. Both are best-effort
/// (logged on failure, never surfaced) — the decision itself was already applied
/// by the caller. `label` is the localized outcome; the message keeps its
/// original prompt text with the outcome appended.
async fn answer_and_edit(
    cx: &mut gpui::AsyncApp,
    token: &str,
    callback_id: String,
    callback_edit: Option<CallbackEdit>,
    label: &str,
) {
    let ack_token = token.to_string();
    let toast = label.to_string();
    let answered = cx
        .background_executor()
        .spawn(async move { client::answer_callback(&ack_token, &callback_id, Some(&toast)) })
        .await;
    trace::delivery("answer_callback", || {
        format!(
            "ok={} edits={} label={}",
            answered.is_ok(),
            callback_edit.is_some(),
            trace::preview(label)
        )
    });
    if let Err(e) = answered {
        LogWriter::log(
            ErrorReport::new("Telegram answerCallbackQuery failed")
                .severity(ErrorSeverity::Info)
                .from_error(&e)
                .at(file!(), line!())
                .dedup("telegram.answer_callback")
                .build(),
        );
    }

    let Some(edit) = callback_edit else {
        return;
    };
    let (edit_chat_id, edit_message_id) = (edit.chat_id, edit.message_id);
    let body = compose_edit_body(&edit.original_text, label);
    let edit_token = token.to_string();
    let edited = cx
        .background_executor()
        .spawn(async move {
            client::edit_message_text(&edit_token, edit.chat_id, edit.message_id, &body)
        })
        .await;
    trace::delivery("edit_message", || {
        format!(
            "chat_id={edit_chat_id} message_id={edit_message_id} ok={}",
            edited.is_ok()
        )
    });
    if let Err(e) = edited {
        LogWriter::log(
            ErrorReport::new("Telegram editMessageText failed")
                .severity(ErrorSeverity::Info)
                .from_error(&e)
                .at(file!(), line!())
                .dedup("telegram.edit_message")
                .build(),
        );
    }
}

/// The localized outcome label for a phone-tapped permission decision, shown
/// both as the callback toast and appended to the rewritten message. Applied →
/// the tapped direction (Allow/Reject); otherwise the reason it didn't apply.
fn permission_feedback(decision: &PermissionDecision, outcome: BotPermissionOutcome) -> String {
    match outcome {
        BotPermissionOutcome::Applied => match decision {
            PermissionDecision::Allow(_) => s::telegram_permission_allowed(),
            PermissionDecision::Reject(_) => s::telegram_permission_rejected(),
        },
        BotPermissionOutcome::Stale => s::telegram_permission_stale(),
        BotPermissionOutcome::Gone => s::telegram_permission_gone(),
    }
}

/// Compose the rewritten message body: the original prompt text with the outcome
/// appended on its own line, so the phone keeps the context of what was asked.
/// Falls back to just the label when the original text is unavailable.
fn compose_edit_body(original_text: &str, label: &str) -> String {
    if original_text.is_empty() {
        label.to_string()
    } else {
        format!("{original_text}\n\n— {label}")
    }
}

/// Enter every open `Workspace` window and run `on_match` against the one
/// whose persisted `WorkspaceUuid` equals `workspace` — the shared
/// `WindowRegistry::for_each_workspace` + `ws.uuid() == workspace` scaffolding
/// both [`InboundAction`] dispatch arms above need (a phone-relayed reply /
/// permission decision names its target pane by `PaneRef { workspace, pane }`,
/// but `PaneId` alone is only unique within one open window, so every window
/// must be checked). `pane.pane` itself is *not* checked against anything
/// here — the workspace-side handlers report a stale/gone pane id back to the
/// caller (`inject_bot_reply` returns `false`, `respond_bot_permission`
/// returns `Gone`), which then answers the phone rather than going quiet.
fn dispatch_to_workspace(
    cx: &mut gpui::AsyncApp,
    workspace: daruda_store::project::WorkspaceUuid,
    mut on_match: impl FnMut(&mut Workspace, &mut gpui::Context<Workspace>),
) {
    cx.update(|cx| {
        WindowRegistry::for_each_workspace(cx, |ws, _window, cx| {
            if ws.uuid() == workspace {
                on_match(ws, cx);
            }
        });
    });
}

/// Give a targetless action the target the app itself is already pointing at.
///
/// The bridge's selection and last-pinged pane are in-memory, so a restart
/// leaves the phone unable to reach anything until it is re-aimed by hand —
/// even though the lane it was talking to is right there, restored. This is
/// the last link in the chain `plain_text_target` walks, resolved here rather
/// than in `route` because only this layer can read a workspace.
///
/// Both outcomes rejoin an arm that already exists, so nothing downstream
/// learns a new shape: with a target, plain text is an `InjectPrompt` and an
/// unowned slash is an `UnknownSlash` for that pane to claim.
fn adopt_fallback_target(action: InboundAction, cx: &mut gpui::AsyncApp) -> InboundAction {
    let adopted = match &action {
        InboundAction::Unaimed { .. } | InboundAction::UnaimedSlash { .. } => {
            cx.update(control_resolve::sole_active_agent_chat)
        }
        _ => return action,
    };
    let Some(pane) = adopted else {
        return action;
    };
    trace::delivery("target.fallback", || {
        format!(
            "pane={} action={}",
            trace::pane(pane),
            trace::action_name(&action)
        )
    });
    match action {
        InboundAction::Unaimed { text } => InboundAction::InjectPrompt { pane, text },
        InboundAction::UnaimedSlash {
            name,
            text,
            suggestion,
        } => InboundAction::UnknownSlash {
            pane,
            name,
            text,
            suggestion,
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[test]
    fn compose_edit_body_appends_outcome_and_falls_back_when_empty() {
        assert_eq!(
            compose_edit_body("Allow write to /tmp/x?", "OK"),
            "Allow write to /tmp/x?\n\n— OK"
        );
        // No original text available → just the label, no stray separator.
        assert_eq!(compose_edit_body("", "OK"), "OK");
    }

    #[test]
    fn permission_feedback_distinguishes_direction_and_reason() {
        let allow = PermissionDecision::Allow("opt".into());
        let reject = PermissionDecision::Reject("opt".into());
        let applied_allow = permission_feedback(&allow, BotPermissionOutcome::Applied);
        let applied_reject = permission_feedback(&reject, BotPermissionOutcome::Applied);
        let stale = permission_feedback(&allow, BotPermissionOutcome::Stale);
        let gone = permission_feedback(&allow, BotPermissionOutcome::Gone);

        // Every outcome yields a distinct, non-empty label so the toast/edit is
        // never blank and Allow reads differently from Reject.
        for label in [&applied_allow, &applied_reject, &stale, &gone] {
            assert!(!label.is_empty());
        }
        assert_ne!(applied_allow, applied_reject);
        assert_ne!(applied_allow, stale);
        assert_ne!(stale, gone);
    }

    /// The restart case the whole fallback exists for: the bridge knows of no
    /// target, and the app's own active lane supplies one. Both targetless
    /// actions rejoin the arms that already had a pane, so `/usage` reaches
    /// the agent and plain text reaches the same chat.
    #[gpui::test]
    async fn a_targetless_action_adopts_the_apps_own_lane(cx: &mut TestAppContext) {
        use crate::test_support::workspace_with_agent_chat;

        let fixture = workspace_with_agent_chat(cx);
        let expected = fixture.pane_ref(cx);
        let mut async_cx = cx.to_async();

        assert_eq!(
            adopt_fallback_target(
                InboundAction::Unaimed {
                    text: "ship it".into()
                },
                &mut async_cx,
            ),
            InboundAction::InjectPrompt {
                pane: expected,
                text: "ship it".into()
            }
        );
        assert_eq!(
            adopt_fallback_target(
                InboundAction::UnaimedSlash {
                    name: "usage".into(),
                    text: "/usage".into(),
                    suggestion: Some("use"),
                },
                &mut async_cx,
            ),
            InboundAction::UnknownSlash {
                pane: expected,
                name: "usage".into(),
                text: "/usage".into(),
                suggestion: Some("use"),
            }
        );
    }

    /// Ambiguity has to survive the adopt step, not just be produced by it.
    /// Two windows each offering their own lane resolve to no candidate, and
    /// the action must come back untouched — rewriting it to a pane picked
    /// from the two would start a turn in whichever the walk saw first.
    #[gpui::test]
    async fn an_ambiguous_app_hands_the_action_back_untouched(cx: &mut TestAppContext) {
        use crate::test_support::workspace_with_agent_chat;

        let _windows = [workspace_with_agent_chat(cx), workspace_with_agent_chat(cx)];
        let mut async_cx = cx.to_async();

        let action = InboundAction::Unaimed {
            text: "ship it".into(),
        };
        assert_eq!(
            adopt_fallback_target(action.clone(), &mut async_cx),
            action,
            "two candidates is not a target"
        );
    }

    /// Every other action is returned as it came — the fallback must not
    /// touch a decision that already named its pane.
    #[gpui::test]
    async fn an_action_that_already_has_a_target_is_untouched(cx: &mut TestAppContext) {
        use crate::test_support::workspace_with_agent_chat;

        let fixture = workspace_with_agent_chat(cx);
        let pane = fixture.pane_ref(cx);
        let mut async_cx = cx.to_async();

        let action = InboundAction::InjectPrompt {
            pane,
            text: "already aimed".into(),
        };
        assert_eq!(adopt_fallback_target(action.clone(), &mut async_cx), action);
    }
}
