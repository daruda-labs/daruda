//! Channel-independent reply routing, target selection, and callback tokens.

use super::{
    BridgePing, InboundAction, InlineKeyboard, PaneRef, PermissionDecision, PreparedPing, Routed,
    Unaimed,
};
use std::collections::{HashMap, VecDeque};
use uuid::Uuid;

/// Upper bound on `(message_id -> PaneRef)` entries in `sent_pings`.
/// Oldest entries are evicted first; a reply-to lookup for an evicted
/// message_id falls back to `last_pinged`.
pub(crate) const SENT_PINGS_CAP: usize = 64;

/// Upper bound on outstanding permission-callback tokens in
/// `pending_permissions`. Tokens whose permission is resolved in-app
/// (never tapped on the phone) are never consumed and would otherwise
/// accumulate forever; oldest are evicted first, and an evicted token
/// routes to `Ignore` on a later tap like any unknown one.
pub(crate) const PENDING_PERMISSIONS_CAP: usize = 64;

/// Callback-data prefix for an approval button. Distinct from the listing
/// prefix and from a permission token (bare `Uuid::simple`, no `:`), so the
/// three namespaces cannot collide structurally.
pub(crate) const APPROVAL_TOKEN_PREFIX: &str = "apv";

/// Approval tokens kept before the oldest is evicted. Two per card, and a
/// card the user never answers times out on the app side — so this only
/// bounds a pathological run of unanswered ones.
const PENDING_APPROVALS_CAP: usize = 64;

/// One recipient's routing state. Native message IDs remain lossless.
pub struct RoutingCore<M = i64> {
    pub(crate) last_pinged: Option<PaneRef>,
    pub(crate) sent_pings: HashMap<M, PaneRef>,
    pub(crate) sent_pings_order: VecDeque<M>,
    pub(crate) pending_permissions: HashMap<String, (PaneRef, u64, PermissionDecision)>,
    pub(crate) pending_permissions_order: VecDeque<String>,
    /// Approval tokens. Deliberately *not* consumed on a tap, unlike
    /// `pending_permissions`: the card stays on screen, and tapping the same
    /// button twice is ordinary use rather than a second decision. The
    /// approval store discards the redundant answer.
    pending_approvals: HashMap<
        String,
        (
            crate::control::approval::ApprovalId,
            crate::control::approval::ApprovalChoice,
        ),
    >,
    pending_approvals_order: VecDeque<String>,
    command_state: crate::remote_channel::command::CommandState,
}

impl<M: Eq + std::hash::Hash + Clone> Default for RoutingCore<M> {
    fn default() -> Self {
        Self {
            last_pinged: None,
            sent_pings: HashMap::new(),
            sent_pings_order: VecDeque::new(),
            pending_permissions: HashMap::new(),
            pending_permissions_order: VecDeque::new(),
            pending_approvals: HashMap::new(),
            pending_approvals_order: VecDeque::new(),
            command_state: crate::remote_channel::command::CommandState::default(),
        }
    }
}

impl<M: Eq + std::hash::Hash + Clone> RoutingCore<M> {
    pub fn command_state_mut(&mut self) -> &mut crate::remote_channel::command::CommandState {
        &mut self.command_state
    }

    /// Called only after the adapter authenticates both sender and conversation.
    pub fn route_text(&mut self, text: String, reply_to_message_id: Option<M>) -> Routed {
        // Parsing sits behind the gate on purpose: a listing names projects,
        // lanes, and session titles, so an unauthorized chat must not reach it.
        // A name we do not own is held back rather than answered: the agent's
        // slash namespace is open and ours is closed, so only the pane can say
        // whether `/usage` is its command or a typo of `/use`.
        let unknown = match crate::control::spec::parse(&text) {
            Ok(command) => return Routed::Ready(InboundAction::RunCommand { command }),
            Err(crate::control::spec::ParseError::NotACommand) => None,
            Err(crate::control::spec::ParseError::Unknown { input, suggestion }) => {
                Some((input, suggestion))
            }
            // A command we *do* own, used wrongly. Ours to answer.
            Err(error) => return Routed::Ready(InboundAction::ReportParseError { error }),
        };

        let reply_to = reply_to_message_id
            .and_then(|id| self.sent_pings.get(&id))
            .copied();

        match self
            .command_state
            .plain_text_target(reply_to, self.last_pinged)
        {
            Some(pane) => Routed::Ready(match unknown {
                Some((name, suggestion)) => InboundAction::UnknownSlash {
                    pane,
                    name,
                    text,
                    suggestion,
                },
                None => InboundAction::InjectPrompt { pane, text },
            }),
            // Nothing here names a target, and this layer cannot ask the app
            // for one. Handed on rather than answered — both the target and,
            // for a slash, whose command it is are still open questions.
            None => Routed::NeedsTarget(match unknown {
                Some((name, suggestion)) => Unaimed::Slash {
                    name,
                    text,
                    suggestion,
                },
                None => Unaimed::Text { text },
            }),
        }
    }

