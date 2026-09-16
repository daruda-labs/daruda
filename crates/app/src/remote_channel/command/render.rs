//! Turning a control outcome into the message a phone shows.
//!
//! Separate from the conversation state next door because the two change for
//! different reasons: what an ordinal points at is a protocol question, and
//! how a row reads on a small screen is a presentation one. The renderer is
//! handed [`CommandState`] read-only, for the two things a reply needs from
//! it — a row's display name, and the callback token behind its button.

use super::{Absorbed, CommandState, ListingRow};
use crate::control::result::{
    Activity, AskDisposition, ChatSummary, ControlError, ControlOutcome, ControlResult,
    FlowOriginKind, Health, Listing, PaneAnswer, SendDisposition, StopDisposition,
};
use crate::control::spec::{Ordinal, ParseError};
use crate::remote_channel::bridge::InlineKeyboard;
use crate::surface::strings as s;

/// Budget for one rendered reply, in bytes. Telegram's `sendMessage` limit is
/// the widest of the three channels; Slack and Discord chunk what exceeds
/// their own narrower section and content limits.
const REPLY_MAX_BYTES: usize = 4096;

/// Room kept for the trailing "and N more" line when a listing overflows.
const OVERFLOW_RESERVE_BYTES: usize = 128;

/// Buttons per keyboard row. Telegram lays a row out horizontally, so a wider
/// row shrinks each label past what a thumb can hit.
const BUTTONS_PER_ROW: usize = 3;

/// Cap on listing buttons. Past this the keyboard is taller than the message
/// it belongs to; the ordinals still work by typing `/use <n>`.
const LISTING_BUTTON_MAX: usize = 24;

/// A rendered command reply, ready for the outbound send path.
pub(crate) struct RenderedReply {
    pub text: String,
    pub keyboard: Option<InlineKeyboard>,
}

/// Render a command outcome for the phone. `state` supplies the ordinals the
/// listing buttons encode, so it is passed in already updated; `absorbed`
/// carries anything [`absorb`](super::absorb) changed on the way that the
/// outcome alone does not say.
pub(crate) fn render(
    outcome: &ControlOutcome,
    absorbed: Absorbed,
    state: &CommandState,
) -> RenderedReply {
    let mut reply = match outcome {
        Ok(result) => render_result(result, state),
        Err(error) => RenderedReply {
            text: render_error(error),
            keyboard: None,
        },
    };
    if absorbed == Absorbed::SelectionDropped {
        reply.text.push('\n');
        reply.text.push_str(&s::control_selection_dropped());
    }
    reply
}

/// Render a parse failure. Separate from [`render_error`] because a parse
/// failure never reached the executor and so has no `ControlError` form —
/// mapping it to one would invent a code no executor produced.
pub(crate) fn render_parse_error(error: &ParseError) -> RenderedReply {
    let text = match error {
        // The adapter routes this to plain text and never renders it; a
        // sentence is still better than an empty message if it ever arrives.
        ParseError::NotACommand => s::control_error_no_target(),
        ParseError::Unknown {
            input,
            suggestion: Some(suggestion),
        } => s::control_error_unknown_command_did_you_mean(input, suggestion),
        ParseError::Unknown {
            input,
            suggestion: None,
        } => s::control_error_unknown_command(input),
        ParseError::MissingArgument { command } => {
            s::control_error_missing_argument(&usage_for(command))
        }
        ParseError::BadOrdinal { input } => s::control_error_bad_ordinal(input),
    };
    RenderedReply {
        text,
        keyboard: None,
    }
}

