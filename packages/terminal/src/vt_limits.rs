//! Size caps used by the cross-chunk VT parsers in `session.rs`.
//!
//! These are tuning knobs, not protocol constants — numbers chosen to
//! bound memory against pathological input without clipping any
//! realistic payload. Raising them should be safe; lowering them needs
//! a look at the scanner call sites first.

/// Maximum entries kept in `TerminalSession::prompt_marks`. Oldest are
/// evicted FIFO. ~32 B per mark × 4096 ≈ 128 KiB — plenty for hours of
/// shell activity without unbounded growth.
pub const PROMPT_MARKS_CAP: usize = 4096;

/// Upper bound on `TerminalSession::parse_tail`. Truncation only
/// happens *after* a scan passes through; a pathological unterminated
/// escape that keeps the scanner stuck past this size triggers a full
/// reset rather than chopping the escape header (which would make OSC
/// 52 / title payloads silently corrupt).
pub const PARSE_TAIL_LIMIT: usize = 65_536;

/// Per-payload cap for the OSC 133 scanner. Long enough for
/// `D;<exit>` plus any realistic semantic annotation; prevents a
/// malformed stream from using the FTCS payload buffer as unbounded
/// memory.
pub const OSC133_PAYLOAD_CAP: usize = 1024;

/// Maximum body size for a single `DCS + q … ST` XTGETTCAP request.
/// A realistic query for a handful of cap names is well under 256 bytes;
/// this cap prevents runaway accumulation from a malformed stream.
pub const XTGETTCAP_BODY_CAP: usize = 256;
