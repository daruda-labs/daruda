//! Process-lifetime query scope, delivery ledger and retained execution results.
mod delivery;
mod observation;
mod state;
pub(crate) use delivery::{WaitAnswer, WaitId, WaitOutcome, WaiterBusy};
pub(crate) use observation::LocalEnd;
pub(crate) use state::RunEvent;
pub(super) use state::RunWatch;
#[cfg(test)]
mod tests;
