//! What a node's declared output has to be for the node to have passed,
//! and the words the agent is told it in.
//!
//! One implementation per question, asked from wherever the answer is
//! needed: the scheduler asks after the runner returns, and a correction
//! turn asks while the session is still open.

// Nothing but registration lives here, so there is no behaviour at this
// level to test — each check is tested in its own submodule.
pub(crate) mod file;
pub(crate) mod prompt;
pub(crate) mod schema;
