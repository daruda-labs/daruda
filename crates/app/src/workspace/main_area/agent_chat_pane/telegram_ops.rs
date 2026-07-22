//! Telegram relay — `Workspace` ops that push agent-chat pings out to the
//! Telegram bridge (`crate::telegram::global::TelegramBridge`) and route
//! phone-tapped replies / permission decisions back into the triggering pane.
//!
//! Sibling of [`super::agent_chat_ops`], whose `maybe_notify_agent_event`
//! (permission wait) and `fire_activity_completion` (turn completion) tee points
//! call into this file's `relay_*` methods; both are `impl Workspace` blocks.

use std::collections::HashSet;

use gpui::Context;

use crate::surface::strings as s;
use crate::telegram::bridge::BotPermissionOutcome;
use crate::telegram::bridge::TelegramTail;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

use super::view::PromptDispatch;

/// Below this char count, [`preview_for`] sends the text verbatim.
const TELEGRAM_PREVIEW_THRESHOLD: usize = 2000;
/// Leading/trailing characters kept when a response is truncated — head carries
/// the ask, tail carries the result; the elided middle is usually tool-output
/// detail already visible in the app.
const TELEGRAM_PREVIEW_HEAD_CHARS: usize = 1000;
const TELEGRAM_PREVIEW_TAIL_CHARS: usize = 1000;

/// Keep `text` verbatim under [`TELEGRAM_PREVIEW_THRESHOLD`] characters; past
/// it, keep the first [`TELEGRAM_PREVIEW_HEAD_CHARS`] and last
/// [`TELEGRAM_PREVIEW_TAIL_CHARS`] characters around `marker` (the caller's
/// localized "…(truncated)…" string). Counts `char`s, not bytes, so a multi-byte
/// response never splits mid-character.
fn preview_for(text: &str, marker: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= TELEGRAM_PREVIEW_THRESHOLD {
        return text.to_string();
    }
    let head: String = chars[..TELEGRAM_PREVIEW_HEAD_CHARS].iter().collect();
    let tail: String = chars[chars.len() - TELEGRAM_PREVIEW_TAIL_CHARS..]
        .iter()
        .collect();
    format!("{head}\n{marker}\n{tail}")
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
// TODO(telegram-first-response-ack task 3): called from
// `relay_first_response_to_telegram` once the event pump wires that in —
// remove this `allow` at that point.
#[cfg_attr(not(test), allow(dead_code))]
fn first_tool_ack_tail(tool_title: Option<&str>) -> String {
    let mut tail = s::agent_notification_telegram_first_tool_ack();
    if let Some(title) = tool_title.filter(|t| !t.is_empty()) {
        tail.push('\n');
        tail.push_str(title);
    }
    tail
}

/// Which relay a deferred entry represents. `Permission` carries the request id
/// so a late-delivered ping can be dropped if that request was already resolved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum DeferKind {
    Completion,
    PostTurn,
    Permission { perm_id: u64 },
}

/// A composed Telegram ping held back because the user was present when it fired.
/// `queued_at` anchors [`ready_to_deliver`]'s quiet-window check to *this ping's
/// own* settle time rather than to whatever raw idle streak preceded it.
#[derive(Clone)]
pub(in crate::workspace) struct DeferredRelay {
    pub kind: DeferKind,
    pub header: String,
    pub tail: TelegramTail,
    pub permission: Option<crate::telegram::bridge::PermissionPromptRef>,
    pub queued_at: std::time::Instant,
}

/// Cap on entries held per pane. `Completion` is deduped to at most one by
/// [`push_deferred`], but `PostTurn`/`Permission` accumulate freely — without a
/// bound, a pane that stays continuously present (never triggers a flush) could
/// grow this Vec without limit. Oldest entries are evicted first, mirroring
/// `BridgeCore`'s `SENT_PINGS_CAP` eviction in `telegram::bridge`.
const MAX_DEFERRED_PER_PANE: usize = 20;

/// Hold a presence-gated relay instead of sending now: the feature is on and
/// the app is foreground. Whether a *held* ping is later actually delivered is
/// decided per-entry by [`ready_to_deliver`] — this only gates whether it goes
/// into the queue at all. Pure so it is unit-testable without a live
/// `NSApplication`.
pub(in crate::workspace) fn should_defer_relay(enabled: bool, app_active: bool) -> bool {
    enabled && app_active
}

/// Whether a held ping, queued at `queued_at`, is ready to actually go out now.
/// The user leaving the app delivers immediately regardless of age — there's no
/// reason to keep waiting once they're gone. While still foregrounded, delivery
/// additionally requires BOTH: at least `quiet_secs` of real time since *this*
/// ping was queued, AND at least `quiet_secs` of current system-input
/// idleness — i.e. a full quiet window that starts at this ping's own settle
/// time, not at whatever idle streak happened to precede it (a long turn the
/// user watches without touching input must not itself burn down the window).
/// Pure so it is unit-testable without a live `NSApplication`/HID query.
pub(in crate::workspace) fn ready_to_deliver(
    queued_at: std::time::Instant,
    now: std::time::Instant,
    app_active: bool,
    idle_secs: f64,
    quiet_secs: u64,
) -> bool {
    if !app_active {
        return true;
    }
    let quiet_secs = quiet_secs as f64;
    let elapsed_since_queued = now.saturating_duration_since(queued_at).as_secs_f64();
    elapsed_since_queued >= quiet_secs && idle_secs >= quiet_secs
}

/// Append a deferred relay to a pane's pending queue, keeping at most one
/// pending `Completion` (a newer completion supersedes an older one's "last
/// response"); post-turn deltas and permissions accumulate, bounded by
/// [`MAX_DEFERRED_PER_PANE`] (oldest evicted first).
fn push_deferred(queue: &mut Vec<DeferredRelay>, entry: DeferredRelay) {
    if entry.kind == DeferKind::Completion {
        queue.retain(|e| e.kind != DeferKind::Completion);
    }
    queue.push(entry);
    while queue.len() > MAX_DEFERRED_PER_PANE {
        queue.remove(0);
    }
}

