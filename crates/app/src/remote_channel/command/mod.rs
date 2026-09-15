//! The Telegram adapter's half of the control surface: the conversation state
//! the core deliberately does not hold, plus the rendering of a result.
//!
//! An ordinal only means something against the listing that produced it, and a
//! selected target only means something for this chat. Both belong here, next
//! to `BridgeCore`'s existing `sent_pings` / `last_pinged` table.
//!
//! This file owns that state and the resolution of a parsed command against
//! it; turning an outcome into a message a phone shows is [`render`]'s.

use crate::control::result::{ChatSummary, ControlError, ControlOutcome, ControlResult, Listing};
use crate::control::spec::{
    ControlCommand, FlowCommand, Ordinal, ResolvedCommand, ResolvedFlowCommand, UseTarget,
};
use crate::remote_channel::bridge::PaneRef;
use crate::surface::strings as s;

/// Callback-data prefix for a listing button. Distinct from the permission
/// prompt's tokens, which are `Uuid::simple` (32 hex chars, no `:`), so the
/// two namespaces cannot collide structurally. `pub(crate)` because the router
/// needs it to tell "this listing is stale" from "this token is unknown".
pub(crate) const LISTING_TOKEN_PREFIX: &str = "lst";

/// One row of a listing, flattened out of its window/project/lane grouping.
/// The adapter keeps the row rather than just its `PaneRef` so `/use 2` can
/// answer with the name the user saw on line 2.
#[derive(Clone, Debug)]
pub(crate) struct ListingRow {
    pub(crate) name: String,
    pub(crate) summary: ChatSummary,
}

/// Depth-first walk in listing order — the same order `record_listing`
/// flattens, so an ordinal here and an ordinal there name the same pane.
fn flatten(listing: &Listing) -> Vec<ListingRow> {
    listing
        .windows
        .iter()
        .flat_map(|w| w.projects.iter())
        .flat_map(|p| {
            p.lanes.iter().flat_map(move |l| {
                l.chats.iter().map(move |c| ListingRow {
                    name: s::control_lane_path(&p.name, &l.name),
                    summary: c.clone(),
                })
            })
        })
        .collect()
}

/// Ordinals and the current target, for one authorized chat.
#[derive(Debug)]
pub(crate) struct CommandState {
    /// Index `i` holds the row `/use {i + 1}` names. Replaced wholesale by
    /// the next `/list`; never re-derived, so an ordinal keeps pointing at the
    /// pane the user actually saw.
    rows: Vec<ListingRow>,
    /// Bumped by every listing, so a button on a superseded one resolves to
    /// nothing instead of to whatever now sits at that index.
    ///
    /// Seeded at random rather than from zero, because a listing message
    /// survives a daruda restart in the user's chat history while this table
    /// does not. Counting from zero again would make a button on a
    /// pre-restart listing resolve against the *new* rows — and since the
    /// label is only the ordinal, nothing on screen would say the two
    /// listings differ.
    generation: u64,
    selected: Option<PaneRef>,
}

impl Default for CommandState {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            generation: uuid::Uuid::new_v4().as_u128() as u64,
            selected: None,
        }
    }
}

impl CommandState {
    pub(crate) fn record_listing(&mut self, listing: &Listing) {
        self.rows = flatten(listing);
        self.generation = self.generation.wrapping_add(1);
    }

    pub(crate) fn resolve(&self, ordinal: Ordinal) -> Result<PaneRef, ControlError> {
        self.row_at(ordinal)
            .map(|row| row.summary.target)
            .ok_or(ControlError::OrdinalNotFound { ordinal: ordinal.0 })
    }

    /// How a pane is named on screen: the row it last appeared on. `None`
    /// once a newer listing has dropped it.
    fn label_for(&self, target: PaneRef) -> Option<String> {
        let row = self.rows.iter().find(|r| r.summary.target == target)?;
        Some(s::control_target_label(
            &row.name,
            &render::title_of(&row.summary),
        ))
    }

    /// The rows of the listing this table was last filled from, in ordinal
    /// order. What the renderer draws, so a row and its ordinal cannot drift.
    pub(crate) fn rows(&self) -> &[ListingRow] {
        &self.rows
    }

    /// The listed summary for `target`, if the current listing still holds it.
    /// A tapped button always does; a stale one resolved to nothing earlier.
    pub(crate) fn summary_for(&self, target: PaneRef) -> Option<ChatSummary> {
        self.rows
            .iter()
            .find(|r| r.summary.target == target)
            .map(|r| r.summary.clone())
    }

    fn row_at(&self, ordinal: Ordinal) -> Option<&ListingRow> {
        self.rows.get(self.index_of(ordinal)?)
    }

