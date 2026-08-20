//! Pane chrome around the conversation: the top status banner, the activity
//! bar (title + icon controls), and the inline "agent is working" indicator
//! with its animated pulse dots and elapsed clock.

use daruda_acp::{ChatItem, ConnectPhase, UsageView};
use gpui::{
    AnyElement, AnyWindowHandle, Context, Hsla, IntoElement, SharedString, div, prelude::*, px,
};

use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{
    ButtonVariants as _, Icon, Selectable as _, Sizable as _, StatusPulseClock, button_bare,
};
use crate::workspace::main_area::agent_chat_pane::view::{
    AgentChatView, AgentSessionStatus, ChatContentWidth, RuntimePrepPhase,
};
use crate::workspace::main_area::pane_tree::PaneId;

const ICON_EXPAND: &str = "icons/ui/expand.svg";
const ICON_COMPRESS: &str = "icons/ui/compress.svg";
const ICON_WIDTH_WIDE: &str = "icons/ui/width-wide.svg";

/// Pane activity bar: resolved session title on the left, icon controls on the
/// right. Always rendered: the reading-width toggle is available even while the
/// conversation is empty or still connecting. The `title` is already resolved
/// by the caller (`activity_bar_title`, falling back to the agent name). The
/// fold buttons appear only when `has_items` is true (render purity: no logic
/// here, just `.when()`).
/// A bottom hairline separates the bar from the conversation body.
pub(super) struct ActivityBarProps<'a> {
    pub pane_id: PaneId,
    pub agent_id: &'a str,
    pub title: Option<&'a str>,
    pub last_active: Option<&'a str>,
    pub usage: Option<&'a UsageView>,
    pub has_items: bool,
    pub content_width: ChatContentWidth,
    pub dim: f32,
}

