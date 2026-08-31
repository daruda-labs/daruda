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
/// Maximum time a phone-triggered turn can stay silent before Telegram receives
/// the fixed fallback acknowledgement.
pub(in crate::workspace) const FIRST_RESPONSE_FALLBACK_SECS: u64 = 60;

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

/// Hold a presence-gated relay instead of sending now: the feature is on, the
/// app is foreground, and the quiet window is positive. `quiet_secs == 0`
/// means "do not hold" so changing the setting to zero releases existing
/// queues on the next flush instead of creating an always-ready but still-held
/// foreground queue. Whether a *held* ping is later actually delivered is
/// decided per-entry by [`ready_to_deliver`] — this only gates whether it goes
/// into the queue at all. Pure so it is unit-testable without a live
/// `NSApplication`.
pub(in crate::workspace) fn should_defer_relay(
    enabled: bool,
    app_active: bool,
    quiet_secs: u64,
) -> bool {
    enabled && app_active && quiet_secs > 0
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
    /// (see `daruda_acp::ChatItem::AssistantText`'s `streaming` field). A
    /// settled message with no text is skipped too — `daruda_acp` collapses a
    /// content block it cannot render to an empty string, and an empty
    /// notification is worse than a late one.
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
                } if !text.trim().is_empty() => Some(FirstResponseOutcome::Text(text.clone())),
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
        // Skips a message with no text for the same reason `resolve` does: it
        // would put an empty preview under the notification header.
        let last_response = view.items.iter().rev().find_map(|item| match item {
            daruda_acp::ChatItem::AssistantText { text, .. } if !text.trim().is_empty() => {
                Some(text.as_str())
            }
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
    /// input". When this permission is the first visible response to a
    /// phone-originated prompt, it bypasses active-window deferral so the phone
    /// immediately gets the action buttons; later permission waits keep the
    /// normal presence gate. Always a [`TelegramTail::Plain`] tail: none of
    /// `tool_title`, `raw_input_summary`, or the "waiting" label is
    /// agent-authored markdown — see [`TelegramTail`]'s doc comment for why
    /// that matters.
    pub(in crate::workspace) fn relay_permission_wait_to_telegram(
        &mut self,
        pane_id: PaneId,
        perm_id: u64,
        options: &[daruda_acp::PermissionChoice],
        tool_title: Option<&str>,
        raw_input_summary: Option<&str>,
        cx: &Context<Self>,
    ) {
        let is_telegram_first_response = self
            .agent_chat_view(pane_id)
            .is_some_and(|view| view.read(cx).is_waiting_for_telegram_first_response());
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
        if is_telegram_first_response {
            self.relay_to_telegram(pane_id, header, TelegramTail::Plain(tail), permission, cx);
        } else {
            self.relay_or_defer_to_telegram(
                pane_id,
                DeferKind::Permission { perm_id },
                header,
                TelegramTail::Plain(tail),
                permission,
                cx,
            );
        }
    }

    /// Presence-gated entry point for the completion / permission / post-turn
    /// relays. While the user is present (app foreground) the composed ping is
    /// held per-pane; the periodic flush (`Workspace::flush_deferred_telegram`)
    /// later decides, per entry, when it's actually ready to go out (see
    /// `ready_to_deliver`). Otherwise it is sent immediately. First-response
    /// acks and first-response permission waits deliberately bypass this and
    /// call `relay_to_telegram` directly.
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
            self.telegram.active_idle_secs,
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

    /// Periodic safety net for phone-triggered turns that produce no completed
    /// assistant text or tool call within [`FIRST_RESPONSE_FALLBACK_SECS`].
    /// The view clears each watch as it is taken, so a pane can emit this
    /// fallback at most once per phone-dispatched turn.
    pub(crate) fn flush_telegram_first_response_fallbacks(&mut self, cx: &mut Context<Self>) {
        let now = std::time::Instant::now();
        let panes: Vec<PaneId> = self
            .main_area
            .runtimes
            .values()
            .flat_map(|runtime| runtime.panes.iter())
            .filter(|pane| pane.agent_chat_view().is_some())
            .map(|pane| pane.id)
            .collect();

        for pane_id in panes {
            let Some(view) = self.agent_chat_view(pane_id).cloned() else {
                continue;
            };
            let overdue = view.update(cx, |v, _| {
                v.take_telegram_first_response_fallback_if_overdue(
                    now,
                    FIRST_RESPONSE_FALLBACK_SECS,
                )
            });
            if overdue {
                self.relay_first_response_fallback_to_telegram(pane_id, cx);
            }
        }
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

    /// Inject a phone-relayed reply as a prompt in this pane through the same
    /// Workspace submit funnel as the bottom-dock composer. If it has to queue
    /// behind an in-flight turn, sends the immediate "queued" notice (there's
    /// nothing yet to watch a first response for); otherwise it dispatches
    /// straight onto the wire and the view itself arms the first-response
    /// watch. Local-only slash commands never reach the agent, so they receive
    /// the fixed fallback ack immediately. A `pub(crate)` entry point for
    /// `crate::telegram::global`'s poll loop to call into (which lives outside
    /// `workspace/` and can't reach the `pub(in crate::workspace)` version).
    pub(crate) fn inject_bot_reply(
        &mut self,
        pane_id: PaneId,
        text: String,
        cx: &mut Context<Self>,
    ) {
        if self.agent_chat_view(pane_id).is_none() {
            return;
        }
        match self.send_agent_prompt_text_from_telegram(pane_id, text, cx) {
            Some(PromptDispatch::Queued) => self.relay_queued_notice_to_telegram(pane_id, cx),
            Some(PromptDispatch::SentNow) => {}
            None => self.relay_first_response_fallback_to_telegram(pane_id, cx),
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
mod tests;
