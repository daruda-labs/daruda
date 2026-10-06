//! Keystroke decode for the picker overlays — the one GPUI-facing rule
//! in the shared picker layer, which is why it sits beside
//! [`super::picker`] rather than in it: that module imports no GPUI
//! types so its rules are testable without a window.
//!
//! Owns the modifier early-out and the `key_char` reduction, and only
//! those. The open-check (a different predicate per overlay), the
//! `on_key` receiver, the [`super::picker::PickerKey`] arms, and
//! `stop_propagation` (which must run *after* dispatch) stay at the
//! call site.

/// A keystroke reduced to what a picker needs, or `None` when it is not
/// the overlay's to take.
///
/// `None` means leave it for the action system: a `platform`/`function`
/// keystroke must reach the window, or the key that opened the overlay
/// can no longer close it and `Cmd+W` stops working. The caller returns
/// *without* `stop_propagation` — which is what separates this from
/// [`super::picker::PickerKey::Unchanged`], where the keystroke is
/// swallowed. Same early-out the terminal view takes, for the same
/// reason.
pub(in crate::workspace) fn picker_keystroke(
    ev: &gpui::KeyDownEvent,
) -> Option<(&str, Option<char>)> {
    if ev.keystroke.modifiers.platform || ev.keystroke.modifiers.function {
        return None;
    }
    let ch = ev
        .keystroke
        .key_char
        .as_deref()
        .and_then(|s| s.chars().next());
    Some((ev.keystroke.key.as_str(), ch))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(modifiers: gpui::Modifiers, key: &str, key_char: Option<&str>) -> gpui::KeyDownEvent {
        gpui::KeyDownEvent {
            keystroke: gpui::Keystroke {
                modifiers,
                key: key.to_string(),
                key_char: key_char.map(str::to_string),
            },
            is_held: false,
            prefer_character_input: false,
        }
    }

    /// The `Cmd+W`-breaking case: a platform chord is the action
    /// system's, so the overlay must hand it back rather than decode it.
    #[test]
    fn a_platform_chord_is_not_the_overlays_to_take() {
        let modifiers = gpui::Modifiers {
            platform: true,
            ..Default::default()
        };
        assert!(picker_keystroke(&event(modifiers, "w", Some("w"))).is_none());
    }

    #[test]
    fn a_function_chord_is_not_the_overlays_to_take() {
        let modifiers = gpui::Modifiers {
            function: true,
            ..Default::default()
        };
        assert!(picker_keystroke(&event(modifiers, "up", None)).is_none());
    }

    #[test]
    fn a_plain_printable_key_carries_its_char() {
        let ev = event(gpui::Modifiers::default(), "x", Some("x"));
        assert_eq!(picker_keystroke(&ev), Some(("x", Some('x'))));
    }

    #[test]
    fn a_plain_named_key_carries_no_char() {
        let ev = event(gpui::Modifiers::default(), "escape", None);
        assert_eq!(picker_keystroke(&ev), Some(("escape", None)));
    }
}