pub(super) fn activity_bar(
    props: ActivityBarProps<'_>,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let title: SharedString = props
        .title
        .map(|s| SharedString::from(s.to_string()))
        .unwrap_or_default();
    // Last-activity timestamp (from `SessionInfoUpdate.updated_at`) surfaces as a
    // tooltip on the title rather than inline text — it's low-frequency detail
    // and the bar is width-constrained (title ellipsizes).
    let last_active_tooltip: Option<SharedString> = props
        .last_active
        .map(format_last_active)
        .map(|when| SharedString::from(s::agent_chat_last_active_tooltip(&when)));

    let expand = button_bare(("agent-chat-expand-all", props.pane_id as usize))
        .ghost()
        .xsmall()
        .icon(Icon::empty().path(ICON_EXPAND))
        .tooltip(SharedString::from(s::agent_chat_expand_all()))
        .on_click(cx.listener(move |this, _ev, window, cx| this.set_all_folds(true, window, cx)));
    let collapse = button_bare(("agent-chat-collapse-all", props.pane_id as usize))
        .ghost()
        .xsmall()
        .icon(Icon::empty().path(ICON_COMPRESS))
        .tooltip(SharedString::from(s::agent_chat_collapse_all()))
        .on_click(cx.listener(move |this, _ev, window, cx| this.set_all_folds(false, window, cx)));
    let reading_selected = props.content_width.is_reading();
    let reading_tooltip = if reading_selected {
        s::agent_chat_reading_width_off()
    } else {
        s::agent_chat_reading_width_on()
    };
    let reading_width = button_bare(("agent-chat-reading-width", props.pane_id as usize))
        .ghost()
        .xsmall()
        .icon(Icon::empty().path(ICON_WIDTH_WIDE))
        .tooltip(SharedString::from(reading_tooltip))
        .selected(reading_selected)
        .on_click(cx.listener(move |this, _ev, _window, cx| this.toggle_content_width(cx)));

    div()
        .flex_none()
        .w_full()
        .flex()
        .flex_row()
        .justify_between()
        .items_center()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .px(px(theme::AGENT_CHAT_PAD_X))
        .py(px(theme::AGENT_CHAT_PAD_Y))
        .border_b_1()
        // Background-derived hairline: the bar sits directly on the pane's
        // `agent_chat_bg` (mirrored terminal bg), where the fixed `t.border`
        // hairline is near-invisible. Matches the tool-card / code-block edges.
        .border_color(theme::dim_toward_gray(
            theme::agent_chat_border_tint(cx),
            props.dim,
        ))
        .child(
            div()
                .id(("agent-chat-title", props.pane_id as usize))
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::AGENT_CHAT_HEADER_ICON_GAP))
                .child(agent_icon(props.agent_id, props.dim, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(theme::agent_chat_font_size(cx)))
                        .text_color(theme::dim_toward_gray(theme::agent_chat_fg(cx), props.dim))
                        .child(title),
                )
                .when_some(last_active_tooltip, |el, tip| {
                    el.tooltip(crate::ui::tooltip::text(tip))
                }),
        )
        // Context-window meter (from `UsageUpdate`): current fill on the right,
        // detail + optional cost in the tooltip. Distinct from the cumulative
        // Usage tab. Shown only once the agent reports usage.
        .when_some(props.usage, |row, u| {
            let pct = u
                .used
                .saturating_mul(100)
                .checked_div(u.size)
                .map(|p| p.min(100) as u8)
                .unwrap_or(0);
            let cost = u
                .cost
                .as_ref()
                .map(|c| format!(" \u{00b7} {:.2} {}", c.amount, c.currency))
                .unwrap_or_default();
            let label = format!(
                "{} / {}",
                format_token_count(u.used),
                format_token_count(u.size)
            );
            let tip = s::agent_chat_context_tooltip(
                &format_token_count(u.used),
                &format_token_count(u.size),
                pct,
                &cost,
            );
            row.child(
                div()
                    .id(("agent-chat-context-meter", props.pane_id as usize))
                    .flex_none()
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .text_color(theme::dim_toward_gray(
                        theme::agent_chat_fg_muted(cx),
                        props.dim,
                    ))
                    .child(SharedString::from(label))
                    .tooltip(crate::ui::tooltip::text(SharedString::from(tip))),
            )
        })
        .child(
            div()
                .flex_none()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::AGENT_CHAT_MSG_GAP))
                .text_color(theme::dim_toward_gray(
                    theme::agent_chat_fg_muted(cx),
                    props.dim,
                ))
                .when(props.has_items, |bar| bar.child(expand).child(collapse))
                .child(reading_width),
        )
}

fn agent_icon(agent_id: &str, dim: f32, cx: &mut Context<AgentChatView>) -> AnyElement {
    crate::ui::agent_icon(
        crate::agent::icons::icon_for_agent(agent_id),
        px(theme::AGENT_CHAT_HEADER_ICON_SIZE),
        theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim),
    )
}

/// Format an ISO 8601 timestamp (`2026-07-01T14:32:05.000Z`) into a compact
/// `YYYY-MM-DD HH:MM` for the last-active tooltip. Best-effort by slicing the
/// canonical shape (date + `HH:MM`); returns the input unchanged when it does
/// not match, so a non-standard timestamp still shows rather than being dropped.
fn format_last_active(iso: &str) -> String {
    match iso.split_once('T') {
        Some((date, time)) if date.len() == 10 && time.len() >= 5 => {
            format!("{date} {}", &time[..5])
        }
        _ => iso.to_string(),
    }
}

/// Localized banner copy for a runtime-provisioning milestone.
fn runtime_prep_text(phase: RuntimePrepPhase) -> SharedString {
    match phase {
        RuntimePrepPhase::Downloading => s::agent_chat_runtime_downloading(),
        RuntimePrepPhase::Verifying => s::agent_chat_runtime_verifying(),
        RuntimePrepPhase::Extracting => s::agent_chat_runtime_extracting(),
    }
    .into()
}

/// Localized banner copy for a handshake milestone — refines the flat
/// "Connecting…" text once the connection task reports which ACP request is
/// in flight.
fn connect_phase_text(phase: ConnectPhase) -> SharedString {
    match phase {
        ConnectPhase::Handshaking => s::agent_chat_connecting_handshake(),
        ConnectPhase::CreatingSession => s::agent_chat_connecting_creating_session(),
        ConnectPhase::LoadingSession => s::agent_chat_connecting_loading_session(),
        ConnectPhase::ApplyingMode => s::agent_chat_connecting_applying_mode(),
    }
    .into()
}

