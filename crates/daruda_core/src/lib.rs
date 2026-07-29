//! Shared utilities and core logic for daruda.
//!
//! This crate exists so knowledge needed on *both* sides of the GPUI
//! boundary lives in one place. The app can reach every other crate, but
//! the GPUI-free crates (`daruda_acp`) cannot reach the app — so anything
//! they must agree on has to sit below both of them.
//!
//! # What belongs here
//!
//! Every consumer points here and this crate points at none of them, so
//! the dependency question is directional, not a matter of count:
//!
//! - **Nothing from `daruda_*`.** Reaching back into the workspace inverts
//!   the layering. External crates sit below all of it, so those are fine.
//! - **Never `gpui`.** Not a weight judgment: this crate exists so the
//!   GPUI-free crates can share knowledge with the app, and a GPUI
//!   dependency would put it back out of their reach.
//! - **Weigh anything else.** Every consumer inherits it and `daruda_acp`
//!   is deliberately dependency-light, so prefer what they already carry.
//!   Today only `serde` would qualify, and it stays out until something
//!   here actually needs it.
//! - **Two or more consumers.** Code used by exactly one crate belongs in
//!   that crate; moving it here only makes it harder to find.
//! - **Pure.** No I/O, no globals, no environment. `&str`/value in,
//!   value out — so it stays callable from a background executor.
//!
//! A "core" name invites drift into a junk drawer. These criteria are the
//! guard, and they are enforced by review rather than tooling: if a
//! proposed addition fails one of them, it belongs elsewhere.

pub mod git;
pub mod language;
pub mod text;
