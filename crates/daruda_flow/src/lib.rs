//! Declarative flow engine over ACP: a YAML file describes a DAG of agent
//! and command nodes, this crate parses it, validates it, and (from P2b on)
//! runs it serially. GPUI-free — the host resolves launches and paths and
//! hands this crate finished values.

// What a host calls. `load` documents itself as "the one call a host
// makes"; these are the rest of that contract — what a run needs, what it
// reports, and what it leaves on disk.
pub mod error;
pub mod event;
pub mod journal;
pub mod load;
pub mod lock;
pub mod marker;
pub mod node_id;
pub mod record;
pub mod request;
pub mod resume;
pub mod runner;
pub mod schedule;

// Reachable because `load`'s return type and `RunRequest`'s fields name
// them, not because a host is meant to build them.
pub mod graph;
pub mod model;

// The stages `load` runs, and what the scheduler uses to do its work. A
// host never calls these; keeping them crate-private is what lets their
// signatures change without it being an API break.
//
// `parse` is the exception, and deliberately so: a host that *authors* flow
// files edits the file as written, not the resolved flow, so the pre-resolve
// wire types are a contract to it. Changing one is an API break.
pub(crate) mod archive;
pub(crate) mod contract;
pub mod parse;
pub(crate) mod resolve;
pub(crate) mod template;
pub(crate) mod validate;

#[cfg(test)]
pub(crate) mod testing;

pub use error::FlowError;
pub use load::{Inspected, LoadedFlow, inspect, load};
// The one template question an editor asks. The module stays crate-private:
// rendering needs a run's context, which nothing outside a run has.
pub use node_id::NodeId;
pub use template::rename_output_refs;
pub use validate::node_id_is_wellformed;
