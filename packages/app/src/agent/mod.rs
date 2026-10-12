//! Agent-side data models — MCP server registry, Skills catalog, and
//! the GPUI Global wrapper over `daruda_store::tasks::TasksState`.
//! All submodules are GPUI-free except `tasks_global`, which exists
//! solely to attach the `impl Global` marker.

pub mod account;
pub mod adapter_cli;
pub mod adapter_prep;
pub(crate) mod configuration;
pub mod launch_resolve;
pub mod mcp;
pub mod skills;
pub mod tasks_global;