fn render_result(result: &ControlResult, state: &CommandState) -> RenderedReply {
    match result {
        ControlResult::Listing(listing) => render_listing(listing, state),
        ControlResult::Selected { target: None } => plain(s::control_selection_cleared()),
        ControlResult::Selected {
            target: Some(summary),
        } => plain(s::control_selected(
            &state
                .label_for(summary.target)
                .unwrap_or_else(|| bare_label(summary)),
        )),
        ControlResult::Sent { disposition, .. } => plain(match disposition {
            SendDisposition::Delivered => s::control_sent_delivered(),
            SendDisposition::Queued => s::control_sent_queued(),
            SendDisposition::HandledLocally => s::control_sent_handled_locally(),
        }),
        ControlResult::Stopped { disposition, .. } => plain(match disposition {
            StopDisposition::Stopped => s::control_stopped(),
            StopDisposition::AlreadyIdle => s::control_stop_already_idle(),
        }),
        ControlResult::FlowList { flows } if flows.is_empty() => {
            plain(s::control_flow_list_empty())
        }
        ControlResult::FlowList { flows } => {
            let mut rows: Vec<String> = flows
                .iter()
                .map(|e| s::control_flow_list_row(&e.name, &origin_label(e.origin)))
                .collect();
            // One row per name, not per worktree: the phone runs a flow in
            // whichever window's active worktree has it, so two windows
            // offering the same name are one choice to the person reading
            // this. The rows arrive sorted by name, so duplicates are
            // neighbours.
            rows.dedup();
            plain(rows.join("\n"))
        }
        ControlResult::FlowStarting { name, .. } => plain(s::control_flow_starting(name)),
        ControlResult::FlowStopped { disposition, .. } => plain(match disposition {
            StopDisposition::Stopped => s::control_flow_stopped(),
            StopDisposition::AlreadyIdle => s::control_flow_stop_already_idle(),
        }),
        ControlResult::Brief(brief) => plain(s::control_brief(
            brief.working,
            brief.awaiting_permission,
            brief.error,
            brief.total,
        )),
        ControlResult::LaneListing { lanes } if lanes.is_empty() => {
            plain(s::control_lane_listing_empty())
        }
        ControlResult::LaneListing { lanes } => plain(
            lanes
                .iter()
                .map(|l| {
                    s::control_lane_listing_row(&s::control_lane_path(&l.project, &l.name), l.chats)
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        ControlResult::LaneCreated { .. } => plain(s::control_lane_created()),
        ControlResult::ChatCreated { .. } => plain(s::control_chat_created()),
        // The body is the agent's own markdown, already bounded at the source.
        // Rendered as-is rather than wrapped in copy: a person who asked what
        // an agent said wants its words, not a sentence about them.
        //
        // That bound is in *chars* and Telegram's limit is in bytes, so a
        // 4000-char CJK reply would still be refused. Unreachable today — no
        // text command parses to a read or an ask, and these two arms exist
        // because the match is exhaustive.
        ControlResult::Transcript { text, .. } => match text {
            Some(text) => plain(text.clone()),
            None => plain(s::control_transcript_empty()),
        },
        // The agent's own words when there are any, bounded at the source —
        // a person who asked wants the reply, not a sentence about it.
        ControlResult::Answer { answer, .. } => plain(match answer {
            PaneAnswer::Text { text } => text.clone(),
            PaneAnswer::NoAnswer => s::control_answer_none(),
            PaneAnswer::Failed => s::control_answer_failed(),
            PaneAnswer::Interrupted => s::control_answer_interrupted(),
            PaneAnswer::Queued => s::control_sent_queued(),
            PaneAnswer::StillWorking => s::control_answer_still_working(),
        }),
        ControlResult::Accepted { disposition } => plain(match disposition {
            AskDisposition::Connecting => s::control_ask_accepted_connecting(),
            AskDisposition::Sent => s::control_ask_accepted(),
            AskDisposition::Queued => s::control_sent_queued(),
            AskDisposition::HandledLocally => s::control_sent_handled_locally(),
        }),
    }
}

fn render_error(error: &ControlError) -> String {
    match error {
        ControlError::OrdinalNotFound { ordinal } => s::control_error_ordinal_not_found(*ordinal),
        ControlError::NoTargetSelected => s::control_error_no_target(),
        ControlError::TargetGone => s::control_error_target_gone(),
        ControlError::FlowNotFound { name } => s::control_error_flow_not_found(name),
        ControlError::FlowLocked { .. } => s::control_error_flow_locked(),
        ControlError::FlowRefused { name } => s::control_error_flow_refused(name),
        ControlError::FlowNotStarted { name } => s::control_error_flow_not_started(name),
        ControlError::FlowNeedsInteraction { name } => {
            s::control_error_flow_needs_interaction(name)
        }
        ControlError::NoActiveLane => s::control_error_no_active_lane(),
        ControlError::OrchestratorDisabled => s::control_error_orchestrator_disabled(),
        ControlError::OrchestratorUnresolvable => s::control_error_orchestrator_unresolvable(),
        ControlError::OrchestratorUnavailable => s::control_error_orchestrator_unavailable(),
        ControlError::ApprovalRefused => s::control_error_approval_refused(),
        ControlError::ApprovalTimedOut => s::control_error_approval_timed_out(),
        ControlError::AgentLimitReached => s::control_error_agent_limit_reached(),
        ControlError::QueueFull => s::control_error_queue_full(),
        ControlError::SelfTargetRefused => s::control_error_self_target_refused(),
        ControlError::LaneCreateBusy => s::control_error_lane_create_busy(),
        // The phone gets the localized sentence, not git's words: a person
        // reading a notification is not the caller that has to fix an
        // argument. The detail is in the log and in the MCP result.
        ControlError::LaneCreateFailed { .. } => s::control_error_lane_create_failed(),
        ControlError::LaneNameInvalid => s::control_error_lane_name_invalid(),
        ControlError::ApprovalUnavailable => s::control_error_approval_unavailable(),
        ControlError::ApprovalsPending => s::control_error_approvals_pending(),
    }
}

/// Assemble the listing one row at a time, stopping before the byte budget
/// rather than truncating mid-row: a half-written row would show an ordinal
/// the user could then type at a pane it does not name.
fn render_listing(listing: &Listing, state: &CommandState) -> RenderedReply {
    // The rows come from `state`, not from `listing`: `absorb` recorded them a
    // moment ago, and flattening a second time here would be a second answer to
    // "what is row 3" for the ordinals to disagree with.
    let rows: Vec<(u32, &ListingRow)> = state
        .rows()
        .iter()
        .enumerate()
        .map(|(i, row)| (i as u32 + 1, row))
        .collect();
    if rows.is_empty() {
        return plain(s::control_listing_empty());
    }

    let mut text = s::control_listing_header();
    let budget = REPLY_MAX_BYTES - OVERFLOW_RESERVE_BYTES;
    let mut shown = 0usize;
    for (ordinal, row) in &rows {
        let line = format!("\n{}", row_text(*ordinal, row));
        if text.len() + line.len() > budget {
            break;
        }
        text.push_str(&line);
        shown += 1;
    }

    let omitted = rows.len() - shown + listing.omitted as usize;
    if omitted > 0 {
        text.push('\n');
        text.push_str(&s::control_listing_omitted(omitted as u32));
    }

    let buttons: Vec<(String, String)> = rows
        .iter()
        .take(shown.min(LISTING_BUTTON_MAX))
        .filter_map(|(ordinal, _)| {
            let token = state.listing_token(Ordinal(*ordinal))?;
            Some((s::control_button_label(*ordinal), token))
        })
        .collect();
    let keyboard = (!buttons.is_empty()).then(|| InlineKeyboard {
        rows: buttons
            .chunks(BUTTONS_PER_ROW)
            .map(<[(String, String)]>::to_vec)
            .collect(),
    });

    RenderedReply { text, keyboard }
}

fn row_text(ordinal: u32, row: &ListingRow) -> String {
    s::control_listing_row(
        ordinal,
        &row.name,
        &row.summary.agent_name,
        &detail_of(&row.summary),
        &ago_of(&row.summary),
    )
}

/// The segment a row's dash introduces: what the pane is doing, then what it
/// is doing it to. Empty when it has neither to report — a dash with nothing
/// after it reads as a truncation.
fn detail_of(summary: &ChatSummary) -> String {
    let badge = state_badge(summary);
    let title = summary.title.as_deref().unwrap_or_default();
    let parts: Vec<&str> = [badge.as_str(), title]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return String::new();
    }
    s::control_listing_detail(&parts.join(" "))
}

/// The pane's condition, bracketed. Health wins over activity: a pane that
/// cannot be talked to is not meaningfully idle.
///
/// Sits against the title rather than in a column of its own, because the two
/// are read together — what a run is doing means little without what it is
/// working on. A pane with no session yet draws nothing: after a restore that
/// is most of the list, so a badge there marks nothing out, and the badge
/// earns its place by naming the panes that do have one.
fn state_badge(summary: &ChatSummary) -> String {
    let glyph = match summary.health {
        Health::Error => s::control_state_error(),
        Health::Unavailable => return String::new(),
        Health::Ok => match summary.activity {
            Activity::Idle => s::control_state_idle(),
            Activity::Working => s::control_state_working(),
            Activity::AwaitingPermission => s::control_state_awaiting_permission(),
        },
    };
    s::control_listing_state(&glyph)
}

/// The title's own segment of a row, its separator included. Empty for a
/// session that has not titled itself: a row then ends at what it does know,
/// rather than at a dash with a stand-in after it.
///
/// The title itself already arrives bounded and single-line — `ChatSummary`
/// caps it at construction.
pub(crate) fn title_suffix(summary: &ChatSummary) -> String {
    summary
        .title
        .as_deref()
        .map_or_else(String::new, s::control_listing_detail)
}

/// What to call a pane the current listing no longer holds a row for: its
/// title, or the agent running it. This is the whole of the sentence that
/// confirms a selection, so unlike a row it cannot fall back to nothing.
fn bare_label(summary: &ChatSummary) -> String {
    summary
        .title
        .clone()
        .unwrap_or_else(|| summary.agent_name.clone())
}

/// How long ago this pane last did anything, or nothing at all when the stamp
/// is missing or sits in the future (a clock skew must not render as a span).
fn ago_of(summary: &ChatSummary) -> String {
    let Some(then) = summary.last_activity else {
        return String::new();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let Some(elapsed) = now.checked_sub(then) else {
        return String::new();
    };
    s::control_listing_ago(&s::format_duration_compact(std::time::Duration::from_secs(
        elapsed,
    )))
}

fn origin_label(origin: FlowOriginKind) -> String {
    match origin {
        FlowOriginKind::Repo => s::control_flow_origin_repo(),
        FlowOriginKind::Project => s::control_flow_origin_project(),
        FlowOriginKind::Global => s::control_flow_origin_global(),
    }
}

/// The usage line for the command that was called without its argument.
/// `/stop` and `/flow` both work bare, so they never reach here.
fn usage_for(command: &str) -> String {
    match command {
        "say" => s::control_usage_say(),
        "daruda" => s::control_usage_daruda(),
        _ => s::control_usage_use(),
    }
}

fn plain(text: String) -> RenderedReply {
    RenderedReply {
        text,
        keyboard: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_channel::bridge::PaneRef;
    use crate::telegram::command::tests::{listing_of, pane};
    #[test]
    fn a_long_listing_is_truncated_under_the_telegram_limit() {
        let chats: Vec<PaneRef> = (0..400).map(pane).collect();
        let mut state = CommandState::default();
        let listing = listing_of(&chats);
        state.record_listing(&listing);
        let rendered = render(
            &Ok(ControlResult::Listing(listing)),
            Absorbed::Nothing,
            &state,
        );
        assert!(
            rendered.text.len() <= REPLY_MAX_BYTES,
            "rendered {} bytes",
            rendered.text.len()
        );
        let shown = rendered.text.lines().count() - 2; // header + the omitted line
        assert!(shown > 0 && shown < 400, "some rows shown, not all");
        assert!(
            rendered
                .text
                .contains(&s::control_listing_omitted((400 - shown) as u32)),
            "the omitted count must be visible: {}",
            rendered.text
        );
    }

    #[test]
    fn listing_buttons_are_folded_into_rows_and_capped() {
        let chats: Vec<PaneRef> = (0..40).map(pane).collect();
        let mut state = CommandState::default();
        let listing = listing_of(&chats);
        state.record_listing(&listing);
        let keyboard = render(
            &Ok(ControlResult::Listing(listing)),
            Absorbed::Nothing,
            &state,
        )
        .keyboard
        .expect("buttons for a non-empty listing");
        let total: usize = keyboard.rows.iter().map(Vec::len).sum();
        assert_eq!(total, LISTING_BUTTON_MAX);
        assert!(keyboard.rows.iter().all(|r| r.len() <= BUTTONS_PER_ROW));
    }

    /// A row names the agent running the pane — the one thing that decides
    /// which chat a person wants, and which no other field carries. The glyph
    /// column is drawn only for a pane that has a session: after a restore a
    /// dormant pane is most of the list, so marking it would say nothing
    /// about any one row.
    #[test]
    fn a_row_names_its_agent_and_marks_only_a_pane_with_a_session() {
        use crate::control::result::{LaneGroup, ProjectGroup, WindowGroup};

        let chat =
            |pane_id, agent: &str, name: &str, health, activity, title: Option<&str>| ChatSummary {
                target: PaneRef {
                    workspace: Default::default(),
                    pane: pane_id,
                },
                agent: agent.into(),
                agent_name: name.into(),
                is_active_lane: true,
                activity,
                health,
                unread: false,
                title: title.map(str::to_owned),
                last_activity: None,
            };
        let listing = Listing {
            windows: vec![WindowGroup {
                index: 0,
                projects: vec![ProjectGroup {
                    name: "daruda".into(),
                    lanes: vec![LaneGroup {
                        name: "main".into(),
                        chats: vec![
                            chat(
                                1,
                                "codex-acp",
                                "Codex",
                                Health::Unavailable,
                                Activity::Idle,
                                None,
                            ),
                            chat(
                                2,
                                "claude",
                                "Claude Code",
                                Health::Ok,
                                Activity::Working,
                                None,
                            ),
                            chat(
                                3,
                                "claude",
                                "Claude Code",
                                Health::Ok,
                                Activity::Idle,
                                Some("notihub routing"),
                            ),
                        ],
                    }],
                }],
            }],
            omitted: 0,
        };
        let mut state = CommandState::default();
        state.record_listing(&listing);
        let text = render(
            &Ok(ControlResult::Listing(listing)),
            Absorbed::Nothing,
            &state,
        )
        .text;

        // Every pane here sits in the active worktree, and no row says so —
        // that axis is still reported to an agent reading the JSON, it just
        // no longer earns room on a phone screen. The three rows are the
        // three shapes the dash segment takes: nothing to report, a badge
        // with no title behind it, and both.
        let rows: Vec<&str> = text.lines().skip(1).collect();
        assert_eq!(rows[0], "1. daruda/main (Codex)");
        assert_eq!(
            rows[1],
            format!(
                "2. daruda/main (Claude Code) — [{}]",
                s::control_state_working()
            )
        );
        assert_eq!(
            rows[2],
            format!(
                "3. daruda/main (Claude Code) — [{}] notihub routing",
                s::control_state_idle()
            )
        );
    }

    #[test]
    fn an_empty_listing_says_so_and_offers_no_buttons() {
        let state = CommandState::default();
        let rendered = render(
            &Ok(ControlResult::Listing(listing_of(&[]))),
            Absorbed::Nothing,
            &state,
        );
        assert_eq!(rendered.text, s::control_listing_empty());
        assert!(rendered.keyboard.is_none());
    }
    #[test]
    fn every_ask_disposition_is_worded_differently() {
        let state = CommandState::default();
        let texts: Vec<String> = [
            AskDisposition::Connecting,
            AskDisposition::Sent,
            AskDisposition::Queued,
            AskDisposition::HandledLocally,
        ]
        .into_iter()
        .map(|disposition| {
            let reply = render(
                &Ok(ControlResult::Accepted { disposition }),
                Absorbed::Nothing,
                &state,
            );
            assert!(reply.keyboard.is_none());
            reply.text
        })
        .collect();
        assert!(texts.iter().all(|t| !t.is_empty()));
        assert_eq!(
            texts.iter().collect::<std::collections::HashSet<_>>().len(),
            4,
            "{texts:?}"
        );
    }

    /// Three refusals with three different fixes, so three different
    /// sentences — a shared one would send the user to the wrong place.
    #[test]
    fn each_orchestrator_refusal_says_something_different() {
        let state = CommandState::default();
        let texts: Vec<String> = [
            ControlError::OrchestratorDisabled,
            ControlError::OrchestratorUnresolvable,
            ControlError::OrchestratorUnavailable,
        ]
        .into_iter()
        .map(|e| render(&Err(e), Absorbed::Nothing, &state).text)
        .collect();
        assert!(texts.iter().all(|t| !t.is_empty()));
        assert_eq!(
            texts.iter().collect::<std::collections::HashSet<_>>().len(),
            3,
            "{texts:?}"
        );
    }

    /// `/daruda` with no text names its own usage line, not `/use`'s.
    #[test]
    fn a_bare_daruda_is_answered_with_its_own_usage() {
        let rendered = render_parse_error(&ParseError::MissingArgument { command: "daruda" });
        assert_eq!(
            rendered.text,
            s::control_error_missing_argument(&s::control_usage_daruda())
        );
    }

    #[test]
    fn a_typo_is_answered_with_the_suggestion() {
        let rendered = render_parse_error(&ParseError::Unknown {
            input: "lst".into(),
            suggestion: Some("list"),
        });
        assert_eq!(
            rendered.text,
            s::control_error_unknown_command_did_you_mean("lst", "list")
        );
    }
}
