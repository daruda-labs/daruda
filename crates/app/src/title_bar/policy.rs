//! Who draws the window frame, and how a click on a control reaches its
//! action. Both answers are values, resolved once at window construction and
//! passed down — so the arm this host will never run still compiles, is unit
//! tested here, and can be driven into a capture (AGENTS.md "Prefer a value
//! over a `cfg`").
//!
//! Deliberately free of `gpui`: the caller converts `Decorations` to the
//! `server_decorated` fact, which is what makes every arm testable on macOS.

/// The two facts that decide who draws the frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct FrameFacts {
    /// macOS keeps its traffic lights in a transparent title bar, so the OS
    /// still owns the frame even though `appears_transparent` is set.
    pub os_draws_caption: bool,
    /// The compositor draws its own controls — Linux without client-side
    /// decoration support. Drawing ours there would double them.
    pub server_decorated: bool,
}

/// Who draws the window's frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TitleBarChrome {
    /// Something else draws the controls: macOS traffic lights, or a Linux
    /// compositor under server-side decorations. We only reserve space.
    Native,
    /// `appears_transparent` removed the caption and nothing replaced it, so
    /// the app owns drag, minimize, maximize and close.
    Client,
}

/// How a click on a window control reaches its action.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ControlTier {
    /// Windows answers `WM_NCHITTEST` from the hitboxes `window_control_area`
    /// registers and runs the action itself, Snap Layouts included. No click
    /// handler, and `start_window_move` is not implemented there.
    Hitbox,
    /// Linux discards that callback in both backends, so the app runs the
    /// action itself and drags with `start_window_move`.
    Handlers,
}

/// Everything the title bar needs to know about the platform, resolved once.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct WindowChrome {
    pub kind: TitleBarChrome,
    pub tier: ControlTier,
}

impl WindowChrome {
    pub(crate) fn new(facts: FrameFacts, is_windows: bool) -> Self {
        Self {
            kind: chrome_for(facts),
            tier: control_tier(is_windows),
        }
    }

    pub(crate) fn is_client(self) -> bool {
        self.kind == TitleBarChrome::Client
    }
}

pub(crate) const fn chrome_for(facts: FrameFacts) -> TitleBarChrome {
    if facts.os_draws_caption || facts.server_decorated {
        TitleBarChrome::Native
    } else {
        TitleBarChrome::Client
    }
}

pub(crate) const fn control_tier(is_windows: bool) -> ControlTier {
    if is_windows {
        ControlTier::Hitbox
    } else {
        ControlTier::Handlers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: FrameFacts = FrameFacts {
        os_draws_caption: true,
        server_decorated: false,
    };
    const CSD: FrameFacts = FrameFacts {
        os_draws_caption: false,
        server_decorated: false,
    };
    const SSD: FrameFacts = FrameFacts {
        os_draws_caption: false,
        server_decorated: true,
    };

    /// The whole point of taking facts rather than reading `cfg!`: every arm
    /// is reachable from whichever host runs the suite.
    #[test]
    fn only_an_undecorated_window_draws_its_own_chrome() {
        let cases = [
            (MAC, TitleBarChrome::Native, "macOS draws traffic lights"),
            (CSD, TitleBarChrome::Client, "no caption, no compositor"),
            (
                SSD,
                TitleBarChrome::Native,
                "a server-decorated window already has controls",
            ),
        ];
        for (facts, expected, why) in cases {
            assert_eq!(chrome_for(facts), expected, "{why}");
        }
    }

    /// Windows is the only platform whose hit-test callback is wired up; every
    /// other one needs the app to run the action.
    #[test]
    fn only_windows_lets_the_os_run_the_controls() {
        assert_eq!(control_tier(true), ControlTier::Hitbox);
        assert_eq!(control_tier(false), ControlTier::Handlers);
    }

    #[test]
    fn is_client_tracks_the_kind() {
        assert!(WindowChrome::new(CSD, false).is_client());
        assert!(!WindowChrome::new(MAC, false).is_client());
        assert!(!WindowChrome::new(SSD, false).is_client());
    }
}
