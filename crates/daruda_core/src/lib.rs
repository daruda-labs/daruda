//! Shared utilities and core logic for daruda.
//!
//! This crate exists so knowledge needed on *both* sides of the GPUI
//! boundary lives in one place. The app can reach every other crate, but
//! the GPUI-free crates (`daruda_acp`) cannot reach the app — so anything
//! they must agree on has to sit below both of them.
//!
//! # What belongs here
//!
//! - **No dependencies.** Adding one makes this crate a transitive burden
//!   on every consumer, including the deliberately dependency-light
//!   protocol core.
//! - **Two or more consumers.** Code used by exactly one crate belongs in
//!   that crate; moving it here only makes it harder to find.
//! - **Pure.** No I/O, no globals, no environment. `&str`/value in,
//!   value out — so it stays callable from a background executor.
//!
//! A "core" name invites drift into a junk drawer. These criteria are the
//! guard, and they are enforced by review rather than tooling: if a
//! proposed addition fails one of them, it belongs elsewhere.

pub mod language;
