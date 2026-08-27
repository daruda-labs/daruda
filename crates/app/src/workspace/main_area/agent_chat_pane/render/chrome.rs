//! Pane chrome around the conversation: the top status banner, the activity
//! bar (title + icon controls), and the inline "agent is working" indicator
//! with its animated pulse dots and elapsed clock.

use daruda_acp::{ChatItem, ConnectPhase, UsageView};
use daruda_config::TAIL_WINDOW_CHOICES;
use gpui::{
    AnyElement, AnyWindowHandle, Context, Hsla, IntoElement, SharedString, div, prelude::*, px,
};

use crate::surface::strings as s;
use crate::surface::timestamp;
use crate::ui::theme;
use crate::ui::{
    ButtonVariants as _, DropdownMenu as _, Icon, PopupMenu, PopupMenuItem, Selectable as _,
    Sizable as _, StatusPulseClock, button, button_bare,
};
use crate::workspace::main_area::agent_chat_pane::display_filter::DisplayFilter;
use crate::workspace::main_area::agent_chat_pane::fold_mode::FoldMode;
use crate::workspace::main_area::agent_chat_pane::rows::tail::TailWindow;
use crate::workspace::main_area::agent_chat_pane::view::{
    AgentChatView, AgentSessionStatus, ChatContentWidth, RuntimePrepPhase,
};
use crate::workspace::main_area::pane_tree::PaneId;

const ICON_EXPAND: &str = "icons/ui/expand.svg";
const ICON_COMPRESS: &str = "icons/ui/compress.svg";
const ICON_WIDTH_WIDE: &str = "icons/ui/width-wide.svg";

/// Pane activity bar: resolved session title on the left, the context-window
/// meter and the icon controls on the right, with a bottom hairline against the
/// conversation body. Always rendered, so the reading-width toggle stays
/// reachable while the conversation is empty or still connecting. `title` is
/// already resolved by the caller (`activity_bar_title`, falling back to the
/// agent name).
pub(super) struct ActivityBarProps<'a> {
    pub pane_id: PaneId,
    pub agent_id: &'a str,
    pub title: Option<&'a str>,
    pub last_active: Option<&'a str>,
    pub usage: Option<&'a UsageView>,
    pub has_items: bool,
    pub content_width: ChatContentWidth,
    pub tail: TailWindow,
    pub display_filter: DisplayFilter,
    pub fold_mode: FoldMode,
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
    // A tooltip rather than inline text: low-frequency detail, and the bar is
    // width-constrained (the title ellipsizes). The agent reports it via
    // `SessionInfoUpdate`; `reconcile_activity` advances it on each settle.
    let last_active = props.last_active.map(last_active_tooltip);

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
    let tail = tail_window_chip(props.pane_id, props.tail, cx);
    let display_filter =
        super::filter::display_filter_chip(props.pane_id, props.display_filter, cx);
    let fold_mode = super::fold_mode::fold_mode_chip(props.pane_id, props.fold_mode, cx);
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
                .when_some(last_active, |el, tip| {
                    el.tooltip(crate::ui::tooltip::text(tip))
                }),
        )
        // Context-window meter (from `UsageUpdate`): current fill on the right,
        // detail + optional cost in the tooltip. Distinct from the cumulative
        // Usage tab.
        .when_some(props.usage.map(context_meter), |row, meter| {
            row.child(
                div()
                    .id(("agent-chat-context-meter", props.pane_id as usize))
                    .flex_none()
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .text_color(theme::dim_toward_gray(
                        theme::agent_chat_fg_muted(cx),
                        props.dim,
                    ))
                    .child(SharedString::from(meter.label))
                    .tooltip(crate::ui::tooltip::text(SharedString::from(meter.tooltip))),
            )
        })
        .child(
            div()
                .flex_shrink()
                .min_w_0()
                .max_w_full()
                .flex()
                .flex_row()
                .flex_wrap()
                .justify_end()
                .items_center()
                .gap(px(theme::AGENT_CHAT_MSG_GAP))
                .text_color(theme::dim_toward_gray(
                    theme::agent_chat_fg_muted(cx),
                    props.dim,
                ))
                .when(props.has_items, |bar| {
                    bar.child(fold_mode)
                        .child(display_filter)
                        .child(tail)
                        .child(expand)
                        .child(collapse)
                })
                .child(reading_width),
        )
}

