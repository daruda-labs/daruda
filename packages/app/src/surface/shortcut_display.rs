//! Render a [`crate::surface::keybindings`] shortcut string for display.
//!
//! The binding strings are GPUI's chord syntax (`secondary-shift-o`), which
//! is what the app binds but not what a user reads. The Landing view's cheat
//! sheet shows the real bindings rather than a hand-kept copy, so the
//! conversion lives here as a pure function over that syntax.
//!
//! Recognition mirrors `gpui::Keystroke::parse`: modifiers are matched **by
//! name** in any order and everything else is the key, including its two
//! special cases — a trailing `-` is the minus key, and a bare uppercase
//! letter means shift plus that letter. Modifiers then print in the
//! platform's canonical order rather than the order they were written.
//!
//! Platform is a parameter, not a `#[cfg]`: both spellings are reachable
//! from either host, so a test on a macOS machine can assert what Windows
//! renders.

/// Which modifiers a chord names, independent of the order it names them in.
#[derive(Default, PartialEq, Eq, Debug)]
struct Mods {
    control: bool,
    alt: bool,
    shift: bool,
    function: bool,
    /// cmd on macOS, the Windows/Super key elsewhere.
    platform: bool,
}

/// Split a chord into its modifiers and its key, following
/// `gpui::Keystroke::parse`'s recognition rules.
fn split(shortcut: &str, mac: bool) -> (Mods, String) {
    let mut mods = Mods::default();
    let mut key = String::new();
    let mut components = shortcut.split('-').peekable();

    while let Some(component) = components.next() {
        if component.eq_ignore_ascii_case("ctrl") {
            mods.control = true;
            continue;
        }
        if component.eq_ignore_ascii_case("alt") {
            mods.alt = true;
            continue;
        }
        if component.eq_ignore_ascii_case("shift") {
            mods.shift = true;
            continue;
        }
        if component.eq_ignore_ascii_case("fn") {
            mods.function = true;
            continue;
        }
        if component.eq_ignore_ascii_case("secondary") {
            if mac {
                mods.platform = true;
            } else {
                mods.control = true;
            }
            continue;
        }
        if component.eq_ignore_ascii_case("cmd")
            || component.eq_ignore_ascii_case("super")
            || component.eq_ignore_ascii_case("win")
        {
            mods.platform = true;
            continue;
        }

        // `secondary--` — the separator itself is the key. gpui detects this
        // as an empty peeked component on a source ending in `-`.
        if components.peek().is_some_and(|next| next.is_empty()) && shortcut.ends_with('-') {
            key = "-".into();
            break;
        }

        // A bare uppercase letter is shift plus the lowercase letter.
        if component.len() == 1 && component.as_bytes()[0].is_ascii_uppercase() {
            mods.shift = true;
        }
        key = component.to_ascii_lowercase();
    }

    (mods, key)
}

/// A chord rendered for a reader, e.g. `⌘⇧O` on macOS and `Ctrl+Shift+O`
/// elsewhere. An unrecognised key passes through capitalised so a chord
/// added without touching this file looks wrong rather than disappearing.
pub fn display_for(shortcut: &str, mac: bool) -> String {
    let (mods, key) = split(shortcut, mac);
    let mut parts: Vec<&str> = Vec::new();

    // Canonical order, not source order: ⌃⌥⇧⌘ is the Apple convention, and
    // Ctrl+Alt+Shift+Win the common one elsewhere.
    if mods.control {
        parts.push(if mac { "⌃" } else { "Ctrl" });
    }
    if mods.alt {
        parts.push(if mac { "⌥" } else { "Alt" });
    }
    if mods.shift {
        parts.push(if mac { "⇧" } else { "Shift" });
    }
    if mods.function {
        parts.push("Fn");
    }
    if mods.platform {
        parts.push(if mac { "⌘" } else { "Win" });
    }

    let key = key_name(&key, mac);
    let mut out = parts.join(if mac { "" } else { "+" });
    if !out.is_empty() && !mac {
        out.push('+');
    }
    out.push_str(&key);
    out
}

/// [`display_for`] on the host this build runs on.
pub fn display(shortcut: &str) -> String {
    display_for(shortcut, cfg!(target_os = "macos"))
}

