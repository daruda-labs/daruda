//! Telegram relay — `Workspace` ops that push agent-chat pings out to the
//! Telegram bridge (`crate::telegram::global::TelegramBridge`) and route
//! phone-tapped replies / permission decisions back into the triggering pane.
//!
//! Sibling of [`super::agent_chat_ops`], whose `maybe_notify_agent_event`
//! (permission wait) and `fire_activity_completion` (turn completion) tee points
//! call into this file's `relay_*` methods; both are `impl Workspace` blocks.

use std::time::Instant;

use gpui::Context;

use crate::surface::strings as s;
use crate::telegram::bridge::BotPermissionOutcome;
use crate::telegram::bridge::TelegramTail;
use crate::telegram::trace;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

use super::view::PromptDispatch;

mod turn;

pub(in crate::workspace) use turn::{FirstResponseOutcome, PhoneTurn};

/// Leading/trailing characters kept when a response is truncated — head carries
/// the ask, tail carries the result; the elided middle is usually tool-output
/// detail already visible in the app. Their sum is the threshold below which
/// text goes verbatim.
const TELEGRAM_PREVIEW_HEAD_CHARS: usize = 1000;
const TELEGRAM_PREVIEW_TAIL_CHARS: usize = 1000;
/// Maximum time a phone-triggered turn can stay silent before Telegram receives
/// the fixed fallback acknowledgement.
pub(in crate::workspace) const FIRST_RESPONSE_FALLBACK_SECS: u64 = 60;

/// The phone's slice of an agent response: the first
/// [`TELEGRAM_PREVIEW_HEAD_CHARS`] and last [`TELEGRAM_PREVIEW_TAIL_CHARS`]
/// characters around `marker` (the caller's localized "…(truncated)…" string).
///
/// This file owns the *budget* and the localized marker; the elision itself is
/// `crate::control::agent_text`'s, shared with the orchestrator's own reader —
/// which keeps a different amount and a fixed English marker.
fn preview_for(text: &str, marker: &str) -> String {
    crate::control::agent_text::elide_middle(
        text,
        TELEGRAM_PREVIEW_HEAD_CHARS,
        TELEGRAM_PREVIEW_TAIL_CHARS,
        marker,
    )
}

/// Compose the permission-wait ping's tail: the localized "waiting for input"
/// line, then optional tool-title and raw-input-summary lines (the same
/// `daruda_acp::PermissionItem` fields the in-app card is built from) so the
/// phone message names *what* is being asked. An empty string is treated as
/// absent (defensive; the source already filters it).
fn permission_wait_tail(tool_title: Option<&str>, raw_input_summary: Option<&str>) -> String {
    let mut tail = s::agent_notification_waiting();
    for line in [tool_title, raw_input_summary] {
        if let Some(line) = line.filter(|l| !l.is_empty()) {
            tail.push('\n');
            tail.push_str(line);
        }
    }
    tail
}

/// Compose the "went straight to a tool call" first-response ack's tail: the
/// fixed i18n label, plus the tool's title on its own line when the agent
/// supplied a non-empty one.
fn first_tool_ack_tail(tool_title: Option<&str>) -> String {
    let mut tail = s::agent_notification_telegram_first_tool_ack();
    if let Some(title) = tool_title.filter(|t| !t.is_empty()) {
        tail.push('\n');
        tail.push_str(title);
    }
    tail
}

/// Build one Telegram button per permission choice, in the same order and with
/// the same labels the in-app card uses (the same `daruda_acp::PermissionChoice`
/// list). A richer option set (e.g. codex-acp's "Allow Once" / "Allow for
/// Session" / execpolicy amendment, alongside Reject) stays fully choosable from
/// the phone — no collapsing to a single Allow/Reject pair. Each `*Once`/
/// `*Always` kind maps to the same `Allow`/`Reject` wire outcome (the kind only
/// picks in-app styling; the outcome is carried by `option_id`). Empty `Vec`
/// only when the agent supplied no options (caller then skips the relay).
fn permission_buttons(
    options: &[daruda_acp::PermissionChoice],
) -> Vec<(String, crate::telegram::bridge::PermissionDecision)> {
    use crate::telegram::bridge::PermissionDecision;
    use daruda_acp::PermissionKindView as Kind;
    options
        .iter()
        .map(|o| {
            let decision = match o.kind {
                Kind::AllowOnce | Kind::AllowAlways => {
                    PermissionDecision::Allow(o.option_id.clone())
                }
                Kind::RejectOnce | Kind::RejectAlways => {
                    PermissionDecision::Reject(o.option_id.clone())
                }
            };
            (o.name.clone(), decision)
        })
        .collect()
}