    pub(crate) fn select(&mut self, target: Option<PaneRef>) {
        self.selected = target;
    }

    pub(crate) fn selected(&self) -> Option<PaneRef> {
        self.selected
    }

    /// Drop `target` from the selection. Called when a command reports the
    /// pane is gone, so the next plain message is not answered with the same
    /// error forever.
    pub(crate) fn forget(&mut self, target: PaneRef) {
        if self.selected == Some(target) {
            self.selected = None;
        }
    }

    /// Where a message with no command goes. An explicit act beats an implicit
    /// one: replying names a pane on purpose, selecting names one on purpose,
    /// and `last_pinged` is only whoever spoke last.
    pub(crate) fn plain_text_target(
        &self,
        reply_to: Option<PaneRef>,
        last_pinged: Option<PaneRef>,
    ) -> Option<PaneRef> {
        reply_to.or(self.selected).or(last_pinged)
    }

    /// Callback data for the button at `ordinal`.
    pub(crate) fn listing_token(&self, ordinal: Ordinal) -> Option<String> {
        let index = self.index_of(ordinal)?;
        self.rows.get(index)?;
        Some(format!(
            "{LISTING_TOKEN_PREFIX}:{}:{index}",
            self.generation
        ))
    }

    /// Resolve a listing button tap. Non-consuming — tapping the same row
    /// twice is ordinary use, not an error.
    pub(crate) fn resolve_token(&self, token: &str) -> Option<PaneRef> {
        let mut parts = token.split(':');
        if parts.next()? != LISTING_TOKEN_PREFIX {
            return None;
        }
        let generation: u64 = parts.next()?.parse().ok()?;
        if generation != self.generation {
            return None;
        }
        let index: usize = parts.next()?.parse().ok()?;
        self.rows.get(index).map(|row| row.summary.target)
    }

    /// Zero-based position of a one-based ordinal. `/use 0` has no row.
    fn index_of(&self, ordinal: Ordinal) -> Option<usize> {
        usize::try_from(ordinal.0).ok()?.checked_sub(1)
    }
}

mod render;

pub(crate) use render::{RenderedReply, render, render_parse_error};

/// What the poll loop should do with a parsed command.
pub(crate) enum Resolution {
    /// Hand this to `control::exec::run`. The `PaneRef` it addresses, if any,
    /// travels alongside so a `TargetGone` can drop the stale selection.
    Run(ResolvedCommand, Option<PaneRef>),
    /// Already answered here — `/use` only moves adapter state, and a failed
    /// ordinal never reaches the executor.
    Answer(ControlOutcome),
    /// `/daruda` — the destination is the orchestrator's to name, and naming
    /// it may have to start one. That needs the `App` this step does not
    /// have, so the caller finishes the command.
    Ask(String),
    /// `/flow <name>` — a person naming a flow has not named a worktree, and
    /// finding one that holds it means walking every window. Same reason as
    /// [`Self::Ask`]: the target is real, just not resolvable from the
    /// ordinal table alone.
    RunFlow(String),
}

/// Turn ordinals into concrete panes, and handle the one command that is
/// purely adapter state.
pub(crate) fn resolve_command(command: ControlCommand, state: &mut CommandState) -> Resolution {
    match command {
        ControlCommand::List => Resolution::Run(ResolvedCommand::List, None),
        ControlCommand::Brief => Resolution::Run(ResolvedCommand::Brief, None),
        // Listing needs no target and each row says which worktree it came
        // from; running needs one, and only the `App` can pick it.
        ControlCommand::Flow(FlowCommand::List) => {
            Resolution::Run(ResolvedCommand::Flow(ResolvedFlowCommand::List), None)
        }
        ControlCommand::Flow(FlowCommand::Run { name }) => Resolution::RunFlow(name),
        ControlCommand::Ask { text } => Resolution::Ask(text),
        ControlCommand::Use(UseTarget::Clear) => {
            state.select(None);
            Resolution::Answer(Ok(ControlResult::Selected { target: None }))
        }
        ControlCommand::Use(UseTarget::Select(ordinal)) => {
            let Some(summary) = state.row_at(ordinal).map(|row| row.summary.clone()) else {
                return Resolution::Answer(Err(ControlError::OrdinalNotFound {
                    ordinal: ordinal.0,
                }));
            };
            state.select(Some(summary.target));
            Resolution::Answer(Ok(ControlResult::Selected {
                target: Some(summary),
            }))
        }
        ControlCommand::Say { target, text } => match state.resolve(target) {
            Ok(target) => Resolution::Run(ResolvedCommand::Say { target, text }, Some(target)),
            Err(e) => Resolution::Answer(Err(e)),
        },
        // A bare `/stop` means the current target — the whole reason `/use`
        // exists is not having to repeat the number.
        ControlCommand::Stop { target: None } => match state.selected() {
            Some(target) => Resolution::Run(ResolvedCommand::Stop { target }, Some(target)),
            None => Resolution::Answer(Err(ControlError::NoTargetSelected)),
        },
        ControlCommand::Stop {
            target: Some(ordinal),
        } => match state.resolve(ordinal) {
            Ok(target) => Resolution::Run(ResolvedCommand::Stop { target }, Some(target)),
            Err(e) => Resolution::Answer(Err(e)),
        },
    }
}

