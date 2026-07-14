//! Telegram relay — `Workspace` ops that push agent-chat pings out to the
//! Telegram bridge (`crate::telegram::global::TelegramBridge`) and route
//! phone-tapped replies / permission decisions back into the pane that
//! triggered them.
//!
//! Sibling of [`super::agent_chat_ops`], which owns the desktop-notification
//! pipeline and still fires the two tee points — `maybe_notify_agent_event`
//! (permission wait) and `fire_activity_completion` (turn completion) — that
//! call into this file's `relay_*` methods exactly as before this domain was
//! split out; both files are `impl Workspace` blocks in the same module tree,
//! so no new plumbing was needed to keep that wired up.

use gpui::Context;

use crate::surface::strings as s;
use crate::telegram::bridge::TelegramTail;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

/// Below this char count, [`preview_for`] sends the text verbatim.
const TELEGRAM_PREVIEW_THRESHOLD: usize = 2000;
/// How many leading/trailing characters survive when a response is
/// truncated — the head carries what was asked/done, the tail carries the
/// final result; the middle is usually tool-output detail already visible
/// in the app.
const TELEGRAM_PREVIEW_HEAD_CHARS: usize = 1000;
const TELEGRAM_PREVIEW_TAIL_CHARS: usize = 1000;

/// Keep `text` verbatim under [`TELEGRAM_PREVIEW_THRESHOLD`] characters;
/// past it, keep the first [`TELEGRAM_PREVIEW_HEAD_CHARS`] and last
/// [`TELEGRAM_PREVIEW_TAIL_CHARS`] characters around `marker` (the
/// caller's localized "…(truncated)…" string). Counts `char`s, not bytes,
/// so a multi-byte response (Korean, emoji, …) never splits mid-character.
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

/// Compose the permission-wait ping's tail: the localized "waiting for
/// input" line, then an optional tool-title line, then an optional
/// raw-input-summary line — both are the same
/// `daruda_acp::PermissionItem::{tool_title,raw_input_summary}` fields the
/// in-app permission card is built from, so the phone message says *what*
/// is being asked (e.g. "Write /tmp/x.rs" / "command: npm install") instead
/// of just that something is. An empty string is treated the same as
/// absent (defensive — `raw_input_summary` already filters this at the
/// source, see `daruda_acp::mapping::summarize_raw_input`).
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

/// Build one Telegram button per permission choice, in the same order and
/// with the same labels the in-app card renders them (the same
/// `daruda_acp::PermissionChoice` view-model list
/// `AgentChatView::apply_event` renders as in-app buttons — see
/// `daruda_acp::mapping::permission_item`). No collapsing to a single
/// Allow/Reject pair: a richer option set (e.g. codex-acp's "Allow Once" /
/// "Allow for Session" / an execpolicy-amendment allow, alongside Reject)
/// stays fully choosable from the phone, matching the desktop card exactly.
/// Each choice's `*Once`/`*Always` kind maps to the same
/// `daruda_acp::PermissionDecision::Allow`/`Reject` outcome
/// `AgentChatView::respond_permission` sends for it — the finer-grained kind
/// only picks in-app button styling (accent vs. danger), never the wire
/// outcome, which is carried entirely by `option_id`. Returns an empty
/// `Vec` only if the agent supplied no options at all (the caller skips the
/// relay in that case — nothing to build a keyboard from).
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

impl Workspace {
    /// The owning project's display name for `pane_id`, if it belongs to
    /// one (`main_area.runtimes` is keyed by `LaneRef { project, lane }`,
    /// so the project id comes from whichever runtime's pane list contains
    /// this pane). `None` for a pane whose lane/project has since gone away.
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

    /// Compose the turn-completion ping's header + tail: project name and
    /// agent name as the (always plain) header, and the agent's actual last
    /// response (see [`preview_for`] for the head/tail truncation past 2000
    /// chars) as a [`TelegramTail::Markdown`] tail — so the phone shows
    /// what the agent actually said, with its own markdown rendered,
    /// rather than a bare "completed" line or literal `**`/backticks.
    /// Falls back to a [`TelegramTail::Plain`] "finished responding" tail
    /// when the turn produced no assistant text at all (e.g. a tool-only
    /// turn) or the pane's view is already gone — that fallback text is
    /// plain i18n copy, not agent-authored markdown, so it must not be
    /// markdown-parsed either (see [`TelegramTail`]'s doc comment).
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
        &self,
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
        self.relay_to_telegram(
            pane_id,
            self.pane_title(pane_id, cx),
            TelegramTail::Plain(tail),
            Some(crate::telegram::bridge::PermissionPromptRef { perm_id, buttons }),
            cx,
        );
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

