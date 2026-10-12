//! Claude Code skills as the app holds them: the GPUI-free model is
//! `daruda_agent::skills`, and [`global`] makes its state app-wide.

pub mod global;

pub use daruda_agent::skills::*;