/// The thin top banner — shown while connecting or on error; hidden once the
/// session is live (the conversation itself signals readiness). The `Error`
/// arm carries an inline "Retry" button (`window_handle` + `pane_id` locate
/// the owning `Workspace` op) — otherwise a failed connect has no way back
/// short of closing the pane; see `Workspace::retry_agent_chat_connect`.
/// `has_cwd` gates the button: a cwd-less pane's `Error` (no lane working
/// directory, or a remote agent with no configured remote path) can never
/// reconnect — `retry_agent_chat_connect` itself no-ops on it — so the
/// button would otherwise sit there doing nothing on every click.
///
/// The status's own [`Remedy`](daruda_acp::Remedy) gates it too. Retrying an
/// expired login or an organization-blocked account reconnects with the same
/// credentials and fails identically, so those classes get the message and no
/// button rather than a loop that looks like progress.
/// Whether the error banner may offer "Retry", as a pure decision so it can be
/// asserted without building an element.
///
/// Two independent gates, both necessary. `has_cwd` is mechanical: a cwd-less
/// pane cannot reconnect at all, and `retry_agent_chat_connect` no-ops on it.
/// The remedy is semantic: reconnecting after an expired login or an
/// organization-blocked account re-runs the identical handshake with the
/// identical credentials, so the button can only fail the same way — an
/// invitation to click forever while nothing changes.
pub(super) fn banner_offers_retry(status: &AgentSessionStatus, has_cwd: bool) -> bool {
    match status {
        AgentSessionStatus::Error { remedy, .. } => {
            has_cwd && matches!(remedy, daruda_acp::Remedy::Retry)
        }
        _ => false,
    }
}

/// Whether the error banner may offer a re-login, as a pure decision.
///
/// Deliberately NOT gated on `has_cwd`, unlike [`banner_offers_retry`]: that
/// gate exists because a cwd-less pane cannot *reconnect*, while signing in
/// again writes credentials the pane will read on its next connect regardless.
/// Inheriting it would hide the button on precisely the failures it fixes.
pub(super) fn banner_offers_reauth(status: &AgentSessionStatus) -> bool {
    match status {
        AgentSessionStatus::Error { remedy, .. } => {
            matches!(remedy, daruda_acp::Remedy::Reauthenticate)
        }
        _ => false,
    }
}

