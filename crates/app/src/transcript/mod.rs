//! How an agent transcript is presented: the fold matrix, the display filter,
//! and the tool categories both index by.
//!
//! These sit outside the chat pane because two peers now edit the same values —
//! the pane, which applies them to the transcript in front of the user, and the
//! Settings agent catalog, which authors the defaults a pane starts on. Neither
//! owns the other, so the model and the editor that writes it live here and
//! both hosts reach in.

pub(crate) mod display_filter;
pub(crate) mod editor;
pub(crate) mod fold_mode;
pub(crate) mod tool_category;