    pub fn route_callback(&mut self, data: String) -> InboundAction {
        // Listing tokens are tried first and are never consumed — tapping the
        // same row twice is ordinary use. Permission tokens below are.
        if data.starts_with(crate::remote_channel::command::LISTING_TOKEN_PREFIX) {
            return match self.command_state.resolve_token(&data) {
                Some(pane) => InboundAction::SelectTarget { pane },
                None => InboundAction::StaleListing,
            };
        }

        if data.starts_with(APPROVAL_TOKEN_PREFIX) {
            return match self.pending_approvals.get(&data) {
                Some((id, choice)) => InboundAction::ResolveApproval {
                    id: *id,
                    choice: *choice,
                },
                None => InboundAction::Ignore,
            };
        }

        match self.pending_permissions.remove(&data) {
            Some((pane, perm_id, decision)) => InboundAction::RespondPermission {
                pane,
                perm_id,
                decision,
            },
            None => InboundAction::Ignore,
        }
    }

    /// Mint the two callback tokens one approval card needs and remember what
    /// each means. Returns them in `(approve, refuse)` order.
    pub fn record_pending_approval(
        &mut self,
        id: crate::control::approval::ApprovalId,
    ) -> (String, String) {
        use crate::control::approval::ApprovalChoice;
        let approve = self.insert_pending_approval(id, ApprovalChoice::Approved);
        let refuse = self.insert_pending_approval(id, ApprovalChoice::Refused);
        (approve, refuse)
    }

    #[cfg(test)]
    pub fn pending_approval_token_count(&self) -> usize {
        self.pending_approvals.len()
    }

    /// Drop both of `id`'s tokens. Called when the request settles, so the
    /// bounded table holds only cards that can still be answered.
    pub fn forget_pending_approval(&mut self, id: crate::control::approval::ApprovalId) {
        self.pending_approvals.retain(|_, (held, _)| *held != id);
        self.pending_approvals_order
            .retain(|token| self.pending_approvals.contains_key(token));
    }

    fn insert_pending_approval(
        &mut self,
        id: crate::control::approval::ApprovalId,
        choice: crate::control::approval::ApprovalChoice,
    ) -> String {
        // Random rather than derived from `id`: a card survives a restart in
        // the user's chat history while this table does not, and a derived
        // token would let a stale button resolve a *new* request that happens
        // to reuse the number.
        let token = format!(
            "{APPROVAL_TOKEN_PREFIX}:{}",
            uuid::Uuid::new_v4().as_simple()
        );
        self.pending_approvals.insert(token.clone(), (id, choice));
        self.pending_approvals_order.push_back(token.clone());
        if self.pending_approvals_order.len() > PENDING_APPROVALS_CAP
            && let Some(oldest) = self.pending_approvals_order.pop_front()
        {
            self.pending_approvals.remove(&oldest);
        }
        token
    }

    pub fn build_ping(&mut self, ping: BridgePing) -> PreparedPing {
        let keyboard = ping.permission.map(|prompt| {
            let buttons: Vec<(String, String)> = prompt
                .buttons
                .into_iter()
                .map(|(label, decision)| {
                    let token = Uuid::new_v4().simple().to_string();
                    self.insert_pending_permission(
                        token.clone(),
                        (ping.pane, prompt.perm_id, decision),
                    );
                    (label, token)
                })
                .collect();

            InlineKeyboard::single_row(buttons)
        });

        PreparedPing {
            header: ping.header,
            tail: ping.tail,
            keyboard,
        }
    }

    /// Registers one permission-callback token, enforcing the
    /// `PENDING_PERMISSIONS_CAP` bound by evicting the oldest token
    /// (from both the map and its companion order queue) once
    /// exceeded — mirrors `record_sent`'s eviction shape. `build_ping`
    /// calls this once per button in the permission-wait ping's
    /// `PermissionPromptRef::buttons`.
    fn insert_pending_permission(
        &mut self,
        token: String,
        value: (PaneRef, u64, PermissionDecision),
    ) {
        self.pending_permissions.insert(token.clone(), value);
        self.pending_permissions_order.push_back(token);

        if self.pending_permissions_order.len() > PENDING_PERMISSIONS_CAP
            && let Some(oldest) = self.pending_permissions_order.pop_front()
        {
            self.pending_permissions.remove(&oldest);
        }
    }

    pub fn record_sent(&mut self, message_id: M, pane: PaneRef) {
        self.last_pinged = Some(pane);
        self.sent_pings.insert(message_id.clone(), pane);
        self.sent_pings_order.push_back(message_id);

        if self.sent_pings_order.len() > SENT_PINGS_CAP
            && let Some(oldest) = self.sent_pings_order.pop_front()
        {
            self.sent_pings.remove(&oldest);
        }
    }
}

#[cfg(test)]
mod tests;