pub(super) fn status_banner(
    status: &AgentSessionStatus,
    pane_id: PaneId,
    window_handle: AnyWindowHandle,
    has_cwd: bool,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> Option<impl IntoElement + use<>> {
    let reauthable = banner_offers_reauth(status);
    let (text, bg, fg, retryable): (SharedString, Hsla, Hsla, bool) = match status {
        AgentSessionStatus::Idle => (
            s::agent_chat_idle().into(),
            t.banner_info_bg,
            t.banner_info_text,
            false,
        ),
        AgentSessionStatus::PreparingRuntime(phase) => (
            runtime_prep_text(*phase),
            t.banner_info_bg,
            t.banner_info_text,
            false,
        ),
        AgentSessionStatus::Connecting => (
            s::agent_chat_connecting().into(),
            t.banner_info_bg,
            t.banner_info_text,
            false,
        ),
        AgentSessionStatus::Handshaking(phase) => (
            connect_phase_text(*phase),
            t.banner_info_bg,
            t.banner_info_text,
            false,
        ),
        AgentSessionStatus::Connected => return None,
        AgentSessionStatus::Error { message, .. } => (
            format!("{} {}", s::agent_chat_error_prefix(), message).into(),
            t.banner_error_bg,
            t.banner_error_text,
            banner_offers_retry(status, has_cwd),
        ),
    };
    let retry_button = retryable.then(|| {
        super::blocks::banner_action_button(
            ("agent-chat-retry", pane_id as usize),
            s::agent_chat_retry(),
            t,
            cx,
        )
        .on_click(cx.listener(move |_this, _ev, _window, cx| {
            // `cx.listener` has this AgentChatView leased for the duration of
            // this callback; `retry_agent_chat_connect` reads/updates this
            // same entity (via `Workspace::agent_chat_view`), which would
            // double-lease-panic if called inline (CLAUDE.md Pitfall #5).
            // `cx.defer` runs after the lease is released.
            cx.defer(move |cx| {
                if let Some(workspace) =
                    crate::window_registry::WindowRegistry::workspace_for_window(window_handle, cx)
                {
                    // SILENT-OK: the workspace window may already be closed by the time this deferred callback runs — nothing left to retry
                    let _ = workspace.update(cx, |ws, cx| {
                        ws.retry_agent_chat_connect(pane_id, cx);
                    });
                }
            });
        }))
    });
    let reauth_button = reauthable.then(|| {
        super::blocks::banner_action_button(
            ("agent-chat-reauth", pane_id as usize),
            s::agent_chat_sign_in_again(),
            t,
            cx,
        )
        .on_click(cx.listener(move |_this, _ev, _window, cx| {
            // Same lease hazard as the retry button above: the login op
            // reaches this AgentChatView through `Workspace`, which would
            // double-lease-panic inline (CLAUDE.md Pitfall #5).
            cx.defer(move |cx| {
                if let Some(workspace) =
                    crate::window_registry::WindowRegistry::workspace_for_window(window_handle, cx)
                {
                    // SILENT-OK: the workspace window may already be closed by the time this deferred callback runs — nothing left to sign in for
                    let _ = workspace.update(cx, |ws, cx| {
                        ws.reauthenticate_pane_account(pane_id, cx);
                    });
                }
            });
        }))
    });
    Some(
        div()
            .flex_none()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(theme::AGENT_CHAT_MSG_GAP))
            .px(px(theme::AGENT_CHAT_PAD_X))
            .py(px(theme::AGENT_CHAT_PAD_Y))
            .bg(bg)
            .text_color(fg)
            .text_size(px(theme::agent_chat_font_size(cx)))
            .child(div().flex_1().min_w_0().child(text))
            .when_some(retry_button, |el, btn| el.child(btn))
            .when_some(reauth_button, |el, btn| el.child(btn)),
    )
}

/// Title of the tool call currently in progress, if any — drives the
/// ExecutingTool footer label. The agent runs calls sequentially, so the last
/// `InProgress` call is the live one.
fn running_tool_title(items: &[ChatItem]) -> Option<String> {
    items.iter().rev().find_map(|item| match item {
        ChatItem::ToolCall(tc) if tc.status.is_live() => Some(tc.title.clone()),
        _ => None,
    })
}

/// Collapse an adapter-supplied tool-call title to a single clean line for the
/// status label: runs of whitespace (incl. newlines and tabs) become one space
/// and the ends are trimmed, so a multi-line command title can never force a
/// line break or leak a raw `\n`. Horizontal length is left to the label's
/// `overflow_hidden` + ellipsis, so no arbitrary character cap is imposed here.
fn single_line_title(title: &str) -> String {
    title.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Human-readable elapsed time for the working indicator.
/// Formats as `"5s"` under a minute, `"1m05s"` at or over a minute.
fn format_elapsed(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{:02}s", secs / 60, secs % 60)
    }
}

/// Compact token count for the context meter: exact below 1000, otherwise
/// rounded to the nearest thousand with a `k` suffix (e.g. 53_000 → `53k`,
/// 200_000 → `200k`). Precision loss at the low end is irrelevant for a
/// context-window gauge whose values run in the tens of thousands.
fn format_token_count(n: u64) -> String {
    if n < 1000 {
        n.to_string()
    } else {
        format!("{}k", (n + 500) / 1000)
    }
}

