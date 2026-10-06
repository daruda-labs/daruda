//! JSONL fallback channel — ported from c9watch (MIT).
//!
//! Reads the tail of `~/.claude/projects/<encoded>/<session>.jsonl` and
//! infers `SessionStatus` from message-history structure. Used when the
//! hook channel is unavailable (user hasn't installed daruda hooks yet,
//! or `disableAllHooks: true`).
//!
//! See `LICENSE-THIRD-PARTY.md` for c9watch attribution.

pub mod fsm;
pub mod parser;
pub mod permissions;
pub mod tail;