/// The pane's unresolved permission cards the phone has not been shown, in
/// card order. Cloned rather than borrowed because relaying needs `&mut
/// Workspace` while the view read is still live.
fn untold_permissions(view: &super::view::AgentChatView) -> Vec<daruda_acp::PermissionItem> {
    view.items
        .iter()
        .filter_map(|item| match item {
            daruda_acp::ChatItem::Permission(p)
                if p.resolved.is_none()
                    && view.pending_permissions.contains(&p.id)
                    && !view.permissions_told_to_phone.contains(&p.id) =>
            {
                Some(p.clone())
            }
            _ => None,
        })
        .collect()
}

impl Workspace {
    /// The owning project's display name, excluding workspace-owned chats
    /// and panes whose lane or project has gone away.
    fn project_name_for_pane(&self, pane_id: PaneId) -> Option<String> {
        let project_id = self.lane_ref_for_pane(pane_id)?.project;
        self.project_for(project_id).map(|p| p.name.clone())
    }

    /// The header line(s) for a pane's Telegram pings: project name + agent name
    /// when the pane's view is live, else the pane title. Shared by the
    /// completion and ack relays so they all read identically.
    pub(in crate::workspace) fn telegram_header(
        &self,
        pane_id: PaneId,
        cx: &Context<Self>,
    ) -> String {
        let project_line = self.project_name_for_pane(pane_id);
        match self.agent_chat_view(pane_id) {
            Some(view) => {
                let agent = view.read(cx).agent_name.clone();
                match project_line {
                    Some(project) => format!("{project}\n{agent}"),
                    None => agent,
                }
            }
            None => self.pane_title(pane_id, cx),
        }
    }

    /// Compose the turn-completion ping's header + tail: the plain header, and
    /// the agent's last response ([`preview_for`] truncates past 2000 chars) as
    /// a [`TelegramTail::Markdown`] tail so the phone shows what the agent
    /// actually said with its markdown rendered. Falls back to a
    /// [`TelegramTail::Plain`] "finished responding" tail when the turn produced
    /// no assistant text (e.g. tool-only) or the view is gone — that fallback is
    /// plain i18n copy, not agent markdown, so it must not be parsed as such.
    ///
    /// `None` when the sender already has this answer: a phone-dispatched turn
    /// whose last message is the very one the first-response relay sent has
    /// nothing left to report, and repeating it puts the same text on the
    /// phone twice.
    ///
    /// A pure query, so asking is free and asking twice is the same as asking
    /// once. [`Self::close_phone_turn`] is what ends the turn, and
    /// `fire_activity_completion` calls it whether or not this returned
    /// something to send.
    pub(super) fn telegram_completion_parts(
        &self,
        pane_id: PaneId,
        cx: &Context<Self>,
    ) -> Option<(String, TelegramTail)> {
        let header = self.telegram_header(pane_id, cx);
        let Some(view) = self.agent_chat_view(pane_id) else {
            return Some((
                self.pane_title(pane_id, cx),
                TelegramTail::Plain(s::agent_notification_completed()),
            ));
        };
        let view = view.read(cx);
        // A phone turn reports only what *it* said. Without the anchor the
        // scan runs back through the whole transcript, so a turn that
        // produced no text of its own hands the sender an earlier turn's
        // answer as though it were this one's. A turn the phone did not send
        // has no anchor to bound it by — that gap is the same one every
        // reader of this transcript has, and closing it needs a per-turn
        // anchor kept for every turn, not just these.
        let anchor = view.phone_turn().map_or(0, PhoneTurn::items_anchor);
        // Skips a message with no text for the same reason `first_response`
        // does: it would put an empty preview under the notification header.
        let last_response = view.items[anchor.min(view.items.len())..]
            .iter()
            .rev()
            .find_map(|item| match item {
                daruda_acp::ChatItem::AssistantText {
                    text, message_id, ..
                } if !text.trim().is_empty() => Some((text.as_str(), message_id.as_deref())),
                _ => None,
            });
        match last_response {
            Some((_, message_id))
                if view
                    .phone_turn()
                    .is_some_and(|turn| turn.already_sent(message_id)) =>
            {
                None
            }
            Some((text, _)) => Some((
                header,
                TelegramTail::Markdown(preview_for(
                    text,
                    &s::agent_notification_telegram_truncated_marker(),
                )),
            )),
            None => Some((
                header,
                TelegramTail::Plain(s::agent_notification_completed()),
            )),
        }
    }

