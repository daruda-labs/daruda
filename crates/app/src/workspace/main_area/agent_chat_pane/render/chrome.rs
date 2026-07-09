//! Pane chrome around the conversation: the top status banner, the activity
//! bar (title + expand/collapse), and the inline "agent is working" indicator
//! with its animated pulse dots and elapsed clock.

use daruda_acp::{ChatItem, UsageView};
use gpui::{AnyElement, App, Hsla, IntoElement, SharedString, div, prelude::*, px, svg};

use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{ButtonVariants as _, Icon, IconName, Sizable as _, StatusPulseClock};
use crate::workspace::main_area::agent_chat_pane::view::{
    AgentChatView, AgentSessionStatus, RuntimePrepPhase,
};
use crate::workspace::main_area::pane_tree::PaneId;

/// Pane activity bar: resolved session title on the LEFT, "Expand all" /
/// "Collapse all" ghost buttons on the RIGHT. Always rendered — it holds the
/// fold buttons even while the conversation is empty or still connecting. The
/// `title` is already resolved by the caller (`activity_bar_title`, falling
/// back to the agent name). The fold buttons appear only when `has_items` is
/// true (render purity: no logic here, just `.when()`).
/// A bottom hairline separates the bar from the conversation body.
pub(super) struct ActivityBarProps<'a> {
    pub pane_id: PaneId,
    pub agent_id: &'a str,
    pub title: Option<&'a str>,
    pub last_active: Option<&'a str>,
    pub usage: Option<&'a UsageView>,
    pub has_items: bool,
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

    let expand = crate::ui::button(
        ("agent-chat-expand-all", props.pane_id as usize),
        SharedString::from(s::agent_chat_expand_all()),
    )
    .ghost()
    .xsmall()
    .on_click(cx.listener(move |this, _ev, _window, cx| this.set_all_folds(true, cx)));
    let collapse = crate::ui::button(
        ("agent-chat-collapse-all", props.pane_id as usize),
        SharedString::from(s::agent_chat_collapse_all()),
    )
    .ghost()
    .xsmall()
    .on_click(cx.listener(move |this, _ev, _window, cx| this.set_all_folds(false, cx)));

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
            let pct = if u.size > 0 {
                (u.used.saturating_mul(100) / u.size).min(100) as u8
            } else {
                0
            };
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
        .when(props.has_items, |row| {
            row.child(
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
                    .child(expand)
                    .child(collapse),
            )
        })
}

fn agent_icon(agent_id: &str, dim: f32, cx: &mut Context<AgentChatView>) -> AnyElement {
    let color = theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim);
    match agent_icon_path(agent_id) {
        Some(path) => svg()
            .flex_none()
            .w(px(theme::AGENT_CHAT_HEADER_ICON_SIZE))
            .h(px(theme::AGENT_CHAT_HEADER_ICON_SIZE))
            .path(path)
            .text_color(color)
            .into_any_element(),
        None => Icon::new(IconName::Bot)
            .xsmall()
            .text_color(color)
            .into_any_element(),
    }
}

fn agent_icon_path(agent_id: &str) -> Option<&'static str> {
    Some(match agent_id {
        "claude" | "claude-acp" => "icons/agents/claude-acp.svg",
        "codex" | "codex-acp" => "icons/agents/codex-acp.svg",
        "agoragentic-acp" => "icons/agents/agoragentic-acp.svg",
        "amp-acp" => "icons/agents/amp-acp.svg",
        "auggie" => "icons/agents/auggie.svg",
        "autohand" => "icons/agents/autohand.svg",
        "cline" => "icons/agents/cline.svg",
        "codebuddy-code" => "icons/agents/codebuddy-code.svg",
        "cortex-code" => "icons/agents/cortex-code.svg",
        "corust-agent" => "icons/agents/corust-agent.svg",
        "crow-cli" => "icons/agents/crow-cli.svg",
        "cursor" => "icons/agents/cursor.svg",
        "deepagents" => "icons/agents/deepagents.svg",
        "devin" => "icons/agents/devin.svg",
        "dimcode" => "icons/agents/dimcode.svg",
        "dirac" => "icons/agents/dirac.svg",
        "factory-droid" => "icons/agents/factory-droid.svg",
        "fast-agent" => "icons/agents/fast-agent.svg",
        "gemini" => "icons/agents/gemini.svg",
        "github-copilot-cli" => "icons/agents/github-copilot-cli.svg",
        "glm-acp-agent" => "icons/agents/glm-acp-agent.svg",
        "goose" => "icons/agents/goose.svg",
        "grok-build" => "icons/agents/grok-build.svg",
        "harn" => "icons/agents/harn.svg",
        "junie" => "icons/agents/junie.svg",
        "kilo" => "icons/agents/kilo.svg",
        "kimi" => "icons/agents/kimi.svg",
        "minion-code" => "icons/agents/minion-code.svg",
        "mistral-vibe" => "icons/agents/mistral-vibe.svg",
        "nova" => "icons/agents/nova.svg",
        "opencode" => "icons/agents/opencode.svg",
        "pi-acp" => "icons/agents/pi-acp.svg",
        "poolside" => "icons/agents/poolside.svg",
        "qoder" => "icons/agents/qoder.svg",
        "qwen-code" => "icons/agents/qwen-code.svg",
        "sigit" => "icons/agents/sigit.svg",
        "stakpak" => "icons/agents/stakpak.svg",
        "vtcode" => "icons/agents/vtcode.svg",
        _ => return None,
    })
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

