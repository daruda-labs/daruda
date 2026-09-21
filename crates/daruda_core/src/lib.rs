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
//!   is deliberately dependency-light, so prefer what they already carry —
//!   and prefer a target-gated one, which costs the platforms that do not
//!   need it nothing. `libc` is here on `cfg(unix)` for that reason. A
//!   plain dependency still has to earn its place: `serde` would qualify
//!   on weight alone and stays out until something here needs it.
//! - **Two or more consumers.** Code used by exactly one crate belongs in
//!   that crate; moving it here only makes it harder to find. A *registry* —
//!   one table every crate has to agree on — is weighed as a whole rather
//!   than entry by entry, since what it buys is that no entry is spelled
//!   anywhere else.
//! - **Pure by default; two named exceptions.** Modules take values in and
//!   return values out. Each exception carries the reason it is one:
//!   - [`process_env`] may read only its registered names, without caching.
//!     It never writes: `set_var` is unsound once the process is
//!     multi-threaded, so bootstrap writes stay where the caller can prove
//!     the process is still single-threaded.
//!   - **Platform capabilities** ([`path`], [`process`], [`shell`]) call the OS, because containing
//!     those calls is what they exist for. The alternative is what they
//!     replace: the same capability spelled out in every domain crate that
//!     needs it, each growing an arm per platform. Splitting "decide" from
//!     "do" would leave the doing in the callers and the duplication with
//!     it. A capability module is admitted on that reasoning rather than on
//!     the consumer count above — it is the *only* place the call may live.
//!
//! A "core" name invites drift into a junk drawer. These criteria are the
//! guard, and they are enforced by review rather than tooling: if a
//! proposed addition fails one of them, it belongs elsewhere.

pub mod git;
pub mod language;
pub mod path;
pub mod process;
pub mod process_env;
pub mod shell;
pub mod text;
