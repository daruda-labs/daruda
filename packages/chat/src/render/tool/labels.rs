//! Tool-card header vocabulary: title summary, command tags, labels, icons,
//! and the exit and status badges.

use super::*;

/// Collapse a multiline tool title to its first line + "…" so the collapsed
/// header's inline summary always stays a single line. Safe to truncate for
/// every kind (including `Execute`) now: the full, untruncated title has its
/// own line in the expanded body (see `tool_card`), so nothing is lost by
/// truncating here — the card is always expandable. A single-line title — or
/// one whose only newline is trailing (no real second line) — is returned
/// unchanged, so the "…" only appears when content is actually hidden.
pub(super) fn tool_title_summary(title: &str) -> String {
    match title.split_once('\n') {
        Some((first, rest)) if !rest.trim().is_empty() => format!("{first}…"),
        _ => title.to_string(),
    }
}

/// Render cached metadata only; raw command parsing belongs to row reconciliation.
pub(super) fn command_tags(
    tool_id: &str,
    analysis: Option<&CommandAnalysis>,
    dim: f32,
    cx: &App,
) -> Vec<AnyElement> {
    let Some(analysis) = analysis else {
        return Vec::new();
    };
    if analysis.programs.is_empty() {
        return Vec::new();
    }
    let bg = theme::dim_toward_gray(theme::agent_chat_tint(cx), dim);
    let border = theme::dim_toward_gray(theme::agent_chat_border_tint(cx), dim);
    let fg = theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim);
    let badge = |label: String| {
        daruda_ui::Badge::new(label)
            .monospace()
            .bg_color(bg)
            .border_color(border)
            .text_color(fg)
    };
    let mut tags = Vec::with_capacity(analysis.programs.len() + 1);
    for (index, program) in analysis.programs_in_display_order().into_iter().enumerate() {
        tags.push(
            div()
                .id(SharedString::from(format!(
                    "command-program-{tool_id}-{index}"
                )))
                .flex()
                .min_w_0()
                .max_w(px(theme::AGENT_CHAT_COMMAND_TAG_MAX_W))
                .child(badge(program.clone()).truncate())
                .tooltip(daruda_ui::tooltip::text(program.clone()))
                .into_any_element(),
        );
    }
    if let Some(effect) = analysis.effect {
        let light = theme::agent_chat_syntax_is_light(cx);
        let (label, color) = match (effect, light) {
            (CommandEffect::Read, false) => (s::agent_chat::read(), theme::AGENT_READING),
            (CommandEffect::Read, true) => (s::agent_chat::read(), theme::AGENT_READING_LIGHT),
            (CommandEffect::Edit, false) => (s::agent_chat::edit(), theme::AGENT_EDITING),
            (CommandEffect::Edit, true) => (s::agent_chat::edit(), theme::AGENT_EDITING_LIGHT),
        };
        let color = theme::dim_toward_gray(color, dim);
        tags.push(
            badge(label)
                .text_color(color)
                .bg_color(theme::with_alpha(color, theme::AGENT_CHAT_CARD_TINT_ALPHA))
                .border_color(theme::with_alpha(
                    color,
                    theme::AGENT_CHAT_CARD_BORDER_ALPHA,
                ))
                .into_any_element(),
        );
    }
    tags
}

/// Prefer the agent's tool name; nameless launches fall back to "Subagent".
pub(super) fn tool_header_label(tc: &ToolCallItem) -> String {
    tc.tool_name.clone().unwrap_or_else(|| {
        if tc.is_subagent_launch() {
            s::agent_chat::tool_kind_subagent()
        } else {
            tool_kind_label(tc.kind)
        }
    })
}

/// Map a tool kind to its short, fixed-vocabulary header label (Read/Edit/
/// Search/…), independent of the (possibly long) per-call title.
pub(super) fn tool_kind_label(kind: ToolKindView) -> String {
    match kind {
        ToolKindView::Read => s::agent_chat::read(),
        ToolKindView::Edit => s::agent_chat::edit(),
        ToolKindView::Delete => s::agent_chat::tool_kind_delete(),
        ToolKindView::Move => s::agent_chat::tool_kind_move(),
        ToolKindView::Search => s::agent_chat::tool_kind_search(),
        ToolKindView::Execute => s::agent_chat::tool_kind_execute(),
        ToolKindView::Think => s::agent_chat::tool_kind_think(),
        ToolKindView::Fetch => s::agent_chat::tool_kind_fetch(),
        ToolKindView::SwitchMode => s::agent_chat::tool_kind_switch_mode(),
        ToolKindView::Other => s::agent_chat::tool_kind_other(),
    }
}

