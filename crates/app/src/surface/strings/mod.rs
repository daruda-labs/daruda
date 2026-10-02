//! Every user-visible app-chrome string, generated from `locales/en.yml`.
//!
//! One module per top-level section, one function per key:
//! `flow.more_waiting: "%{n} more"` is `strings::flow::more_waiting(n)`, its
//! doc the `#` lines right above the key. A string that needs logic lives in
//! `custom/<section>.rs` and, named like its key, replaces the generated one.
//! Terminal-view text is in `daruda_terminal::ux::strings`.

mod custom;

include!(concat!(env!("OUT_DIR"), "/strings.rs"));

#[cfg(test)]
mod tests;
