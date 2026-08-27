//! Domain data models and persistence for daruda.
//!
//! GPUI-free — every module contains only data models and file I/O
//! so they remain independently unit-testable without a GPU context.
//!
//! # Modules
//! - [`agent_vocabulary`] — per-agent mode / model option lists as last
//!   advertised by the adapter (`agent_vocabulary.json`)
//! - [`panels`] — bottom dock panel tabs and widgets (`panels.json`)
//! - [`project`] — workspace session state and recent list (`projects/`, `recent.json`)
//! - [`tasks`] — right-panel task queue (`tasks.json`)
//! - [`observability`] — error reports, NDJSON log writer, system info
//!   summary used by the toast / modal / log-file pipeline

pub mod accounts;
pub mod agent_vocabulary;
pub mod marks;
pub mod observability;
pub mod panels;
pub mod persistence;
pub(crate) mod profile;
pub mod project;
pub mod tasks;