/// Material Symbols "psychology" — the reasoning mark. The vendored lucide set
/// carries no such glyph, so `Think` reaches outside it (see [`tool_icon`]).
pub(super) const ICON_THINK: &str = "icons/ui/psychology.svg";

/// The category a call's *name* answers for, when no ACP kind can: a launch
/// arrives as [`ToolKindView::Think`] and an MCP tool as [`ToolKindView::Other`],
/// each sharing that kind with something else.
fn name_keyed_category(tc: &ToolCallItem) -> Option<ToolCategory> {
    if tc.is_subagent_launch() {
        return Some(ToolCategory::Agent);
    }
    tc.tool_name
        .as_deref()
        .is_some_and(is_mcp_tool_name)
        .then_some(ToolCategory::Mcp)
}

/// The glyph for a category [`name_keyed_category`] answers for; `None` when
/// the kind answers instead. One home, so a card and its group bar cannot pick
/// two different glyphs for the same question.
pub(in crate::render) fn name_keyed_icon(category: ToolCategory) -> Option<SharedString> {
    match category {
        ToolCategory::Agent => Some(IconName::Bot.path()),
        ToolCategory::Mcp => Some(IconName::ExternalLink.path()),
        _ => None,
    }
}

/// The asset behind a tool call's leading header icon, mirroring zed's
/// kind-based icon. Returns a path rather than an `IconName` because two glyph
/// families meet here — lucide is named, Material is not.
pub(in crate::render) fn tool_icon(tc: &ToolCallItem) -> SharedString {
    name_keyed_category(tc)
        .and_then(name_keyed_icon)
        .unwrap_or_else(|| tool_kind_icon(tc.kind))
}

/// The kind half of [`tool_icon`], for a caller that has a kind and no call —
/// the group bar, which maps its category to a representative kind.
pub(in crate::render) fn tool_kind_icon(kind: ToolKindView) -> SharedString {
    // The vendored `IconName` set has no pencil/edit glyph, so Edit falls back
    // to `File` (Read already uses `Eye`, so no visual collision).
    match kind {
        ToolKindView::Read => IconName::Eye.path(),
        ToolKindView::Edit => IconName::File.path(),
        ToolKindView::Delete => IconName::Delete.path(),
        ToolKindView::Move => IconName::ArrowRight.path(),
        ToolKindView::Search => IconName::Search.path(),
        ToolKindView::Execute => IconName::SquareTerminal.path(),
        ToolKindView::Think => ICON_THINK.into(),
        ToolKindView::Fetch => IconName::Globe.path(),
        ToolKindView::SwitchMode => IconName::Refresh.path(),
        ToolKindView::Other => IconName::Settings2.path(),
    }
}

/// The exit-status badge text for a shell tool call, if abnormal. `None` for a
/// clean exit (code 0, no signal), an unreported exit, or a reported-but-empty
/// one (`code: None, signal: None` — a side channel that had nothing usable).
/// A signal takes priority over the code when both are present, since a
/// signal-killed process's reported exit code is rarely the meaningful part.
pub(super) fn exit_badge_label(exit: &Option<CommandExit>) -> Option<String> {
    let exit = exit.as_ref()?;
    if let Some(signal) = exit.signal.as_deref() {
        return Some(s::agent_chat::tool_exit_signal(signal));
    }
    match exit.code {
        Some(code) if code != 0 => Some(s::agent_chat::tool_exit_code(code)),
        _ => None,
    }
}

/// Map a tool status to its badge label + colour.
/// The word behind the status mark — the mark's tooltip, so an icon-only badge
/// does not cost the localized reading.
pub(super) fn tool_status_badge(status: ToolStatusView) -> SharedString {
    match status {
        // `Pending` and `InProgress` both read as "running": the adapter marks
        // every call `Pending` until an SDK progress ping (which many tools
        // never get), and a live `Pending` always means an in-flight call in the
        // active turn (see `ToolStatusView::is_live`).
        ToolStatusView::Pending | ToolStatusView::InProgress => {
            s::agent_chat::tool_status_running().into()
        }
        ToolStatusView::Completed => s::agent_chat::tool_status_done().into(),
        ToolStatusView::Failed => s::agent_chat::tool_status_failed().into(),
        ToolStatusView::Cancelled => s::agent_chat::tool_status_cancelled().into(),
    }
}
