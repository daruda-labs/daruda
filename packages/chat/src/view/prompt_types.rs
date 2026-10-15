//! Prompt identity, origin and the outcomes of the send / Escape / empty-submit
//! gestures.

/// Stable identity of a queued prompt, minted per pane. Lets the strip target
/// an entry for removal independent of its position, which shifts as earlier
/// entries drain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PromptId(pub(super) u64);

impl std::fmt::Display for PromptId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Where a prompt came from — the bottom-dock composer, or a phone-relayed
/// Telegram reply. Only the latter arms the Telegram first-response watch on
/// dispatch; an in-app prompt never does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptOrigin {
    InApp,
    Telegram,
}

/// Whether a send put the prompt on the wire immediately or only queued it
/// behind an in-flight turn — the Telegram relay needs this to decide whether
/// a "queued" notice is owed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptDispatch {
    ReadOnly,
    SentNow,
    Queued,
    /// The pane is already holding `QUEUE_DEPTH_MAX` prompts, so this one was
    /// not taken. A named outcome rather than a silent drop: every producer
    /// (an agent's tool call, a phone `/say`, a person typing) has something
    /// to tell whoever sent it.
    QueueFull,
}

/// Which Telegram watch action `apply_event`'s tail owes, decided once per
/// event arm so the three lifecycle methods (clear/check/finish) each have
/// exactly one call site instead of being duplicated at every arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(super) enum PhoneTurnAction {
    #[default]
    None,
    /// A Stop's cancel-ack landed — the turn is over and no relay is owed.
    Clear,
    /// A permission card interrupted the turn. The card itself was relayed
    /// as the phone's first sign of life (before this fold), so the wait is
    /// answered — but the turn is still the phone's, and its completion is
    /// still owed.
    Answer,
    /// New conversation content arrived — check whether it resolves the
    /// armed watch.
    CheckUpdate,
    /// The turn or session settled — resolve the watch to its terminal
    /// effect.
    Finish,
}

/// A prompt submitted while a turn was in flight, held in the queue until it
/// can be dispatched. Not echoed into the transcript — lives only here (and
/// the queued-prompt strip) until drained and echoed as a `UserText`.
#[derive(Debug, Clone)]
pub struct QueuedPrompt {
    pub(crate) id: PromptId,
    pub(crate) text: String,
    pub(crate) attachments: Vec<daruda_acp::PromptAttachment>,
    pub(crate) origin: PromptOrigin,
    /// The task run this prompt delivers, reported to the host when it
    /// reaches the wire or is dropped unsent. Separate from `origin`: a task
    /// prompt is in-app, and the Telegram watch keys on `origin` alone.
    pub(crate) task: Option<daruda_store::tasks::ExecutionRef>,
}

impl QueuedPrompt {
    pub fn id(&self) -> PromptId {
        self.id
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn attachments(&self) -> &[daruda_acp::PromptAttachment] {
        &self.attachments
    }
}

/// What the Escape shortcut resolved to for a focused Agent chat pane. Returned
/// so the `Workspace` shim fires the settle-edge completion only when a turn
/// was actually cancelled.
pub enum EscapeOutcome {
    /// Cancelled an in-flight turn (or trailing background-subagent work) — the
    /// caller must run `reconcile_activity` + `fire_activity_completion`.
    Cancelled,
    /// Discarded a parked queue (the "Esc twice clears the queue" gesture); no
    /// turn was running, so there is no completion to fire.
    ClearedQueue,
    /// Nothing to act on — the caller reports not-handled so Escape propagates.
    Ignored,
}

/// What an empty-composer submit resolved to for a focused Agent chat pane.
/// The `Workspace` shim needs the distinction only for tests today; the strip
/// reads `resume_armed()` instead. Shaped after [`EscapeOutcome`], the gesture
/// on the other side of the same parked queue.
pub enum EmptySubmitOutcome {
    /// First Enter — the strip now asks for a second one before resuming.
    Armed,
    /// Second Enter — the parked queue went back to the front of the live one.
    Resumed,
    /// Nothing parked, so the gesture has nothing to act on.
    Ignored,
}