    /// End the turn's phone conversation. Separate from
    /// [`Self::telegram_completion_parts`] because that one is a question and
    /// this is the effect: composing a ping must not be what retires the turn,
    /// or a second reader of the same question would silently retire it.
    pub(super) fn close_phone_turn(&self, pane_id: PaneId, cx: &mut Context<Self>) {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        view.update(cx, |v, _| v.end_phone_turn());
    }

    /// Relay a permission-wait ping with one button per option the agent
    /// offered — skips the relay entirely (not a broken partial ping) if
    /// the agent supplied no options at all (shouldn't happen for a real
    /// permission request, but nothing to build a keyboard from if it
    /// somehow does). `tool_title` / `raw_input_summary` are the same
    /// `daruda_acp::PermissionItem` fields the in-app card is built
    /// from — see [`permission_wait_tail`] — so the phone ping names the
    /// actual action awaiting approval instead of just "waiting for your
    /// input". A permission wait during a phone-started turn bypasses
    /// presence — the first visible response directly here, later ones
    /// through the ledger check in [`Self::relay_when_presence_allows`].
    /// Always a [`TelegramTail::Plain`] tail: none of
    /// `tool_title`, `raw_input_summary`, or the "waiting" label is
    /// agent-authored markdown — see [`TelegramTail`]'s doc comment for why
    /// that matters.
    pub(super) fn relay_permission_wait_to_telegram(
        &mut self,
        pane_id: PaneId,
        perm_id: u64,
        options: &[daruda_acp::PermissionChoice],
        tool_title: Option<&str>,
        raw_input_summary: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let is_telegram_first_response = self
            .agent_chat_view(pane_id)
            .is_some_and(|view| view.read(cx).is_phone_turn_waiting());
        let buttons = permission_buttons(options);
        if buttons.is_empty() {
            if is_telegram_first_response {
                self.relay_first_response_fallback_to_telegram(pane_id, cx);
            }
            return;
        }
        let tail = permission_wait_tail(tool_title, raw_input_summary);
        let header = self.pane_title(pane_id, cx);
        let permission = Some(crate::telegram::bridge::PermissionPromptRef { perm_id, buttons });
        let told = if is_telegram_first_response {
            self.relay_to_telegram(pane_id, header, TelegramTail::Plain(tail), permission, cx);
            true
        } else {
            self.relay_when_presence_allows(
                pane_id,
                header,
                TelegramTail::Plain(tail),
                permission,
                cx,
            )
        };
        if told {
            self.mark_permission_told_to_phone(pane_id, perm_id, cx);
        }
    }

    /// Record that the phone has been shown this permission, so the periodic
    /// re-ask leaves it alone.
    fn mark_permission_told_to_phone(&self, pane_id: PaneId, perm_id: u64, cx: &mut Context<Self>) {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        view.update(cx, |v, _| {
            v.permissions_told_to_phone.insert(perm_id);
        });
    }