/// Animated trailing dots (".", "..", "...") for any "in progress" label. Cycles
/// off the shared, CPU-gated `StatusPulseClock` — the pulse pump dirties each
/// agent chat view in the Working activity state (incl. background subagent
/// activity) every tick (`Workspace::notify_in_flight_agent_chats`), so callers
/// advance without a per-frame animation. Shared by the working footer /
/// indicator and the running tool-call badge.
pub(super) fn pulse_dots(cx: &gpui::App) -> String {
    let tick = cx
        .try_global::<StatusPulseClock>()
        .map(|c| c.tick)
        .unwrap_or(0);
    ".".repeat((tick % 3) as usize + 1)
}

/// The live activity label this turn: blocked on a permission prompt, running
/// background subagents, running a named tool, or otherwise generating prose.
/// The animated trailing dots are appended by [`working_indicator`]. Subagent
/// activity outranks the running-tool title because during a subagent run the
/// live tool is a (noisy) child call — the count is the signal the user wants.
fn working_status(content: &AgentChatView) -> SharedString {
    if content.has_pending_permission() {
        s::agent_chat_awaiting_permission().into()
    } else if let Some(running) = content.subagent_progress() {
        s::agent_chat_subagent_progress(running).into()
    } else if let Some(title) = running_tool_title(&content.items) {
        s::agent_chat_working_tool(&single_line_title(&title)).into()
    } else {
        s::agent_chat_working().into()
    }
}

/// Inline "agent is working" indicator, projected as the tail row of the last
/// turn for the whole time a turn is in flight (through tool execution and
/// streaming) — see `rows::project`'s gate. It lives *in* the conversation
/// flow, so the progress signal sits where the next response will appear. The
/// label gets animated trailing dots (".", "..", "...") off the shared
/// `StatusPulseClock` — the pulse pump dirties this view while it is in the
/// Working activity state (incl. background subagent activity)
/// (`Workspace::notify_in_flight_agent_chats`), so they advance without a
/// per-frame animation. Cancelling is the bottom-dock Stop button.
pub(super) fn working_indicator(
    content: &AgentChatView,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let base = working_status(content);
    let dots = pulse_dots(cx);
    let elapsed_label = content.activity_elapsed().map(format_elapsed);
    let mut row = div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_color(content.dim(theme::agent_chat_fg_subtle(cx)))
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(SharedString::from(format!("{base}{dots}"))),
        );
    if let Some(elapsed) = elapsed_label {
        row = row.child(
            div()
                .flex_none()
                .text_color(content.dim(theme::agent_chat_fg_muted(cx)))
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(SharedString::from(elapsed)),
        );
    }
    row
}

#[cfg(test)]
mod tests {
    use super::{
        AgentSessionStatus, banner_offers_reauth, banner_offers_retry, format_elapsed,
        format_last_active, format_token_count, running_tool_title, single_line_title,
    };
    use daruda_acp::{ChatItem, Remedy, ToolCallItem, ToolKindView, ToolStatusView};

    fn errored(remedy: Remedy) -> AgentSessionStatus {
        AgentSessionStatus::Error {
            message: "boom".to_owned(),
            remedy,
        }
    }

    /// The whole point of routing the banner through a remedy: an expired
    /// login used to get a Retry button that reconnected with the same
    /// credentials and failed identically.
    #[test]
    fn banner_withholds_retry_from_failures_it_cannot_fix() {
        for remedy in [
            Remedy::Reauthenticate,
            Remedy::ExternalAction,
            Remedy::Configure,
            Remedy::NoneAvailable,
        ] {
            assert!(
                !banner_offers_retry(&errored(remedy), true),
                "{remedy:?} is not fixed by reconnecting"
            );
        }
    }

    /// The cwd gate belongs to Retry alone. Signing in again fixes an expired
    /// login whether or not the pane has a working directory, so inheriting
    /// that gate would hide the button on exactly the failures it can fix.
    #[test]
    fn the_reauth_button_does_not_inherit_the_retry_cwd_gate() {
        assert!(banner_offers_reauth(&errored(Remedy::Reauthenticate)));
        assert!(!banner_offers_retry(&errored(Remedy::Reauthenticate), true));
    }

