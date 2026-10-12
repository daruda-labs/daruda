//! Every user-visible chat-pane string, generated from `locales/en.yml`: one
//! module per section, one function per key. A string that needs logic lives
//! in `custom/<section>.rs` and, named like its key, replaces the generated one.

mod custom;

include!(concat!(env!("OUT_DIR"), "/strings.rs"));

#[cfg(test)]
mod tests;