/// Fold an outcome back into adapter state before it is rendered: a listing
/// becomes the new ordinal table, and a pane that reported itself gone stops
/// being the target so the next plain message is not answered with the same
/// error forever.
pub(crate) fn absorb(
    outcome: &ControlOutcome,
    addressed: Option<PaneRef>,
    state: &mut CommandState,
) -> Absorbed {
    match outcome {
        Ok(ControlResult::Listing(listing)) => {
            state.record_listing(listing);
            Absorbed::Nothing
        }
        Err(ControlError::TargetGone) => match addressed {
            Some(target) if state.selected() == Some(target) => {
                state.forget(target);
                Absorbed::SelectionDropped
            }
            _ => Absorbed::Nothing,
        },
        _ => Absorbed::Nothing,
    }
}

/// A state change worth telling the user about, on top of the outcome itself.
///
/// Only one so far, and it earns the type: clearing the target silently would
/// send their *next* plain message somewhere else with no warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Absorbed {
    Nothing,
    SelectionDropped,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::control::result::{Activity, Health, LaneGroup, ProjectGroup, WindowGroup};
    use daruda_store::project::WorkspaceUuid;

    pub(crate) fn pane(n: u64) -> PaneRef {
        PaneRef {
            workspace: WorkspaceUuid::new(),
            pane: n,
        }
    }

    fn summary(target: PaneRef) -> ChatSummary {
        ChatSummary {
            target,
            is_active_lane: false,
            activity: Activity::Idle,
            health: Health::Ok,
            unread: false,
            title: None,
            last_activity: None,
        }
    }

    pub(crate) fn listing_of(panes: &[PaneRef]) -> Listing {
        Listing {
            windows: vec![WindowGroup {
                index: 0,
                projects: vec![ProjectGroup {
                    name: "p".into(),
                    lanes: vec![LaneGroup {
                        name: "l".into(),
                        chats: panes.iter().map(|p| summary(*p)).collect(),
                    }],
                }],
            }],
            omitted: 0,
        }
    }
    #[test]
    fn ordinals_follow_listing_order_starting_at_one() {
        let (a, b) = (pane(10), pane(20));
        let mut state = CommandState::default();
        state.record_listing(&listing_of(&[a, b]));
        assert_eq!(state.resolve(Ordinal(1)), Ok(a));
        assert_eq!(state.resolve(Ordinal(2)), Ok(b));
        assert_eq!(
            state.resolve(Ordinal(3)),
            Err(ControlError::OrdinalNotFound { ordinal: 3 })
        );
        assert_eq!(
            state.resolve(Ordinal(0)),
            Err(ControlError::OrdinalNotFound { ordinal: 0 })
        );
    }

    #[test]
    fn a_new_listing_replaces_the_previous_generation() {
        let (a, b) = (pane(10), pane(20));
        let mut state = CommandState::default();
        state.record_listing(&listing_of(&[a, b]));
        state.record_listing(&listing_of(&[b]));
        assert_eq!(state.resolve(Ordinal(1)), Ok(b));
        assert_eq!(
            state.resolve(Ordinal(2)),
            Err(ControlError::OrdinalNotFound { ordinal: 2 })
        );
    }

    #[test]
    fn reply_to_beats_selection_and_last_pinged() {
        let (a, b, c) = (pane(10), pane(20), pane(30));
        let mut state = CommandState::default();
        state.select(Some(b));
        assert_eq!(state.plain_text_target(Some(a), Some(c)), Some(a));
    }

    #[test]
    fn selection_beats_last_pinged() {
        let (b, c) = (pane(20), pane(30));
        let mut state = CommandState::default();
        state.select(Some(b));
        assert_eq!(state.plain_text_target(None, Some(c)), Some(b));
    }

    #[test]
    fn clearing_the_selection_falls_back_to_last_pinged() {
        let (b, c) = (pane(20), pane(30));
        let mut state = CommandState::default();
        state.select(Some(b));
        state.select(None);
        assert_eq!(state.plain_text_target(None, Some(c)), Some(c));
    }

    #[test]
    fn a_vanished_selection_is_dropped() {
        let b = pane(20);
        let mut state = CommandState::default();
        state.select(Some(b));
        state.forget(b);
        assert_eq!(state.selected(), None);
        assert_eq!(state.plain_text_target(None, None), None);
    }

    #[test]
    fn a_list_button_can_be_tapped_more_than_once() {
        let a = pane(10);
        let mut state = CommandState::default();
        state.record_listing(&listing_of(&[a]));
        let token = state.listing_token(Ordinal(1)).expect("token");
        assert_eq!(state.resolve_token(&token), Some(a));
        assert_eq!(
            state.resolve_token(&token),
            Some(a),
            "tokens are not consumed"
        );
    }

    #[test]
    fn a_previous_generation_token_stops_resolving() {
        let (a, b) = (pane(10), pane(20));
        let mut state = CommandState::default();
        state.record_listing(&listing_of(&[a]));
        let stale = state.listing_token(Ordinal(1)).expect("token");
        state.record_listing(&listing_of(&[b]));
        assert_eq!(state.resolve_token(&stale), None);
    }
    #[test]
    fn use_selects_without_reaching_the_executor() {
        let a = pane(10);
        let mut state = CommandState::default();
        state.record_listing(&listing_of(&[a]));
        let resolution = resolve_command(
            ControlCommand::Use(UseTarget::Select(Ordinal(1))),
            &mut state,
        );
        let Resolution::Answer(Ok(ControlResult::Selected { target: Some(s) })) = resolution else {
            panic!("/use answers from adapter state alone");
        };
        assert_eq!(s.target, a);
        assert_eq!(state.selected(), Some(a));
    }

    #[test]
    fn a_bare_stop_uses_the_current_target() {
        let a = pane(10);
        let mut state = CommandState::default();
        state.record_listing(&listing_of(&[a]));
        state.select(Some(a));
        let Resolution::Run(ResolvedCommand::Stop { target }, addressed) =
            resolve_command(ControlCommand::Stop { target: None }, &mut state)
        else {
            panic!("a selected target makes a bare /stop concrete");
        };
        assert_eq!(target, a);
        assert_eq!(addressed, Some(a));
    }

    #[test]
    fn a_bare_stop_with_no_target_is_refused_before_the_executor() {
        let mut state = CommandState::default();
        assert!(matches!(
            resolve_command(ControlCommand::Stop { target: None }, &mut state),
            Resolution::Answer(Err(ControlError::NoTargetSelected))
        ));
    }

    /// `/daruda` has no ordinal, so this step leaves the command unfinished:
    /// naming the orchestrator can mean starting it, which needs an `App`.
    #[test]
    fn daruda_leaves_its_destination_to_the_caller() {
        let mut state = CommandState::default();
        let Resolution::Ask(text) = resolve_command(
            ControlCommand::Ask {
                text: "make me a pane".into(),
            },
            &mut state,
        ) else {
            panic!("/daruda resolves to an Ask");
        };
        assert_eq!(text, "make me a pane");
    }

    #[test]
    fn a_gone_target_stops_being_the_selection() {
        let a = pane(10);
        let mut state = CommandState::default();
        state.record_listing(&listing_of(&[a]));
        state.select(Some(a));
        absorb(&Err(ControlError::TargetGone), Some(a), &mut state);
        assert_eq!(state.selected(), None);
    }

    #[test]
    fn a_listing_outcome_becomes_the_new_ordinal_table() {
        let (a, b) = (pane(10), pane(20));
        let mut state = CommandState::default();
        absorb(
            &Ok(ControlResult::Listing(listing_of(&[a, b]))),
            None,
            &mut state,
        );
        assert_eq!(state.resolve(Ordinal(2)), Ok(b));
    }

    /// A listing message outlives daruda in the user's chat history, but this
    /// table does not. If the generation counted from zero again, a button on
    /// a pre-restart listing would resolve against the new rows — and the
    /// label is only the ordinal, so nothing on screen would say they differ.
    #[test]
    fn a_button_from_a_previous_process_does_not_resolve() {
        let (a, b) = (pane(10), pane(20));
        let mut before = CommandState::default();
        before.record_listing(&listing_of(&[a]));
        let stale = before.listing_token(Ordinal(1)).expect("token");

        let mut after = CommandState::default();
        after.record_listing(&listing_of(&[b]));
        assert_eq!(
            after.resolve_token(&stale),
            None,
            "a token minted by another process must not select row 1 here"
        );
    }

    #[test]
    fn a_permission_token_is_not_mistaken_for_a_listing_one() {
        let a = pane(10);
        let mut state = CommandState::default();
        state.record_listing(&listing_of(&[a]));
        assert_eq!(state.resolve_token("2f6c1b9e4a5d"), None);
    }
}