    /// Re-offer permission prompts the phone has never been shown, for every
    /// pane whose request is still outstanding.
    ///
    /// This is not the deferral queue coming back. A completion ping reports a
    /// past event, so holding one and flushing it later delivers something
    /// stale; a permission request is a *live blocking state* — the agent is
    /// stopped until someone answers. A request still outstanding when absence
    /// is first detected is as true then as when it fired, so relaying it is a
    /// fresh ping about the present, not a late one about the past. Nothing is
    /// stored but the request id: the text is recomposed from the live card,
    /// so a resolved request simply stops appearing.
    ///
    /// Level-triggered on the periodic pump, mirroring
    /// [`Self::flush_telegram_first_response_fallbacks`].
    pub(crate) fn relay_outstanding_permissions(&mut self, cx: &mut Context<Self>) {
        let panes: Vec<PaneId> = self.every_agent_chat().map(|(id, _)| id).collect();
        for pane_id in panes {
            let Some(view) = self.agent_chat_view(pane_id).cloned() else {
                continue;
            };
            // Drop bookkeeping for requests that have since been answered, so
            // the set cannot outgrow the outstanding ones.
            let untold = view.update(cx, |v, _| {
                v.permissions_told_to_phone
                    .retain(|id| v.pending_permissions.contains(id));
                untold_permissions(v)
            });
            for prompt in untold {
                self.relay_permission_wait_to_telegram(
                    pane_id,
                    prompt.id,
                    &prompt.options,
                    prompt.tool_title.as_deref(),
                    prompt.raw_input_summary.as_deref(),
                    cx,
                );
            }
        }
    }

    /// Presence-gated entry point for the completion and permission relays.
    /// Asks `app_presence` whether the user is away and acts on the answer
    /// immediately: sent, or dropped for good. Nothing is queued, so a
    /// ping's send time is always its settle time — a later absence does not
    /// resurrect a ping this call declined.
    ///
    /// First-response acks and first-response permission waits deliberately
    /// bypass this and call [`Self::relay_to_telegram`] directly: the phone
    /// asked for those, so presence is not what decides them. The same
    /// reasoning covers the rest of a phone-started turn — while the pane's
    /// [`PhoneTurn`] ledger is open, every relay here is solicited too, and
    /// goes out the same way. Presence decides only the unsolicited.
    ///
    /// Returns whether the ping went out, so a caller tracking a live state
    /// (an outstanding permission) knows whether the phone has been told.
    pub(super) fn relay_when_presence_allows(
        &mut self,
        pane_id: PaneId,
        header: String,
        tail: TelegramTail,
        permission: Option<crate::telegram::bridge::PermissionPromptRef>,
        cx: &mut Context<Self>,
    ) -> bool {
        // The ledger closes at the completion tee, after the completion relay
        // has run — so a completion and a second permission wait both land
        // here with it still open.
        if self.phone_turn_open(pane_id, cx) {
            trace::delivery("relay.solicited", || {
                format!("pane={pane_id} text={}", trace::tail_digest(&tail))
            });
            self.relay_to_telegram(pane_id, header, tail, permission, cx);
            return true;
        }
        // `relay_to_telegram` asks this too, but asking here first keeps a
        // disabled/unpaired bridge out of the presence trace below, where it
        // would read as a presence decision it never was.
        let away = crate::app_presence::is_away(cx);
        let remote_sent = crate::remote_channel::global::RemoteChannels::send_ping(
            self.remote_ping(pane_id, header.clone(), tail.clone(), permission.clone()),
            crate::remote_channel::global::Delivery::Presence { away },
            cx,
        );
        if self.telegram_bridge(cx).is_none() {
            trace::delivery("relay.gated", || {
                format!("pane={pane_id} entry=presence reason=bridge")
            });
            return remote_sent;
        }
        // Asked unconditionally, not short-circuited by the opt-out, so the
        // trace records what presence actually was even when it did not
        // decide — the alternative logs a sample from the last pump tick.
        let send = away || !self.telegram.only_when_away;
        let state = crate::app_presence::snapshot(cx);
        let away_secs = state.away_secs(Instant::now());
        // `relay.send` belongs to `relay_to_telegram`; this line is the
        // verdict that precedes it, and the only record of a drop. The rule is
        // logged alongside the readings because it is read live from config —
        // a log reader has no other way to recover what they were compared to.
        let rule = crate::app_presence::rule(cx);
        trace::delivery("relay.presence", || {
            format!(
                "pane={pane_id} send={send} away={away} only_when_away={} \
                 away_secs={away_secs:?} idle_secs={} away_grace_secs={} \
                 away_idle_secs={} away_idle_foreground_secs={} {} text={}",
                self.telegram.only_when_away,
                trace::opt(state.idle().map(|i| i.as_secs())),
                rule.grace.as_secs(),
                rule.idle_bar.as_secs(),
                rule.foreground_idle_bar.as_secs(),
                trace::presence(
                    away_secs.is_none(),
                    self.lane_ref_for_pane(pane_id),
                    self.active_ref()
                ),
                trace::tail_digest(&tail)
            )
        });
        if send {
            self.relay_telegram_only(pane_id, header, tail, permission, cx);
        }
        send || remote_sent
    }

