//! User-experience constants — strings the user reads and numbers the
//! user feels (timing, palette, pixel dimensions).
//!
//! Split by change trigger:
//!   * [`strings`] — display text + Durations. The knobs that move when
//!     we localize or retune "feels fast enough" timings.
//!   * [`theme`] — colors, pixel sizes, font sizes. The knobs that move
//!     when we reskin or adjust layout density.
//!
//! Everything user-visible in `daruda_terminal` must flow through one
//! of these two modules. A literal string displayed to the user, a
//! timer that the user can perceive, a magic pixel number — none of
//! them live inline in a view file anymore.

pub mod strings;
pub mod theme;