/// Splits a pane's deferred queue into `(ready, still_holding)`: a permission
/// ping whose `perm_id` is no longer outstanding (resolved or cancelled
/// in-app) is dropped outright — never delivered, never re-queued. Of the
/// rest, an entry moves to `ready` when [`ready_to_deliver`] says so; anything
/// not yet ready is returned in `still_holding` for a later flush tick.
pub(in crate::workspace) fn partition_deferred(
    queue: Vec<DeferredRelay>,
    live_perms: &HashSet<u64>,
    now: std::time::Instant,
    app_active: bool,
    idle_secs: f64,
    quiet_secs: u64,
) -> (Vec<DeferredRelay>, Vec<DeferredRelay>) {
    let mut ready = Vec::new();
    let mut still_holding = Vec::new();
    for entry in queue {
        if let DeferKind::Permission { perm_id } = entry.kind
            && !live_perms.contains(&perm_id)
        {
            continue;
        }
        if ready_to_deliver(entry.queued_at, now, app_active, idle_secs, quiet_secs) {
            ready.push(entry);
        } else {
            still_holding.push(entry);
        }
    }
    (ready, still_holding)
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

/// What a chat item appended since a [`FirstResponseWatch`] started resolves
/// to, if anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) enum FirstResponseOutcome {
    /// The agent's first visible reply was text.
    Text(String),
    /// The agent went straight to a tool call with no preceding text.
    /// `tool_title` names it when the agent supplied a non-empty title.
    Tool { tool_title: Option<String> },
}

/// Tracks whether a pane's in-flight turn was dispatched from a phone-injected
/// reply and, if so, since when — so the caller knows both whether to keep
/// waiting and where in `items` to resume scanning from. Owned by
/// `AgentChatView` as `Option<FirstResponseWatch>`; `None` means no phone-
/// triggered turn is currently being watched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) struct FirstResponseWatch {
    started_at: std::time::Instant,
    items_len_at_start: usize,
}

impl FirstResponseWatch {
    /// Starts a watch anchored at `now`, scanning `items` from `items_len`
    /// onward (the length of the pane's chat items at the moment this turn's
    /// prompt was echoed — so the echoed `UserText` itself is never mistaken
    /// for a response).
    pub(in crate::workspace) fn start(now: std::time::Instant, items_len: usize) -> Self {
        Self {
            started_at: now,
            items_len_at_start: items_len,
        }
    }

    /// Whether `timeout_secs` has elapsed since this watch started with
    /// nothing qualifying found yet — the periodic fallback pump's readiness
    /// check.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::workspace) fn is_overdue(
        &self,
        now: std::time::Instant,
        timeout_secs: u64,
    ) -> bool {
        now.saturating_duration_since(self.started_at).as_secs() >= timeout_secs
    }

    /// The first item appended since this watch started that resolves it, if
    /// any — `None` means keep watching. A still-streaming `AssistantText`
    /// and any `Thinking` item are skipped: reasoning isn't the visible
    /// reply, and a text reply must be complete before it's worth sending
    /// (see `daruda_acp::ChatItem::AssistantText`'s `streaming` field).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::workspace) fn resolve(
        &self,
        items: &[daruda_acp::ChatItem],
    ) -> Option<FirstResponseOutcome> {
        items
            .get(self.items_len_at_start..)?
            .iter()
            .find_map(|item| match item {
                daruda_acp::ChatItem::AssistantText {
                    text,
                    streaming: false,
                    ..
                } => Some(FirstResponseOutcome::Text(text.clone())),
                daruda_acp::ChatItem::ToolCall(tool) => Some(FirstResponseOutcome::Tool {
                    tool_title: Some(tool.title.clone()).filter(|t| !t.is_empty()),
                }),
                _ => None,
            })
    }
}

impl Workspace {
    /// The owning project's display name for `pane_id` (found via whichever
    /// `main_area.runtimes` entry's pane list contains it). `None` for a pane
    /// whose lane/project has since gone away.
    fn project_name_for_pane(&self, pane_id: PaneId) -> Option<String> {
        let project_id = self
            .main_area
            .runtimes
            .iter()
            .find(|(_, rt)| rt.panes.iter().any(|p| p.id == pane_id))
            .map(|(lane_ref, _)| lane_ref.project)?;
        self.project_for(project_id).map(|p| p.name.clone())
    }

    /// The header line(s) for a pane's Telegram pings: project name + agent name
    /// when the pane's view is live, else the pane title. Shared by the
    /// completion, ack, and post-turn relays so they all read identically.
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
    pub(in crate::workspace) fn telegram_completion_parts(
        &self,
        pane_id: PaneId,
        cx: &Context<Self>,
    ) -> (String, TelegramTail) {
        let header = self.telegram_header(pane_id, cx);
        let Some(view) = self.agent_chat_view(pane_id) else {
            return (
                self.pane_title(pane_id, cx),
                TelegramTail::Plain(s::agent_notification_completed()),
            );
        };
        let view = view.read(cx);
        let last_response = view.items.iter().rev().find_map(|item| match item {
            daruda_acp::ChatItem::AssistantText { text, .. } => Some(text.as_str()),
            _ => None,
        });
        match last_response {
            Some(text) => (
                header,
                TelegramTail::Markdown(preview_for(
                    text,
                    &s::agent_notification_telegram_truncated_marker(),
                )),
            ),
            None => (
                header,
                TelegramTail::Plain(s::agent_notification_completed()),
            ),
        }
    }

    /// Relay a permission-wait ping with one button per option the agent
    /// offered — skips the relay entirely (not a broken partial ping) if
    /// the agent supplied no options at all (shouldn't happen for a real
    /// permission request, but nothing to build a keyboard from if it
    /// somehow does). `tool_title` / `raw_input_summary` are the same
    /// `daruda_acp::PermissionItem` fields the in-app card is built
    /// from — see [`permission_wait_tail`] — so the phone ping names the
    /// actual action awaiting approval instead of just "waiting for your
    /// input". Always a [`TelegramTail::Plain`] tail: none of `tool_title`,
    /// `raw_input_summary`, or the "waiting" label is agent-authored
    /// markdown — see [`TelegramTail`]'s doc comment for why that matters.
    pub(in crate::workspace) fn relay_permission_wait_to_telegram(
        &mut self,
        pane_id: PaneId,
        perm_id: u64,
        options: &[daruda_acp::PermissionChoice],
        tool_title: Option<&str>,
        raw_input_summary: Option<&str>,
        cx: &Context<Self>,
    ) {
        let buttons = permission_buttons(options);
        if buttons.is_empty() {
            return;
        }
        let tail = permission_wait_tail(tool_title, raw_input_summary);
        self.relay_or_defer_to_telegram(
            pane_id,
            DeferKind::Permission { perm_id },
            self.pane_title(pane_id, cx),
            TelegramTail::Plain(tail),
            Some(crate::telegram::bridge::PermissionPromptRef { perm_id, buttons }),
            cx,
        );
    }