/// Activity-bar chip for the tail window.
fn tail_window_chip(
    pane_id: PaneId,
    tail: TailWindow,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let label = SharedString::from(s::agent_chat_tail_window_chip(&tail_window_value(tail)));
    let view = cx.entity().downgrade();
    button(("agent-chat-tail-window", pane_id as usize), label)
        .ghost()
        .xsmall()
        .tooltip(SharedString::from(s::agent_chat_tail_window_tooltip()))
        .dropdown_menu(move |menu, _window, _cx| build_tail_window_menu(&view, tail, menu))
}

/// The chip's value slot reuses the menu item's own wording, so the chip and
/// the item that set it read alike — and a bare count can't be mistaken for the
/// "N earlier steps" row it sits above.
fn tail_window_value(tail: TailWindow) -> String {
    match tail {
        TailWindow::All => s::agent_chat_tail_window_all(),
        TailWindow::Last(n) => s::agent_chat_tail_window_last(n),
    }
}

fn build_tail_window_menu(
    view: &gpui::WeakEntity<AgentChatView>,
    current: TailWindow,
    menu: PopupMenu,
) -> PopupMenu {
    let choices = std::iter::once((TailWindow::All, s::agent_chat_tail_window_all())).chain(
        TAIL_WINDOW_CHOICES.into_iter().map(|n| {
            (
                TailWindow::last(n),
                s::agent_chat_tail_window_last(usize::from(n)),
            )
        }),
    );
    choices.fold(menu, |m, (choice, name)| {
        let view = view.clone();
        m.item(
            PopupMenuItem::new(SharedString::from(name))
                .checked(choice == current)
                .on_click(move |_, _window, app| {
                    if let Some(view) = view.upgrade() {
                        view.update(app, |v, cx| v.set_tail_window(choice, cx));
                    }
                }),
        )
    })
}

fn agent_icon(agent_id: &str, dim: f32, cx: &mut Context<AgentChatView>) -> AnyElement {
    crate::ui::agent_icon(
        crate::agent::icons::icon_for_agent(agent_id),
        px(theme::AGENT_CHAT_HEADER_ICON_SIZE),
        theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim),
    )
}

/// The context meter's two pieces of copy, derived from one `UsageUpdate`.
struct ContextMeter {
    label: String,
    tooltip: String,
}

/// Derive the context-meter copy. `checked_div` keeps a size-0 window — seen
/// before the first real `UsageUpdate` — from dividing by zero.
fn context_meter(u: &UsageView) -> ContextMeter {
    let used = format_token_count(u.used);
    let size = format_token_count(u.size);
    let percent = u
        .used
        .saturating_mul(100)
        .checked_div(u.size)
        .map(|p| p.min(100) as u8)
        .unwrap_or(0);
    ContextMeter {
        label: s::agent_chat_context_meter(&used, &size),
        tooltip: match &u.cost {
            Some(c) => s::agent_chat_context_tooltip_with_cost(
                &used,
                &size,
                percent,
                &format!("{:.2}", c.amount),
                &c.currency,
            ),
            None => s::agent_chat_context_tooltip(&used, &size, percent),
        },
    }
}

/// The title's tooltip: when this session was last active, in the machine's
/// local zone. A timestamp we cannot parse shows verbatim — the protocol
/// promises ISO 8601, which is wider than the RFC 3339 subset we read, so the
/// agent's own wording beats an empty tooltip.
fn last_active_tooltip(iso: &str) -> SharedString {
    let when = timestamp::local_datetime(iso).unwrap_or_else(|| iso.to_owned());
    SharedString::from(s::agent_chat_last_active_tooltip(&when))
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
mod tests;