fn key_name(key: &str, mac: bool) -> String {
    if !mac {
        return capitalize(key);
    }
    match key {
        "up" => "↑".into(),
        "down" => "↓".into(),
        "left" => "←".into(),
        "right" => "→".into(),
        "enter" => "⏎".into(),
        "escape" => "⎋".into(),
        "backspace" => "⌫".into(),
        "delete" => "⌦".into(),
        "tab" => "⇥".into(),
        "space" => "␣".into(),
        other => capitalize(other),
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::keybindings as k;

    /// macOS runs the symbols together, in the HIG's ⌃⌥⇧⌘ order — Finder
    /// writes New Folder as ⇧⌘N, not ⌘⇧N.
    #[test]
    fn mac_joins_symbols_without_separator() {
        assert_eq!(display_for(k::SHORTCUT_OPEN_FOLDER, true), "⌘O");
        assert_eq!(
            display_for(k::SHORTCUT_OPEN_FOLDER_IN_NEW_WINDOW, true),
            "⇧⌘O"
        );
    }

    /// The whole reason platform is a parameter: this asserts the Windows
    /// and Linux spelling from a macOS dev machine, where a `#[cfg]` arm
    /// would never compile. `secondary` is cmd there and ctrl here.
    #[test]
    fn non_mac_spells_modifiers_and_joins_with_plus() {
        assert_eq!(display_for(k::SHORTCUT_OPEN_FOLDER, false), "Ctrl+O");
        assert_eq!(
            display_for(k::SHORTCUT_OPEN_FOLDER_IN_NEW_WINDOW, false),
            "Ctrl+Shift+O"
        );
    }

    #[test]
    fn punctuation_keys_survive() {
        assert_eq!(display_for(k::SHORTCUT_SETTINGS, true), "⌘,");
        assert_eq!(display_for(k::SHORTCUT_KEYBOARD_SHORTCUTS, true), "⌘/");
    }

    /// `secondary--` is the `-` key, not a chord ending in an empty
    /// segment — gpui's parser has a dedicated branch for it, and a naive
    /// split-and-join renders it as `Ctrl++-`.
    #[test]
    fn trailing_separator_is_the_minus_key() {
        assert_eq!(display_for("secondary--", true), "⌘-");
        assert_eq!(display_for("secondary--", false), "Ctrl+-");
    }

    #[test]
    fn named_keys_become_glyphs_on_mac_only() {
        assert_eq!(display_for(k::SHORTCUT_JUMP_PROMPT_PREV, true), "⇧⌘↑");
        assert_eq!(
            display_for(k::SHORTCUT_JUMP_PROMPT_PREV, false),
            "Ctrl+Shift+Up"
        );
    }

    /// Modifiers are recognised by name in any order and printed in the
    /// canonical one, so two spellings of one chord render identically.
    #[test]
    fn modifier_order_is_canonical_not_as_written() {
        assert_eq!(display_for("shift-secondary-k", true), "⇧⌘K");
        assert_eq!(display_for("secondary-shift-k", true), "⇧⌘K");
    }

    /// A bare uppercase letter means shift plus that letter — gpui folds it
    /// into the modifier set, so the display has to as well.
    #[test]
    fn bare_uppercase_letter_implies_shift() {
        assert_eq!(display_for("secondary-K", true), "⇧⌘K");
    }

    /// An unrecognised key must still render — a chord added to
    /// `keybindings.rs` without touching this file should look wrong, not
    /// disappear from the cheat sheet.
    #[test]
    fn unknown_keys_pass_through_capitalised() {
        assert_eq!(display_for("secondary-f13", false), "Ctrl+F13");
    }

    /// Every binding the cheat sheet can show renders non-empty on both
    /// platforms, so a remap can't blank a row.
    #[test]
    fn every_landing_binding_renders_on_both_platforms() {
        for binding in [
            k::SHORTCUT_OPEN_FOLDER,
            k::SHORTCUT_OPEN_FOLDER_IN_NEW_WINDOW,
            k::SHORTCUT_NEW_WINDOW,
            k::SHORTCUT_KEYBOARD_SHORTCUTS,
        ] {
            assert!(!display_for(binding, true).is_empty(), "{binding} on mac");
            assert!(!display_for(binding, false).is_empty(), "{binding} off mac");
        }
    }
}