    /// An organization-blocked account is the case this separation exists for:
    /// signing in again succeeds and changes nothing, so the button must not
    /// appear next to a message that already says to ask an admin.
    #[test]
    fn only_a_reauthenticable_failure_offers_the_button() {
        for remedy in [
            Remedy::Retry,
            Remedy::ExternalAction,
            Remedy::Configure,
            Remedy::NoneAvailable,
        ] {
            assert!(
                !banner_offers_reauth(&errored(remedy)),
                "{remedy:?} is not fixed by signing in again"
            );
        }
    }

    #[test]
    fn no_status_but_error_offers_a_reauth_button() {
        for status in [
            AgentSessionStatus::Idle,
            AgentSessionStatus::Connecting,
            AgentSessionStatus::Connected,
        ] {
            assert!(!banner_offers_reauth(&status), "{status:?}");
        }
    }

    #[test]
    fn banner_offers_retry_only_for_a_transient_failure_on_a_connectable_pane() {
        assert!(banner_offers_retry(&errored(Remedy::Retry), true));
        // Mechanical gate still applies: no cwd, nothing to reconnect to.
        assert!(!banner_offers_retry(&errored(Remedy::Retry), false));
    }

    /// Every non-error status renders its own copy and never a retry button.
    #[test]
    fn banner_offers_no_retry_outside_the_error_status() {
        for status in [
            AgentSessionStatus::Idle,
            AgentSessionStatus::Connecting,
            AgentSessionStatus::Connected,
        ] {
            assert!(!banner_offers_retry(&status, true), "{status:?}");
        }
    }

    #[test]
    fn single_line_title_collapses_newlines_tabs_and_runs() {
        assert_eq!(
            single_line_title("git commit -m \"line one\nline two\""),
            "git commit -m \"line one line two\""
        );
        assert_eq!(single_line_title("  foo\t\tbar \n baz  "), "foo bar baz");
        assert_eq!(single_line_title("already clean"), "already clean");
    }

    #[test]
    fn format_last_active_cases() {
        for (input, expected) in [
            ("2026-07-01T14:32:05.123Z", "2026-07-01 14:32"),
            ("2026-12-25T09:00:00+09:00", "2026-12-25 09:00"),
            ("not-a-timestamp", "not-a-timestamp"),
            ("2026-07-01", "2026-07-01"),
        ] {
            assert_eq!(format_last_active(input), expected);
        }
    }

    fn tool(id: &str, status: ToolStatusView) -> ChatItem {
        ChatItem::ToolCall(ToolCallItem {
            id: id.to_owned(),
            title: format!("Tool {id}"),
            kind: ToolKindView::Edit,
            tool_name: None,
            status,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: None,
            exit: None,
        })
    }

    #[test]
    fn running_tool_title_cases() {
        let items = [
            ChatItem::AssistantText {
                text: "a".into(),
                streaming: true,
                message_id: None,
            },
            tool("c1", ToolStatusView::Completed),
        ];
        assert_eq!(running_tool_title(&items), None);

        // Settled (Completed) calls are skipped; the latest *live* one wins.
        // `Pending` counts as live (see `ToolStatusView::is_live`), so a trailing
        // Pending call outranks an earlier InProgress one.
        let items = [
            tool("c1", ToolStatusView::Completed),
            tool("c2", ToolStatusView::InProgress),
            tool("c3", ToolStatusView::Pending),
        ];
        assert_eq!(running_tool_title(&items), Some("Tool c3".to_owned()));
    }

    #[test]
    fn format_elapsed_cases() {
        for (secs, expected) in [
            (0, "0s"),
            (5, "5s"),
            (60, "1m00s"),
            (65, "1m05s"),
            (600, "10m00s"),
        ] {
            assert_eq!(
                format_elapsed(std::time::Duration::from_secs(secs)),
                expected
            );
        }
    }

    #[test]
    fn format_token_count_cases() {
        for (tokens, expected) in [
            (0, "0"),
            (512, "512"),
            (999, "999"),
            (1000, "1k"),
            (1500, "2k"),
            (53_000, "53k"),
            (200_000, "200k"),
        ] {
            assert_eq!(format_token_count(tokens), expected);
        }
    }
}
