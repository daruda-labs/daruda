//! Value types for the pane's activity span, settled-turn records and display
//! modes.

/// The pane's busy span, as of the last `reconcile_activity` tick. `Busy`
/// carries the wall-clock start instant so the enum can't represent "busy with
/// no start time" or "idle with one" — the level and the anchor move together
/// on every edge, which is the only way they are ever written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum ActivitySpan {
    #[default]
    Idle,
    Busy {
        started_at: std::time::Instant,
    },
}

impl ActivitySpan {
    /// The edge-detection memory: whether the pane was busy at the last tick.
    pub(crate) fn is_busy(self) -> bool {
        matches!(self, Self::Busy { .. })
    }

    /// How long the current span has been running, or `None` when idle.
    pub(super) fn elapsed(self) -> Option<std::time::Duration> {
        match self {
            Self::Idle => None,
            Self::Busy { started_at } => Some(started_at.elapsed()),
        }
    }
}

/// What one settled turn cost, keyed by the run it belongs to.
///
/// The three values come from three places and none of them survives on its
/// own: the duration lives only while the span is `Busy`, the wall clock has no
/// source at all (the pane measures with a monotonic clock, which cannot name a
/// time of day), and the token count arrives on the turn's reply. They are
/// recorded together at the busy→idle settle edge — the one point that answers
/// "when did this finish" for everything else in the pane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TurnRecord {
    pub(crate) worked_for: std::time::Duration,
    pub(crate) finished_at: chrono::DateTime<chrono::Local>,
    /// `None` for an agent that reports no usage.
    pub(crate) output_tokens: Option<u64>,
}

/// Terminal outcome of an activity span, captured when the turn/session ends
/// but fired (notification + backing-task done) only when the pane actually
/// settles busy→idle (which may trail `end_turn` while subagents finish).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnOutcome {
    Completed,
    Errored,
    Stopped,
}

/// The pane's derived activity, the single source consumed by the working
/// indicator, the lane badge, and the status-pulse gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityState {
    Idle,
    Working,
    AwaitingPermission,
}

/// How a tool-group summary names each category: `Compact` is the icon and
/// its count, `Labeled` adds the category's words ("3 commands").
///
/// No `Default`, for the reason [`ChatContentWidth`] has none: the starting
/// value is `agent.tool_summary_labels`'s answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolSummaryStyle {
    Compact,
    Labeled,
}

impl ToolSummaryStyle {
    pub(crate) fn from_config(tool_summary_labels: bool) -> Self {
        if tool_summary_labels {
            Self::Labeled
        } else {
            Self::Compact
        }
    }

    /// One segment's text: the bare count, or `labeled`'s words for it.
    pub(crate) fn segment(self, count: usize, labeled: impl FnOnce(usize) -> String) -> String {
        match self {
            Self::Compact => count.to_string(),
            Self::Labeled => labeled(count),
        }
    }
}

/// AgentChat conversation content-column width mode. `Reading` constrains each
/// row to the configured reading width; `Full` spends the whole pane.
///
/// Deliberately no `Default`: which one a pane starts on is `agent
/// .use_reading_width`'s answer, and a type-level default would be a second
/// place to state it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatContentWidth {
    Full,
    Reading,
}

impl ChatContentWidth {
    pub(crate) fn from_config(use_reading_width: bool) -> Self {
        if use_reading_width {
            Self::Reading
        } else {
            Self::Full
        }
    }

    pub(crate) fn is_reading(self) -> bool {
        matches!(self, Self::Reading)
    }

    pub(crate) fn toggle(self) -> Self {
        match self {
            Self::Full => Self::Reading,
            Self::Reading => Self::Full,
        }
    }
}
