//! Append-only, event-sourced run log. A domain [`Machine`] proposes events;
//! [`RunLog`] checks each one, makes it durable, and only then applies it,
//! so in-memory state never runs ahead of disk and a crash at any point
//! reopens to a committed prefix. GPUI-free; the caller owns paths and the
//! single-writer lock.

pub mod backend;
pub mod committer;
pub mod error;
pub mod log;
pub mod machine;
pub mod record;
mod replay;
mod snapshot;

#[cfg(feature = "test-support")]
pub mod testing;

pub use backend::{FileBackend, Generation, LogBackend};
pub use committer::{AfterCommit, Committer};
pub use error::{CommitError, FlushTimedOut, OpenError, SubscribeError};
pub use log::{CatchUp, Committed, RunLog};
pub use machine::Machine;
pub use record::{Body, FORMAT_VERSION, Record};