/// The thin top banner — shown while connecting or on error; hidden once
/// the session is live (the conversation itself signals readiness).
pub(super) fn status_banner(
    status: &AgentSessionStatus,
    t: &theme::DarudaTheme,
    cx: &App,
) -> Option<impl IntoElement + use<>> {
    let (text, bg, fg): (SharedString, Hsla, Hsla) = match status {
        AgentSessionStatus::Idle => (
            s::agent_chat_idle().into(),
            t.banner_info_bg,
            t.banner_info_text,
        ),
        AgentSessionStatus::PreparingRuntime(phase) => (
            runtime_prep_text(*phase),
            t.banner_info_bg,
            t.banner_info_text,
        ),
        AgentSessionStatus::Connecting => (
            s::agent_chat_connecting().into(),
            t.banner_info_bg,
            t.banner_info_text,
        ),
        AgentSessionStatus::Connected => return None,
        AgentSessionStatus::Error(message) => (
            format!("{} {}", s::agent_chat_error_prefix(), message).into(),
            t.banner_error_bg,
            t.banner_error_text,
        ),
    };
    Some(
        div()
            .flex_none()
            .w_full()
            .px(px(theme::AGENT_CHAT_PAD_X))
            .py(px(theme::AGENT_CHAT_PAD_Y))
            .bg(bg)
            .text_color(fg)
            .text_size(px(theme::agent_chat_font_size(cx)))
            .child(text),
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
    if content.pending_permission.is_some() {
        s::agent_chat_awaiting_permission().into()
    } else if let Some(running) = content.subagent_progress() {
        s::agent_chat_subagent_progress(running).into()
    } else if let Some(title) = running_tool_title(&content.items) {
        s::agent_chat_working_tool(&title).into()
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
    use super::{format_elapsed, format_last_active, format_token_count, running_tool_title};
    use daruda_acp::{ChatItem, ToolCallItem, ToolKindView, ToolStatusView};

    #[test]
    fn format_last_active_slices_canonical_iso_to_date_and_hh_mm() {
        assert_eq!(
            format_last_active("2026-07-01T14:32:05.123Z"),
            "2026-07-01 14:32"
        );
        assert_eq!(
            format_last_active("2026-12-25T09:00:00+09:00"),
            "2026-12-25 09:00"
        );
    }

    #[test]
    fn format_last_active_returns_unrecognized_input_unchanged() {
        // No 'T' separator / wrong shape → best-effort passthrough, not a panic.
        assert_eq!(format_last_active("not-a-timestamp"), "not-a-timestamp");
        assert_eq!(format_last_active("2026-07-01"), "2026-07-01");
    }

    fn tool(id: &str, status: ToolStatusView) -> ChatItem {
        ChatItem::ToolCall(ToolCallItem {
            id: id.to_owned(),
            title: format!("Tool {id}"),
            kind: ToolKindView::Edit,
            status,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: None,
        })
    }

    #[test]
    fn running_tool_title_is_none_without_an_in_progress_call() {
        let items = [
            ChatItem::AssistantText {
                text: "a".into(),
                streaming: true,
                message_id: None,
            },
            tool("c1", ToolStatusView::Completed),
        ];
        assert_eq!(running_tool_title(&items), None);
    }

    #[test]
    fn running_tool_title_picks_the_last_live_call() {
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
    fn format_elapsed_zero_seconds() {
        assert_eq!(format_elapsed(std::time::Duration::from_secs(0)), "0s");
    }

    #[test]
    fn format_elapsed_five_seconds() {
        assert_eq!(format_elapsed(std::time::Duration::from_secs(5)), "5s");
    }

    #[test]
    fn format_elapsed_sixty_five_seconds() {
        assert_eq!(format_elapsed(std::time::Duration::from_secs(65)), "1m05s");
    }

    #[test]
    fn format_elapsed_at_one_minute_boundary() {
        assert_eq!(format_elapsed(std::time::Duration::from_secs(60)), "1m00s");
    }

    #[test]
    fn format_elapsed_six_hundred_seconds() {
        assert_eq!(
            format_elapsed(std::time::Duration::from_secs(600)),
            "10m00s"
        );
    }

    #[test]
    fn format_token_count_is_exact_below_one_thousand() {
        assert_eq!(format_token_count(0), "0");
        assert_eq!(format_token_count(512), "512");
        assert_eq!(format_token_count(999), "999");
    }

    #[test]
    fn format_token_count_rounds_to_nearest_thousand_with_k() {
        assert_eq!(format_token_count(1000), "1k");
        assert_eq!(format_token_count(1500), "2k");
        assert_eq!(format_token_count(53_000), "53k");
        assert_eq!(format_token_count(200_000), "200k");
    }
}