    /// Send a lightweight "received, working" ack to Telegram the instant a
    /// phone-relayed reply is injected. Plain tail (fixed i18n copy, not
    /// agent-authored markdown). Gated by `relay_to_telegram`.
    pub(in crate::workspace) fn relay_ack_to_telegram(&self, pane_id: PaneId, cx: &Context<Self>) {
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
        &self,
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
        self.relay_to_telegram(pane_id, header, TelegramTail::Markdown(body), None, cx);
    }

    /// The workspace's persisted identity — needed by cross-cutting
    /// App-level services (e.g. the Telegram bridge,
    /// `crate::telegram::global`) that route by `WorkspaceUuid` since
    /// `PaneId` alone is only unique within one workspace, not across
    /// all open windows.
    pub(crate) fn uuid(&self) -> daruda_store::project::WorkspaceUuid {
        self.uuid
    }

    /// Inject a phone-relayed reply as a prompt in this pane, and first send an
    /// immediate Telegram ack (`relay_ack_to_telegram`) confirming receipt. The
    /// prompt itself still goes through the existing `send_agent_prompt_text`
    /// funnel — no new *prompt* delivery path, only the added ack ping. A
    /// `pub(crate)` entry point for `crate::telegram::global`'s poll loop to
    /// call into (which lives outside `workspace/` and can't reach the
    /// `pub(in crate::workspace)` version).
    pub(crate) fn inject_bot_reply(
        &mut self,
        pane_id: PaneId,
        text: String,
        cx: &mut Context<Self>,
    ) {
        // Immediately confirm receipt on the phone — closes the blind gap
        // between the reply landing and the turn producing any output.
        self.relay_ack_to_telegram(pane_id, cx);
        self.send_agent_prompt_text(pane_id, text, cx);
    }

    /// Resolve a phone-tapped Allow/Reject button against this pane's
    /// currently-pending permission. Delegates to the existing
    /// `AgentChatView::respond_permission`, which already: resolves
    /// the trailing permission card, sends the decision over the ACP
    /// session, and reflows the row list — the same path the in-app
    /// buttons use.
    ///
    /// The bridge's `perm_id` (captured when the ping was built) is
    /// intentionally not checked against the view's current
    /// `pending_permission` here — `AgentChatView::respond_permission`
    /// already only acts on whatever is currently pending and no-ops
    /// if nothing is (`pending_permission.take()` returns `None`). A
    /// phone button for an already-superseded permission request
    /// landing on a newer pending request is a pre-existing accepted
    /// limitation (see `AgentChatView::pending_permission`'s own doc:
    /// "MVP serialises permissions... a new request replaces the
    /// previous pending id") — this method doesn't introduce a new
    /// failure mode, just inherits the existing
    /// single-serialized-permission-per-pane behavior.
    pub(crate) fn respond_bot_permission(
        &mut self,
        pane_id: PaneId,
        decision: crate::telegram::bridge::PermissionDecision,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        let (option_id, kind) = match decision {
            crate::telegram::bridge::PermissionDecision::Allow(id) => {
                (id, daruda_acp::PermissionKindView::AllowOnce)
            }
            crate::telegram::bridge::PermissionDecision::Reject(id) => {
                (id, daruda_acp::PermissionKindView::RejectOnce)
            }
        };
        view.update(cx, |v, cx| v.respond_permission(option_id, kind, cx));
    }
}

#[cfg(test)]
mod tests {
    use gpui::AppContext as _;

    use super::{permission_buttons, permission_wait_tail, preview_for};
    use crate::surface::strings as s;
    use crate::telegram::bridge::PermissionDecision;
    use daruda_acp::{PermissionChoice, PermissionKindView};
    use daruda_store::project::PaneCwd;

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
    /// targeted workspace should receive the injected prompt.
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
                view.read(cx).items.len(),
                1,
                "the targeted workspace's pane received the injected prompt"
            );
        });
        workspace_b.read_with(cx, |ws, cx| {
            let view = ws.agent_chat_view(pane_b).cloned().expect("pane b present");
            assert!(
                view.read(cx).items.is_empty(),
                "the non-matching workspace's pane must not be touched"
            );
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
