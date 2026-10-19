//! Process-wide owners of run state kept in run logs.
//!
//! [`RunsHost`] owns the task log: every task write in the app is a
//! [`daruda_store::tasks::TaskEvent`] proposed here, and [`GlobalTasks`] is
//! the read-only projection every window renders from. It also holds the
//! flow runs in flight ([`flows`]), so no window owns one. Window effects
//! (open a lane, deliver text, find a pane, raise a question) stay with the
//! Workspaces, installed as [`RunsEffects`].

mod cli;
mod delivery;
mod effects;
mod flows;
mod inbox;
mod intent;
#[cfg(any(test, feature = "screenshot"))]
pub(crate) mod seed;
mod tasks;

pub(crate) use cli::sessions_dir as cli_sessions_dir;
pub(crate) use delivery::{DeliveryFailure, RunPane, TerminalDispatchError};
pub(crate) use effects::RunsEffects;
pub(crate) use flows::{
    AlreadyRunning, AnswerRefused, FlowChange, FlowChanged, FlowEnded, FlowRun, FlowSource,
    FlowStart, Lull, NodeRunState, ParkedAsk, RunColouring, RunView,
};
#[cfg(test)]
pub(crate) use flows::{NodeRunStates, apply_run_event};
pub(crate) use intent::TaskIntent;
pub(crate) use tasks::{GlobalTasks, ReadOnlyCause, RunsHost, TaskCommitError, no_hook};

#[cfg(test)]
mod tests;
