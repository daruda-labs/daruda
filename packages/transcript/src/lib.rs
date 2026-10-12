//! How an agent transcript is presented: the fold matrix, the display filter,
//! and the tool categories both index by.
//!
//! The chat pane applies these values to the transcript in front of the user;
//! the Settings agent catalog authors the defaults a pane starts on. Neither
//! owns the other, so the model and the editor that writes it live here.
//! Only `editor` reaches GPUI; the model modules stay GPUI-free.

rust_i18n::i18n!("locales", fallback = "en");

pub mod command_analysis;
pub mod display_filter;
pub mod editor;
pub mod fold_mode;
pub mod tool_category;

mod strings {
    include!(concat!(env!("OUT_DIR"), "/strings.rs"));
}

#[cfg(test)]
mod locale_tests;
