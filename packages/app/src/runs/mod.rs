//! Process-wide owners of run state kept in run logs.
//!
//! [`RunsHost`] owns the task log: every task write in the app is a
//! [`daruda_store::tasks::TaskEvent`] proposed here, and [`GlobalTasks`] is
//! the read-only projection every window renders from. Window effects (open
//! a lane, deliver text, find a pane) stay with the Workspace that asks.

mod cli;
mod delivery;
mod effects;
mod inbox;
mod intent;
#[cfg(any(test, feature = "screenshot"))]
pub(crate) mod seed;
mod tasks;

pub(crate) use cli::sessions_dir as cli_sessions_dir;
pub(crate) use delivery::{DeliveryFailure, RunPane, TerminalDispatchError};
pub(crate) use effects::RunsEffects;
pub(crate) use intent::TaskIntent;
pub(crate) use tasks::{GlobalTasks, RunsHost, TaskCommitError, no_hook};

#[cfg(test)]
mod tests;