    /// Whether the pane's in-flight turn was started from the phone. Any
    /// ledger state counts, not just `Waiting`: an answered first response
    /// does not make the rest of the turn unsolicited. The ledger lives as
    /// long as the activity span the phone started, so a desk prompt queued
    /// behind it shares that span and its delivery — the span settles once,
    /// and the phone hears how it ended.
    fn phone_turn_open(&self, pane_id: PaneId, cx: &Context<Self>) -> bool {
        self.agent_chat_view(pane_id)
            .is_some_and(|view| view.read(cx).phone_turn().is_some())
    }

    /// Relay a ping to the Telegram bridge, if the bridge is configured to
    /// receive one. Gated on BOTH `enabled` (the user turned the feature on)
    /// AND `authorized_chat_id.is_some()` (someone has actually paired) — an
    /// enabled-but-unpaired bridge has nowhere to send, and skipping here
    /// avoids `BridgeCore::build_ping`'s debug-assert/zero-chat_id fallback
    /// path for the common "just turned it on, haven't paired yet" state.
    ///
    /// `cx.try_global` (not `cx.global`, which panics if missing) because
    /// `main.rs` opens the first window before calling
    /// `telegram::global::install` — a `Workspace` can theoretically exist
    /// for a brief window before the `TelegramBridge` global is registered.
    pub(super) fn relay_to_telegram(
        &self,
        pane_id: PaneId,
        header: String,
        tail: TelegramTail,
        permission: Option<crate::telegram::bridge::PermissionPromptRef>,
        cx: &Context<Self>,
    ) {
        crate::remote_channel::global::RemoteChannels::send_ping(
            self.remote_ping(pane_id, header.clone(), tail.clone(), permission.clone()),
            crate::remote_channel::global::Delivery::Explicit,
            cx,
        );
        self.relay_telegram_only(pane_id, header, tail, permission, cx);
    }

    fn remote_ping(
        &self,
        pane_id: PaneId,
        header: String,
        tail: TelegramTail,
        permission: Option<crate::remote_channel::bridge::PermissionPromptRef>,
    ) -> crate::remote_channel::bridge::BridgePing {
        crate::remote_channel::bridge::BridgePing {
            pane: crate::remote_channel::bridge::PaneRef {
                workspace: self.uuid(),
                pane: pane_id,
            },
            header,
            tail,
            permission,
        }
    }

    fn relay_telegram_only(
        &self,
        pane_id: PaneId,
        header: String,
        tail: TelegramTail,
        permission: Option<crate::remote_channel::bridge::PermissionPromptRef>,
        cx: &Context<Self>,
    ) {
        let Some(bridge) = self.telegram_bridge(cx) else {
            trace::delivery("relay.gated", || {
                format!("pane={pane_id} entry=direct reason=bridge")
            });
            return;
        };
        trace::delivery("relay.send", || {
            format!(
                "pane={pane_id} {} text={}",
                trace::presence(
                    crate::platform::attention::is_app_active(),
                    self.lane_ref_for_pane(pane_id),
                    self.active_ref()
                ),
                trace::tail_digest(&tail)
            )
        });
        bridge.send(crate::telegram::bridge::BridgePing {
            pane: crate::telegram::bridge::PaneRef {
                workspace: self.uuid(),
                pane: pane_id,
            },
            header,
            tail,
            permission,
        });
    }

