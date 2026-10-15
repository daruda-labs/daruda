//! Test support for every crate that keeps a [`Machine`](crate::Machine) in
//! a [`RunLog`](crate::RunLog): a fault-injecting in-memory backend, the
//! laws a machine must obey, and an exhaustive crash enumeration.

mod crash;
mod fault;
mod laws;

pub use crash::{CrashCase, CrashSummary, enumerate_crashes, enumerate_crashes_with};
pub use fault::{DirOp, Fault, FaultBackend};
pub use laws::assert_machine_laws;

/// The build string the harness opens logs with.
pub const HARNESS_BUILD: &str = "daruda_runlog-harness";

/// States and events compare by their serialized form, the same bytes a
/// snapshot or record holds, so machines need not implement `PartialEq`.
fn json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("machine values serialize")
}

#[cfg(test)]
mod tests {
    #[test]
    fn json_is_the_compact_form() {
        assert_eq!(super::json(&vec![1, 2]), "[1,2]");
    }
}