    /// Presence-gated entry point for the completion / permission / post-turn
    /// relays. While the user is present (app foreground) the composed ping is
    /// held per-pane; the periodic flush (`Workspace::flush_deferred_telegram`)
    /// later decides, per entry, when it's actually ready to go out (see
    /// `ready_to_deliver`). Otherwise it is sent immediately. Ack deliberately
    /// bypasses this and calls `relay_to_telegram` directly.
    pub(in crate::workspace) fn relay_or_defer_to_telegram(
        &mut self,
        pane_id: PaneId,
        kind: DeferKind,
        header: String,
        tail: TelegramTail,
        permission: Option<crate::telegram::bridge::PermissionPromptRef>,
        cx: &Context<Self>,
    ) {
        // Preserve `relay_to_telegram`'s drop-when-not-ready semantics: never
        // stash (or send) a ping while disabled, unpaired, or before the
        // bridge global is installed.
        if !(self.telegram.enabled && self.telegram.authorized_chat_id.is_some()) {
            return;
        }
        if cx
            .try_global::<crate::telegram::global::TelegramBridge>()
            .is_none()
        {
            return;
        }
        let defer = should_defer_relay(
            self.telegram.defer_while_active,
            crate::platform::attention::is_app_active(),
        );
        if defer {
            push_deferred(
                self.deferred_telegram.entry(pane_id).or_default(),
                DeferredRelay {
                    kind,
                    header,
                    tail,
                    permission,
                    queued_at: std::time::Instant::now(),
                },
            );
        } else {
            self.relay_to_telegram(pane_id, header, tail, permission, cx);
        }
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
    pub(in crate::workspace) fn relay_to_telegram(
        &self,
        pane_id: PaneId,
        header: String,
        tail: TelegramTail,
        permission: Option<crate::telegram::bridge::PermissionPromptRef>,
        cx: &Context<Self>,
    ) {
        if !(self.telegram.enabled && self.telegram.authorized_chat_id.is_some()) {
            return;
        }
        let Some(bridge) = cx.try_global::<crate::telegram::global::TelegramBridge>() else {
            return;
        };
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

    /// Send the "queued behind the current turn" notice — fires the instant
    /// `AgentChatView::send_prompt_text_for_telegram` reports
    /// [`PromptDispatch::Queued`], since a queued reply hasn't reached the
    /// agent yet and there is nothing to watch a first response for. Plain
    /// tail (fixed i18n copy). Gated by `relay_to_telegram`.
    pub(in crate::workspace) fn relay_queued_notice_to_telegram(
        &self,
        pane_id: PaneId,
        cx: &Context<Self>,
    ) {
        let header = self.telegram_header(pane_id, cx);
        self.relay_to_telegram(
            pane_id,
            header,
            TelegramTail::Plain(s::agent_notification_telegram_reply_queued()),
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
    // TODO(telegram-first-response-ack task 3): called from the event pump's
    // post-`apply_event` hook once that's wired in — remove this `allow`.
    #[allow(dead_code)]
    pub(in crate::workspace) fn relay_first_response_to_telegram(
        &self,
        pane_id: PaneId,
        outcome: FirstResponseOutcome,
        cx: &Context<Self>,
    ) {
        let header = self.telegram_header(pane_id, cx);
        let tail = match outcome {
            FirstResponseOutcome::Text(text) => TelegramTail::Markdown(preview_for(
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
    // TODO(telegram-first-response-ack tasks 3-4): called from
    // `fire_activity_completion`'s settle-without-content branch and the 60s
    // fallback pump once those land — remove this `allow` at that point.
    #[allow(dead_code)]
    pub(in crate::workspace) fn relay_first_response_fallback_to_telegram(
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

    /// Relay a post-turn follow-up (agent text that arrived after the turn ended,
    /// e.g. Claude's background-job completion report) to Telegram. Markdown tail
    /// (agent-authored), truncated like the completion ping. Gated by
    /// `relay_to_telegram`.
    pub(in crate::workspace) fn relay_post_turn_to_telegram(
        &mut self,
        pane_id: PaneId,
        delta: String,
        cx: &Context<Self>,
    ) {
        let header = self.telegram_header(pane_id, cx);
        let body = format!(
            "{}\n{}",
            s::agent_notification_telegram_background_update(),
            preview_for(&delta, &s::agent_notification_telegram_truncated_marker()),
        );
        self.relay_or_defer_to_telegram(
            pane_id,
            DeferKind::PostTurn,
            header,
            TelegramTail::Markdown(body),
            None,
            cx,
        );
    }

    /// The workspace's persisted identity — needed by cross-cutting
    /// App-level services (e.g. the Telegram bridge,
    /// `crate::telegram::global`) that route by `WorkspaceUuid` since
    /// `PaneId` alone is only unique within one workspace, not across
    /// all open windows.
    pub(crate) fn uuid(&self) -> daruda_store::project::WorkspaceUuid {
        self.uuid
    }

    /// Inject a phone-relayed reply as a prompt in this pane via
    /// `AgentChatView::send_prompt_text_for_telegram`. If it has to queue
    /// behind an in-flight turn, sends the immediate "queued" notice (there's
    /// nothing yet to watch a first response for); otherwise it dispatches
    /// straight onto the wire and the view itself arms the first-response
    /// watch — from there, `agent_chat_ops.rs`'s event pump resolves it into
    /// the agent's own first-response ack, or the periodic flush pump's 60s
    /// fallback catches it. A `pub(crate)` entry point for
    /// `crate::telegram::global`'s poll loop to call into (which lives outside
    /// `workspace/` and can't reach the `pub(in crate::workspace)` version).
    pub(crate) fn inject_bot_reply(
        &mut self,
        pane_id: PaneId,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        let dispatch = view.update(cx, |v, cx| v.send_prompt_text_for_telegram(text, cx));
        if dispatch == PromptDispatch::Queued {
            self.relay_queued_notice_to_telegram(pane_id, cx);
        }
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
mod tests {
    use futures::{FutureExt as _, StreamExt as _};
    use gpui::AppContext as _;

    use super::{permission_buttons, permission_wait_tail, preview_for};
    use crate::surface::strings as s;
    use crate::telegram::bridge::PermissionDecision;
    use daruda_acp::{PermissionChoice, PermissionKindView};
    use daruda_store::project::PaneCwd;

    #[test]
    fn should_defer_only_when_enabled_and_active() {
        assert!(super::should_defer_relay(true, true));
        assert!(!super::should_defer_relay(true, false));
        assert!(!super::should_defer_relay(false, true));
    }

    fn tool_call(title: &str) -> daruda_acp::ChatItem {
        use daruda_acp::{ToolCallItem, ToolKindView, ToolStatusView};
        daruda_acp::ChatItem::ToolCall(ToolCallItem {
            id: "tool-1".to_string(),
            title: title.to_string(),
            kind: ToolKindView::Edit,
            tool_name: None,
            status: ToolStatusView::InProgress,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: None,
        })
    }

    fn assistant_text(text: &str, streaming: bool) -> daruda_acp::ChatItem {
        daruda_acp::ChatItem::AssistantText {
            text: text.to_string(),
            streaming,
            message_id: None,
        }
    }

    fn thinking(text: &str) -> daruda_acp::ChatItem {
        daruda_acp::ChatItem::Thinking {
            text: text.to_string(),
            streaming: false,
            message_id: None,
        }
    }

    #[test]
    fn resolve_ignores_thinking_and_still_streaming_text() {
        let watch = super::FirstResponseWatch::start(std::time::Instant::now(), 0);
        let items = vec![thinking("pondering"), assistant_text("partial", true)];
        assert_eq!(watch.resolve(&items), None);
    }

    #[test]
    fn resolve_returns_the_first_completed_assistant_text() {
        let watch = super::FirstResponseWatch::start(std::time::Instant::now(), 0);
        let items = vec![thinking("hmm"), assistant_text("done", false)];
        assert_eq!(
            watch.resolve(&items),
            Some(super::FirstResponseOutcome::Text("done".to_string()))
        );
    }

    #[test]
    fn resolve_returns_a_tool_call_with_no_preceding_text() {
        let watch = super::FirstResponseWatch::start(std::time::Instant::now(), 0);
        let items = vec![thinking("hmm"), tool_call("Write /tmp/x.rs")];
        assert_eq!(
            watch.resolve(&items),
            Some(super::FirstResponseOutcome::Tool {
                tool_title: Some("Write /tmp/x.rs".to_string())
            })
        );
    }

    #[test]
    fn resolve_only_scans_items_appended_after_the_watch_started() {
        // A prior turn's completed AssistantText, present *before* the watch's
        // anchor point, must not be mistaken for this turn's first response.
        let items = vec![assistant_text("previous turn's answer", false)];
        let watch = super::FirstResponseWatch::start(std::time::Instant::now(), items.len());
        assert_eq!(watch.resolve(&items), None);
    }

    #[test]
    fn is_overdue_boundary_at_exactly_the_timeout() {
        let started = std::time::Instant::now();
        let watch = super::FirstResponseWatch::start(started, 0);
        assert!(!watch.is_overdue(started + std::time::Duration::from_secs(59), 60));
        assert!(watch.is_overdue(started + std::time::Duration::from_secs(60), 60));
    }

    /// `queued_at` no longer participates in the push-time decision; a fresh
    /// timestamp is enough for every test entry here.
    fn mk_relay(kind: super::DeferKind) -> super::DeferredRelay {
        super::DeferredRelay {
            kind,
            header: String::new(),
            tail: super::TelegramTail::Plain(String::new()),
            permission: None,
            queued_at: std::time::Instant::now(),
        }
    }

    #[test]
    fn push_deferred_keeps_one_completion_but_accumulates_others() {
        use super::DeferKind;
        let mut q = Vec::new();
        super::push_deferred(&mut q, mk_relay(DeferKind::Completion));
        super::push_deferred(&mut q, mk_relay(DeferKind::PostTurn));
        super::push_deferred(&mut q, mk_relay(DeferKind::Completion));
        assert_eq!(q.len(), 2);
        assert_eq!(q[0].kind, DeferKind::PostTurn);
        assert_eq!(q[1].kind, DeferKind::Completion);
    }

    #[test]
    fn push_deferred_evicts_oldest_beyond_cap() {
        use super::DeferKind;
        let mut q = Vec::new();
        let extra = 5;
        for i in 0..(super::MAX_DEFERRED_PER_PANE + extra) as u64 {
            super::push_deferred(&mut q, mk_relay(DeferKind::Permission { perm_id: i }));
        }
        assert_eq!(q.len(), super::MAX_DEFERRED_PER_PANE);
        // The oldest entries (ids 0..extra) were evicted; the newest survive.
        assert_eq!(
            q[0].kind,
            DeferKind::Permission {
                perm_id: extra as u64
            }
        );
        assert_eq!(
            q.last().unwrap().kind,
            DeferKind::Permission {
                perm_id: (super::MAX_DEFERRED_PER_PANE + extra - 1) as u64
            }
        );
    }

    #[test]
    fn partition_deferred_drops_stale_permission() {
        use super::DeferKind;
        let now = std::time::Instant::now();
        let q = vec![
            mk_relay(DeferKind::Completion),
            mk_relay(DeferKind::Permission { perm_id: 7 }),
        ];
        // Only id 9 is outstanding, so the ping for the resolved id 7 is dropped
        // outright — not delivered, not held for later.
        let live: std::collections::HashSet<u64> = [9].into_iter().collect();
        // `app_active: false` makes every non-stale entry immediately ready,
        // isolating the staleness filter from the readiness check below.
        let (ready, still_holding) = super::partition_deferred(q, &live, now, false, 0.0, 60);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].kind, DeferKind::Completion);
        assert!(still_holding.is_empty());
    }

    #[test]
    fn partition_deferred_keeps_each_outstanding_permission() {
        use super::DeferKind;
        let now = std::time::Instant::now();
        let q = vec![
            mk_relay(DeferKind::Permission { perm_id: 7 }),
            mk_relay(DeferKind::Permission { perm_id: 8 }),
            mk_relay(DeferKind::Permission { perm_id: 9 }),
        ];
        // 7 and 9 are still outstanding; 8 was answered → its ping is dropped,
        // the other two both survive (a single live id could not express this).
        let live: std::collections::HashSet<u64> = [7, 9].into_iter().collect();
        let (ready, still_holding) = super::partition_deferred(q, &live, now, false, 0.0, 60);
        assert_eq!(ready.len(), 2);
        assert_eq!(ready[0].kind, DeferKind::Permission { perm_id: 7 });
        assert_eq!(ready[1].kind, DeferKind::Permission { perm_id: 9 });
        assert!(still_holding.is_empty());
    }

    #[test]
    fn partition_deferred_holds_back_entries_not_yet_ready() {
        // Still foregrounded, just queued (age 0), currently idle — not ready:
        // the quiet window anchors to this entry's own `queued_at`, not to the
        // (already-past-threshold) idle reading alone.
        use super::DeferKind;
        let now = std::time::Instant::now();
        let q = vec![mk_relay(DeferKind::Completion)];
        let live = std::collections::HashSet::new();
        let (ready, still_holding) = super::partition_deferred(q, &live, now, true, 90.0, 60);
        assert!(ready.is_empty());
        assert_eq!(still_holding.len(), 1);
    }

    #[test]
    fn ready_to_deliver_fires_immediately_once_app_is_not_active() {
        let now = std::time::Instant::now();
        // Just queued, and idle_secs is 0 (input this instant) — would fail
        // every other condition, but leaving the app delivers regardless.
        assert!(super::ready_to_deliver(now, now, false, 0.0, 60));
    }

    #[test]
    fn ready_to_deliver_requires_both_elapsed_and_idle_while_foregrounded() {
        let queued_at = std::time::Instant::now();
        let past_window = queued_at + std::time::Duration::from_secs(61);

        // Enough real time has passed AND the user has been idle that whole
        // time → ready.
        assert!(super::ready_to_deliver(
            queued_at,
            past_window,
            true,
            90.0,
            60
        ));
        let already_quiet = past_window - std::time::Duration::from_secs(61);
        assert!(super::ready_to_deliver(
            already_quiet,
            past_window,
            true,
            60.0,
            60
        ));
        // Enough real time has passed, but `idle_secs` shows recent input
        // (the user came back and used the keyboard) → still held.
        assert!(!super::ready_to_deliver(
            queued_at,
            past_window,
            true,
            5.0,
            60
        ));
        // Not enough real time has passed yet, even though `idle_secs` alone
        // would clear the threshold — the ping's own quiet window hasn't
        // elapsed, so a long turn's leftover idle streak can't fire it early.
        assert!(!super::ready_to_deliver(
            queued_at,
            queued_at + std::time::Duration::from_secs(5),
            true,
            90.0,
            60
        ));
    }

    #[test]
    fn permission_wait_tail_appends_the_tool_title_when_present() {
        assert_eq!(
            permission_wait_tail(Some("Write /tmp/x.rs"), None),
            format!("{}\n{}", s::agent_notification_waiting(), "Write /tmp/x.rs")
        );
    }

    #[test]
    fn permission_wait_tail_appends_the_raw_input_summary_after_the_title() {
        assert_eq!(
            permission_wait_tail(Some("Run npm install"), Some("command: npm install")),
            format!(
                "{}\n{}\n{}",
                s::agent_notification_waiting(),
                "Run npm install",
                "command: npm install"
            )
        );
    }

    #[test]
    fn permission_wait_tail_appends_only_the_raw_input_summary_without_a_title() {
        assert_eq!(
            permission_wait_tail(None, Some("file: /tmp/x.rs")),
            format!("{}\n{}", s::agent_notification_waiting(), "file: /tmp/x.rs")
        );
    }

    #[test]
    fn permission_wait_tail_falls_back_to_plain_waiting_text_without_a_title_or_summary() {
        assert_eq!(
            permission_wait_tail(None, None),
            s::agent_notification_waiting()
        );
    }

    #[test]
    fn permission_wait_tail_treats_empty_strings_as_absent() {
        assert_eq!(
            permission_wait_tail(Some(""), Some("")),
            s::agent_notification_waiting()
        );
    }

    #[test]
    fn first_tool_ack_tail_appends_the_title_when_present() {
        assert_eq!(
            super::first_tool_ack_tail(Some("Write /tmp/x.rs")),
            format!(
                "{}\n{}",
                s::agent_notification_telegram_first_tool_ack(),
                "Write /tmp/x.rs"
            )
        );
    }

    #[test]
    fn first_tool_ack_tail_treats_an_empty_title_as_absent() {
        assert_eq!(
            super::first_tool_ack_tail(Some("")),
            s::agent_notification_telegram_first_tool_ack()
        );
    }

    #[test]
    fn first_tool_ack_tail_without_a_title() {
        assert_eq!(
            super::first_tool_ack_tail(None),
            s::agent_notification_telegram_first_tool_ack()
        );
    }

    fn choice(option_id: &str, name: &str, kind: PermissionKindView) -> PermissionChoice {
        PermissionChoice {
            option_id: option_id.to_string(),
            name: name.to_string(),
            kind,
        }
    }

    #[test]
    fn preview_for_under_threshold_is_verbatim() {
        let text = "a".repeat(2000);
        assert_eq!(preview_for(&text, "…(marker)…"), text);
    }

    #[test]
    fn preview_for_over_threshold_keeps_head_and_tail_around_marker() {
        // 1000 'a's + 1000 'b's = 2001 chars, one over the threshold.
        let text = format!("{}{}", "a".repeat(1000), "b".repeat(1001));
        let result = preview_for(&text, "…(marker)…");
        assert_eq!(
            result,
            format!("{}\n…(marker)…\n{}", "a".repeat(1000), "b".repeat(1000))
        );
    }

    #[test]
    fn preview_for_is_char_safe_with_multibyte_text() {
        // Korean text well past the threshold — must not panic on a
        // byte-boundary split, and must actually keep 1000 *characters*
        // (not bytes) on each side.
        let text = "가".repeat(2500);
        let result = preview_for(&text, "…(중략)…");
        let expected = format!("{}\n…(중략)…\n{}", "가".repeat(1000), "가".repeat(1000));
        assert_eq!(result, expected);
    }

    #[test]
    fn permission_buttons_exposes_every_option_not_just_first_of_kind() {
        // The real motivating case (codex-acp): more than one Allow-shaped
        // option (Once, session-scoped, an execpolicy amendment) alongside
        // Reject — all four must become their own button, in order, not
        // collapse to a single Allow/Reject pair.
        let options = vec![
            choice("allow_once", "Allow Once", PermissionKindView::AllowOnce),
            choice(
                "allow_always",
                "Allow for Session",
                PermissionKindView::AllowAlways,
            ),
            choice(
                "accept_execpolicy_amendment",
                "Allow Commands Starting With …",
                PermissionKindView::AllowAlways,
            ),
            choice("reject_once", "Reject", PermissionKindView::RejectOnce),
        ];

        let buttons = permission_buttons(&options);

        assert_eq!(
            buttons,
            vec![
                (
                    "Allow Once".to_string(),
                    PermissionDecision::Allow("allow_once".to_string())
                ),
                (
                    "Allow for Session".to_string(),
                    PermissionDecision::Allow("allow_always".to_string())
                ),
                (
                    "Allow Commands Starting With …".to_string(),
                    PermissionDecision::Allow("accept_execpolicy_amendment".to_string())
                ),
                (
                    "Reject".to_string(),
                    PermissionDecision::Reject("reject_once".to_string())
                ),
            ]
        );
    }

    #[test]
    fn permission_buttons_maps_reject_always_to_reject_decision() {
        let options = vec![choice(
            "reject_always",
            "Always Reject",
            PermissionKindView::RejectAlways,
        )];
        assert_eq!(
            permission_buttons(&options),
            vec![(
                "Always Reject".to_string(),
                PermissionDecision::Reject("reject_always".to_string())
            )]
        );
    }

    #[test]
    fn permission_buttons_empty_options_is_empty() {
        assert!(permission_buttons(&[]).is_empty());
    }

    /// A phone reply injected into a pane that has never connected (no
    /// `handle`, so `send_prompt_text_for_telegram` always queues) sends the
    /// "queued" notice — there's nothing yet to watch a first response for.
    #[gpui::test]
    async fn inject_bot_reply_sends_the_queued_notice_when_it_has_to_wait(
        cx: &mut gpui::TestAppContext,
    ) {
        let mut outbound =
            cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
        let mut config = daruda_config::Config::default();
        config.telegram.enabled = true;
        config.telegram.authorized_chat_id = Some(42);
        let (handle, workspace) = make_window(cx, &config);
        cx.run_until_parked();

        let tmp = std::env::temp_dir();
        let pane_id = cx
            .update_window(handle.into(), |_, window, cx| {
                workspace.update(cx, |ws, cx| {
                    let pane = ws.create_agent_chat_pane(
                        Some(PaneCwd::Local(tmp.clone())),
                        None,
                        daruda_config::AgentDefinition::claude_default().id,
                        None,
                        window,
                        cx,
                    );
                    let id = pane.id;
                    ws.active_runtime_mut().panes.push(pane);
                    id
                })
            })
            .unwrap();
        cx.run_until_parked();

        workspace.update(cx, |ws, cx| {
            ws.inject_bot_reply(pane_id, "hello from telegram".to_string(), cx);
        });
        cx.run_until_parked();

        workspace.read_with(cx, |ws, cx| {
            let view = ws.agent_chat_view(pane_id).cloned().expect("pane present");
            assert_eq!(
                view.read(cx)
                    .pending_prompts
                    .iter()
                    .map(|q| q.text.as_str())
                    .collect::<Vec<_>>(),
                vec!["hello from telegram"],
                "the reply queues (never connected)"
            );
            assert!(
                view.read(cx).telegram_first_response_watch.is_none(),
                "queuing alone must not arm the watch"
            );
        });

        let sent = outbound
            .next()
            .await
            .expect("the queued notice should be sent");
        assert_eq!(
            sent.tail,
            super::TelegramTail::Plain(s::agent_notification_telegram_reply_queued())
        );
    }

    /// Cross-workspace routing coverage: `crate::telegram::global`'s
    /// `dispatch_action` matches a `PaneRef.workspace` against every open
    /// `Workspace` via `WindowRegistry::for_each_workspace` + `ws.uuid() ==
    /// pane.workspace`, then mutates only the matching one. That guard —
    /// the entire reason `Workspace::uuid` and `for_each_workspace` exist
    /// for this feature — had zero test coverage; the pure pieces
    /// (`permission_buttons`, `BridgeCore`) are covered above and in
    /// `telegram::bridge`, but not the multi-window dispatch itself.
    ///
    /// This drives the same shape `InboundAction::InjectPrompt`'s dispatch
    /// arm uses: two real `Workspace` windows (mirroring
    /// `window_registry.rs`'s `make_window` test helper), one AgentChat pane
    /// per workspace, `for_each_workspace` + a `uuid()` guard, and
    /// `inject_bot_reply` as the observable mutation. Only the pane in the
    /// targeted workspace should receive the injected queued prompt.
    #[gpui::test]
    async fn for_each_workspace_uuid_guard_dispatches_to_only_the_matching_pane(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::window_registry::WindowRegistry;

        let config = daruda_config::Config::default();
        let (handle_a, workspace_a) = make_window(cx, &config);
        let (handle_b, workspace_b) = make_window(cx, &config);
        cx.run_until_parked();

        let tmp = std::env::temp_dir();
        let pane_a = cx
            .update_window(handle_a.into(), |_, window, cx| {
                workspace_a.update(cx, |ws, cx| {
                    let pane = ws.create_agent_chat_pane(
                        Some(PaneCwd::Local(tmp.clone())),
                        None,
                        daruda_config::AgentDefinition::claude_default().id,
                        None,
                        window,
                        cx,
                    );
                    let id = pane.id;
                    ws.active_runtime_mut().panes.push(pane);
                    id
                })
            })
            .unwrap();
        let pane_b = cx
            .update_window(handle_b.into(), |_, window, cx| {
                workspace_b.update(cx, |ws, cx| {
                    let pane = ws.create_agent_chat_pane(
                        Some(PaneCwd::Local(tmp.clone())),
                        None,
                        daruda_config::AgentDefinition::claude_default().id,
                        None,
                        window,
                        cx,
                    );
                    let id = pane.id;
                    ws.active_runtime_mut().panes.push(pane);
                    id
                })
            })
            .unwrap();
        cx.run_until_parked();

        let target_uuid = workspace_a.read_with(cx, |ws, _| ws.uuid());
        assert_ne!(
            target_uuid,
            workspace_b.read_with(cx, |ws, _| ws.uuid()),
            "each window has a distinct workspace uuid"
        );

        // Exactly what `dispatch_action`'s `InjectPrompt` arm does.
        cx.update(|cx| {
            WindowRegistry::for_each_workspace(cx, |ws, _window, cx| {
                if ws.uuid() == target_uuid {
                    ws.inject_bot_reply(pane_a, "hello from telegram".to_string(), cx);
                }
            });
        });
        cx.run_until_parked();

        workspace_a.read_with(cx, |ws, cx| {
            let view = ws.agent_chat_view(pane_a).cloned().expect("pane a present");
            assert_eq!(
                view.read(cx)
                    .pending_prompts
                    .iter()
                    .map(|q| q.text.as_str())
                    .collect::<Vec<_>>(),
                vec!["hello from telegram"],
                "the targeted workspace's pane received the injected queued prompt"
            );
        });
        workspace_b.read_with(cx, |ws, cx| {
            let view = ws.agent_chat_view(pane_b).cloned().expect("pane b present");
            assert!(
                view.read(cx).pending_prompts.is_empty(),
                "the non-matching workspace's pane must not be touched"
            );
        });
    }

    /// `deliver_deferred_telegram` must tolerate a queued entry whose pane has
    /// since closed, filter stale permission pings through the live pane's
    /// `pending_permissions` outstanding-ids set, and still deliver the
    /// remaining live-pane entry.
    #[gpui::test]
    async fn deliver_deferred_telegram_skips_closed_pane_filters_stale_permission_and_sends_live_entry(
        cx: &mut gpui::TestAppContext,
    ) {
        let mut outbound =
            cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
        let mut config = daruda_config::Config::default();
        config.telegram.enabled = true;
        config.telegram.authorized_chat_id = Some(42);
        let (handle, workspace) = make_window(cx, &config);
        cx.run_until_parked();

        let tmp = std::env::temp_dir();
        let live_pane = cx
            .update_window(handle.into(), |_, window, cx| {
                workspace.update(cx, |ws, cx| {
                    let pane = ws.create_agent_chat_pane(
                        Some(PaneCwd::Local(tmp.clone())),
                        None,
                        daruda_config::AgentDefinition::claude_default().id,
                        None,
                        window,
                        cx,
                    );
                    let id = pane.id;
                    ws.active_runtime_mut().panes.push(pane);
                    id
                })
            })
            .unwrap();
        cx.run_until_parked();

        // A pane id well past anything the test ever allocated — stands in
        // for "the pane closed after the ping was queued".
        const CLOSED_PANE: super::PaneId = u64::MAX;

        let mk = |kind, tail: &str| super::DeferredRelay {
            kind,
            header: "header".to_string(),
            tail: super::TelegramTail::Plain(tail.to_string()),
            permission: None,
            queued_at: std::time::Instant::now(),
        };

        workspace.update(cx, |ws, _| {
            ws.deferred_telegram
                .entry(CLOSED_PANE)
                .or_default()
                .push(mk(super::DeferKind::Completion, "closed"));
            ws.deferred_telegram
                .entry(live_pane)
                .or_default()
                .push(mk(super::DeferKind::Completion, "completion"));
            // Live pane, but a permission id that is not in the pane's
            // `pending_permissions` set (empty — no permission was ever
            // requested on this pane) — exercises the `partition_deferred`
            // staleness filter path on a real view.
            ws.deferred_telegram
                .entry(live_pane)
                .or_default()
                .push(mk(super::DeferKind::Permission { perm_id: 42 }, "stale"));
        });

        // `app_active: false` mirrors "presence already dropped" — every
        // non-stale entry is immediately ready regardless of age/idle.
        workspace.update(cx, |ws, cx| {
            ws.deliver_deferred_telegram(false, 999.0, 60, cx);
        });
        cx.run_until_parked();

        workspace.read_with(cx, |ws, _| {
            assert!(
                ws.deferred_telegram.is_empty(),
                "the queue drains after delivery"
            );
        });

        let sent = outbound
            .next()
            .await
            .expect("the live completion entry should be sent");
        assert_eq!(sent.pane.pane, live_pane);
        assert_eq!(sent.header, "header");
        assert_eq!(
            sent.tail,
            super::TelegramTail::Plain("completion".to_string())
        );
        assert!(sent.permission.is_none());
        assert!(
            outbound.next().now_or_never().is_none(),
            "closed panes and stale permissions must not emit extra pings"
        );
    }

    /// A ping that hasn't cleared its own quiet window yet stays queued, then
    /// drains once the same foregrounded/idle pane has cleared that window.
    #[gpui::test]
    async fn deliver_deferred_telegram_holds_until_entry_quiet_window_clears(
        cx: &mut gpui::TestAppContext,
    ) {
        let mut outbound =
            cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
        let mut config = daruda_config::Config::default();
        config.telegram.enabled = true;
        config.telegram.authorized_chat_id = Some(42);
        let (handle, workspace) = make_window(cx, &config);
        cx.run_until_parked();

        let tmp = std::env::temp_dir();
        let pane = cx
            .update_window(handle.into(), |_, window, cx| {
                workspace.update(cx, |ws, cx| {
                    let pane = ws.create_agent_chat_pane(
                        Some(PaneCwd::Local(tmp.clone())),
                        None,
                        daruda_config::AgentDefinition::claude_default().id,
                        None,
                        window,
                        cx,
                    );
                    let id = pane.id;
                    ws.active_runtime_mut().panes.push(pane);
                    id
                })
            })
            .unwrap();
        cx.run_until_parked();

        workspace.update(cx, |ws, _| {
            ws.deferred_telegram
                .entry(pane)
                .or_default()
                .push(super::DeferredRelay {
                    kind: super::DeferKind::Completion,
                    header: "header".to_string(),
                    tail: super::TelegramTail::Plain("completion".to_string()),
                    permission: None,
                    queued_at: std::time::Instant::now(),
                });
        });

        // Foregrounded, and `idle_secs` alone would already clear the
        // threshold — but the entry was just queued, so its own window
        // hasn't elapsed yet.
        workspace.update(cx, |ws, cx| {
            ws.deliver_deferred_telegram(true, 90.0, 60, cx);
        });
        cx.run_until_parked();

        workspace.read_with(cx, |ws, _| {
            assert_eq!(
                ws.deferred_telegram.get(&pane).map(Vec::len),
                Some(1),
                "the entry stays queued instead of firing early"
            );
        });
        assert!(
            outbound.next().now_or_never().is_none(),
            "nothing should have been sent yet"
        );

        workspace.update(cx, |ws, _| {
            ws.deferred_telegram
                .get_mut(&pane)
                .and_then(|q| q.get_mut(0))
                .expect("held entry should still be queued")
                .queued_at = std::time::Instant::now() - std::time::Duration::from_secs(61);
        });

        workspace.update(cx, |ws, cx| {
            ws.deliver_deferred_telegram(true, 60.0, 60, cx);
        });
        cx.run_until_parked();

        workspace.read_with(cx, |ws, _| {
            assert!(
                ws.deferred_telegram.is_empty(),
                "the entry drains once its foreground quiet window clears"
            );
        });
        let sent = outbound
            .next()
            .await
            .expect("the quiet-window-cleared entry should be sent");
        assert_eq!(sent.pane.pane, pane);
        assert_eq!(sent.header, "header");
        assert_eq!(
            sent.tail,
            super::TelegramTail::Plain("completion".to_string())
        );
    }

    /// Turning off active-window deferral should release already-held pings on
    /// the next flush instead of leaving them governed by the old foreground
    /// quiet-window rules.
    #[gpui::test]
    async fn flush_deferred_telegram_drains_existing_queue_when_defer_disabled(
        cx: &mut gpui::TestAppContext,
    ) {
        let mut outbound =
            cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
        let mut config = daruda_config::Config::default();
        config.telegram.enabled = true;
        config.telegram.authorized_chat_id = Some(42);
        config.telegram.defer_while_active = false;
        let (handle, workspace) = make_window(cx, &config);
        cx.run_until_parked();

        let tmp = std::env::temp_dir();
        let pane = cx
            .update_window(handle.into(), |_, window, cx| {
                workspace.update(cx, |ws, cx| {
                    let pane = ws.create_agent_chat_pane(
                        Some(PaneCwd::Local(tmp.clone())),
                        None,
                        daruda_config::AgentDefinition::claude_default().id,
                        None,
                        window,
                        cx,
                    );
                    let id = pane.id;
                    ws.active_runtime_mut().panes.push(pane);
                    id
                })
            })
            .unwrap();
        cx.run_until_parked();

        workspace.update(cx, |ws, _| {
            ws.deferred_telegram
                .entry(pane)
                .or_default()
                .push(super::DeferredRelay {
                    kind: super::DeferKind::Completion,
                    header: "header".to_string(),
                    tail: super::TelegramTail::Plain("completion".to_string()),
                    permission: None,
                    queued_at: std::time::Instant::now(),
                });
        });

        workspace.update(cx, |ws, cx| {
            ws.flush_deferred_telegram(cx);
        });
        cx.run_until_parked();

        workspace.read_with(cx, |ws, _| {
            assert!(
                ws.deferred_telegram.is_empty(),
                "defer-disabled flush should drain the held queue"
            );
        });

        let sent = outbound
            .next()
            .await
            .expect("the held completion entry should be sent");
        assert_eq!(sent.pane.pane, pane);
        assert_eq!(sent.header, "header");
        assert_eq!(
            sent.tail,
            super::TelegramTail::Plain("completion".to_string())
        );
        assert!(outbound.next().now_or_never().is_none());
    }

    /// A phone tap routes by permission id, not by position: with two
    /// permissions outstanding, tapping the button for id A resolves *A* and
    /// leaves B live; a second tap for an already-answered id is a clean no-op.
    /// Mirrors the in-app `concurrent_permissions_resolve_independently_by_id`
    /// for the Telegram `respond_bot_permission` path.
    #[gpui::test]
    async fn respond_bot_permission_routes_by_id_under_concurrency(cx: &mut gpui::TestAppContext) {
        use daruda_acp::{
            ChatItem, PermissionChoice, PermissionItem, PermissionKindView, PermissionResolution,
        };

        let config = daruda_config::Config::default();
        let (handle, workspace) = make_window(cx, &config);
        cx.run_until_parked();

        let tmp = std::env::temp_dir();
        let card = |id: u64| {
            ChatItem::Permission(PermissionItem {
                id,
                tool_title: Some(format!("Write /tmp/{id}")),
                raw_input_summary: None,
                options: vec![
                    PermissionChoice {
                        option_id: "allow_once".to_string(),
                        name: "Allow".to_string(),
                        kind: PermissionKindView::AllowOnce,
                    },
                    PermissionChoice {
                        option_id: "reject_once".to_string(),
                        name: "Reject".to_string(),
                        kind: PermissionKindView::RejectOnce,
                    },
                ],
                resolved: None,
            })
        };

        let pane = cx
            .update_window(handle.into(), |_, window, cx| {
                workspace.update(cx, |ws, cx| {
                    let pane = ws.create_agent_chat_pane(
                        Some(PaneCwd::Local(tmp.clone())),
                        None,
                        daruda_config::AgentDefinition::claude_default().id,
                        None,
                        window,
                        cx,
                    );
                    let id = pane.id;
                    ws.active_runtime_mut().panes.push(pane);
                    let view = ws.agent_chat_view(id).cloned().expect("view present");
                    view.update(cx, |v, _| {
                        v.items = vec![card(100), card(200)];
                        v.pending_permissions.insert(100);
                        v.pending_permissions.insert(200);
                    });
                    id
                })
            })
            .unwrap();
        cx.run_until_parked();

        // Phone-tap the button for id 100, then a stale re-tap for 100.
        workspace.update(cx, |ws, cx| {
            ws.respond_bot_permission(
                pane,
                100,
                PermissionDecision::Allow("allow_once".into()),
                cx,
            );
            // Already answered → is_permission_outstanding(100) is false → no-op.
            ws.respond_bot_permission(
                pane,
                100,
                PermissionDecision::Reject("reject_once".into()),
                cx,
            );
        });
        cx.run_until_parked();

        workspace.read_with(cx, |ws, cx| {
            let view = ws.agent_chat_view(pane).cloned().unwrap();
            let view = view.read(cx);
            let ChatItem::Permission(first) = &view.items[0] else {
                panic!("expected first permission card");
            };
            let ChatItem::Permission(second) = &view.items[1] else {
                panic!("expected second permission card");
            };
            assert_eq!(
                first.resolved,
                Some(PermissionResolution::Chosen("allow_once".to_string())),
                "the tapped id resolves to its own option; the stale re-tap is a no-op"
            );
            assert_eq!(second.resolved, None, "the other permission stays live");
            assert!(view.is_permission_outstanding(200));
            assert!(!view.is_permission_outstanding(100));
        });

        // Phone-tap id 200 → the pane drains fully.
        workspace.update(cx, |ws, cx| {
            ws.respond_bot_permission(
                pane,
                200,
                PermissionDecision::Reject("reject_once".into()),
                cx,
            );
        });
        cx.run_until_parked();

        workspace.read_with(cx, |ws, cx| {
            let view = ws.agent_chat_view(pane).cloned().unwrap();
            let view = view.read(cx);
            let ChatItem::Permission(second) = &view.items[1] else {
                panic!("expected second permission card");
            };
            assert_eq!(
                second.resolved,
                Some(PermissionResolution::Chosen("reject_once".to_string())),
            );
            assert!(!view.has_pending_permission());
        });
    }

    /// Construct a Workspace wrapped in `gpui_component::Root` — matches the
    /// production windowing path so APIs that walk the window root don't
    /// panic during construction. Local adaptation of
    /// `window_registry.rs`'s test-only `make_window` helper (that helper is
    /// private to its own module, so it isn't reachable from here).
    fn make_window(
        cx: &mut gpui::TestAppContext,
        config: &daruda_config::Config,
    ) -> (
        gpui::WindowHandle<gpui_component::Root>,
        gpui::Entity<super::Workspace>,
    ) {
        crate::test_support::init_gpui_component(cx);
        let workspace_for_root = std::cell::RefCell::new(None);
        let wh = cx.add_window(|window, cx| {
            let workspace = cx.new(|cx| super::Workspace::new(config, test_data_dir(), window, cx));
            *workspace_for_root.borrow_mut() = Some(workspace.clone());
            gpui_component::Root::new(workspace, window, cx)
        });
        let workspace = workspace_for_root.borrow().clone().unwrap();
        (wh, workspace)
    }

    fn test_data_dir() -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!("daruda_telegram_ops_test_{id}"))
    }
}