    /// The live bridge, or `None` when nothing may be sent — the feature is
    /// off, no chat is paired, or the global is not installed yet (early
    /// startup, tests).
    ///
    /// The one place that question is answered. Every relay below asks it, so
    /// they cannot come to different conclusions about the same three
    /// conditions, and a fourth relay inherits the rule instead of copying it.
    fn telegram_bridge<'a>(
        &self,
        cx: &'a Context<Self>,
    ) -> Option<&'a crate::telegram::global::TelegramBridge> {
        if !(self.telegram.enabled && self.telegram.authorized_chat_id.is_some()) {
            return None;
        }
        cx.try_global::<crate::telegram::global::TelegramBridge>()
    }

    /// Relay text that belongs to no pane.
    ///
    /// Lives beside [`Self::relay_to_telegram`] despite not being about a chat
    /// pane, because both go through [`Self::telegram_bridge`] — keeping the
    /// two callers of that gate together is what stops it being re-derived.
    ///
    /// Not presence-gated, unlike a ping. This answers a command the phone
    /// sent, so dropping it because the user is at the desktop would be
    /// backwards — the phone asked, the phone gets the answer.
    pub(in crate::workspace) fn relay_notice_to_telegram(&self, text: String, cx: &Context<Self>) {
        crate::remote_channel::global::RemoteChannels::send_notice(text.clone(), cx);
        let Some(bridge) = self.telegram_bridge(cx) else {
            trace::delivery("relay.gated", || {
                format!("entry=notice reason=bridge text={}", trace::digest(&text))
            });
            return;
        };
        bridge.send_notice(text);
    }

    /// Send the "queued behind the current turn" notice — fires the instant
    /// `AgentChatView::send_prompt_text_for_telegram` reports
    /// [`PromptDispatch::Queued`], since a queued reply hasn't reached the
    /// agent yet and there is nothing to watch a first response for. Plain
    /// tail (fixed i18n copy). Goes through [`Self::relay_to_telegram`], so
    /// the bridge gate applies but presence does not.
    pub(super) fn relay_queued_notice_to_telegram(&self, pane_id: PaneId, cx: &Context<Self>) {
        let header = self.telegram_header(pane_id, cx);
        self.relay_to_telegram(
            pane_id,
            header,
            TelegramTail::Plain(s::agent_notification_telegram_reply_queued()),
            None,
            cx,
        );
    }

    /// Send the "queue is full" notice. Same shape as the queued notice next
    /// door, but a different fact: that one says "later", this one says "not
    /// at all".
    pub(super) fn relay_queue_full_notice_to_telegram(&self, pane_id: PaneId, cx: &Context<Self>) {
        let header = self.telegram_header(pane_id, cx);
        self.relay_to_telegram(
            pane_id,
            header,
            TelegramTail::Plain(s::agent_notification_telegram_queue_full()),
            None,
            cx,
        );
    }

    /// Relay a phone-triggered turn's resolved first response: the agent's own
    /// text (markdown, truncated like the completion ping) or a fixed
    /// "checking via tool" note naming it. Bypasses the presence-defer gate
    /// (calls `relay_to_telegram` directly) — the whole point of this relay is
    /// a phone-originated interaction, so instant delivery is what the sender
    /// wants regardless of whether the app happens to be foreground right now.
    pub(super) fn relay_first_response_to_telegram(
        &self,
        pane_id: PaneId,
        outcome: FirstResponseOutcome,
        cx: &Context<Self>,
    ) {
        let header = self.telegram_header(pane_id, cx);
        let tail = match outcome {
            FirstResponseOutcome::Text { text, .. } => TelegramTail::Markdown(preview_for(
                &text,
                &s::agent_notification_telegram_truncated_marker(),
            )),
            FirstResponseOutcome::Tool { tool_title } => {
                TelegramTail::Plain(first_tool_ack_tail(tool_title.as_deref()))
            }
        };
        self.relay_to_telegram(pane_id, header, tail, None, cx);
    }

    /// Relay the fixed fallback ack for a phone-triggered turn that went 60s
    /// without producing text or a tool call, or settled with nothing having
    /// appeared at all. Plain tail (fixed i18n copy, not agent-authored
    /// markdown) — the same copy the old always-immediate ack used. Gated by
    /// `relay_to_telegram`.
    pub(super) fn relay_first_response_fallback_to_telegram(
        &self,
        pane_id: PaneId,
        cx: &Context<Self>,
    ) {
        let header = self.telegram_header(pane_id, cx);
        self.relay_to_telegram(
            pane_id,
            header,
            TelegramTail::Plain(s::agent_notification_telegram_reply_ack()),
            None,
            cx,
        );
    }

    /// Periodic safety net for phone-triggered turns that produce no completed
    /// assistant text or tool call within [`FIRST_RESPONSE_FALLBACK_SECS`].
    /// The view moves each turn it takes to `Answered`, so a pane can emit
    /// this fallback at most once per phone-dispatched turn.
    pub(crate) fn flush_telegram_first_response_fallbacks(&mut self, cx: &mut Context<Self>) {
        let now = std::time::Instant::now();
        let panes: Vec<PaneId> = self.every_agent_chat().map(|(id, _)| id).collect();

        for pane_id in panes {
            let Some(view) = self.agent_chat_view(pane_id).cloned() else {
                continue;
            };
            let overdue = view.update(cx, |v, _| {
                v.take_phone_fallback_if_overdue(now, FIRST_RESPONSE_FALLBACK_SECS)
            });
            if overdue {
                self.relay_first_response_fallback_to_telegram(pane_id, cx);
            }
        }
    }

    /// The workspace's persisted identity — needed by cross-cutting
    /// App-level services (e.g. the Telegram bridge,
    /// `crate::telegram::global`) that route by `WorkspaceUuid` since
    /// `PaneId` alone is only unique within one workspace, not across
    /// all open windows.
    pub(crate) fn uuid(&self) -> daruda_store::project::WorkspaceUuid {
        self.uuid
    }

    /// Inject a phone-relayed reply as a prompt in this pane through the same
    /// Workspace submit funnel as the bottom-dock composer. If it has to queue
    /// behind an in-flight turn, sends the immediate "queued" notice (there's
    /// nothing yet to watch a first response for); otherwise it dispatches
    /// straight onto the wire and the view itself arms the first-response
    /// watch. Local-only slash commands never reach the agent, so they receive
    /// the fixed fallback ack immediately. A `pub(crate)` entry point for
    /// `crate::telegram::global`'s poll loop to call into (which lives outside
    /// `workspace/` and can't reach the `pub(super)` version).
    /// `false` means the pane is gone. Reported rather than swallowed: the
    /// caller resolves the target from a *remembered* selection or last-pinged
    /// pane, either of which can name a pane the user has since closed — and a
    /// silent return there loses the message and leaves the same dead target in
    /// place for every message after it.
    pub(crate) fn inject_bot_reply(
        &mut self,
        pane_id: PaneId,
        text: String,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.agent_chat_view(pane_id).is_none() {
            return false;
        }
        match self.send_agent_prompt_text_from_telegram(pane_id, text, cx) {
            Some(PromptDispatch::Queued) => self.relay_queued_notice_to_telegram(pane_id, cx),
            // Reported, not lost: the sender is on a phone and would otherwise
            // read the silence as the prompt having been accepted.
            Some(PromptDispatch::QueueFull) => {
                self.relay_queue_full_notice_to_telegram(pane_id, cx)
            }
            Some(PromptDispatch::SentNow) => {}
            None => self.relay_first_response_fallback_to_telegram(pane_id, cx),
        }
        true
    }

    /// Resolve a phone-tapped Allow/Reject button against this pane's
    /// currently-outstanding permission. Delegates to the existing
    /// `AgentChatView::respond_permission`, which already: resolves
    /// the card carrying this `perm_id`, sends the decision over the ACP
    /// session, and reflows the row list — the same path the in-app
    /// buttons use.
    ///
    /// The bridge's `perm_id` (captured when the ping was built) must still be
    /// outstanding. A stale phone button should not resolve a request the user
    /// already answered in-app or that was cancelled.
    pub(crate) fn respond_bot_permission(
        &mut self,
        pane_id: PaneId,
        perm_id: u64,
        decision: crate::telegram::bridge::PermissionDecision,
        cx: &mut Context<Self>,
    ) -> BotPermissionOutcome {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return BotPermissionOutcome::Gone;
        };
        if !view.read(cx).is_permission_outstanding(perm_id) {
            return BotPermissionOutcome::Stale;
        }
        let (option_id, kind) = match decision {
            crate::telegram::bridge::PermissionDecision::Allow(id) => {
                (id, daruda_acp::PermissionKindView::AllowOnce)
            }
            crate::telegram::bridge::PermissionDecision::Reject(id) => {
                (id, daruda_acp::PermissionKindView::RejectOnce)
            }
        };
        view.update(cx, |v, cx| {
            v.respond_permission(perm_id, option_id, kind, cx)
        });
        BotPermissionOutcome::Applied
    }
}

#[cfg(test)]
mod tests;
