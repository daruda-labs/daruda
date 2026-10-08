//! Optional inspection of the real desktop startup: replay, capture, and smoke.

#[cfg(feature = "replay")]
pub(super) fn load_replay() -> Option<crate::replay::Loaded> {
    // Load before a restored pane connects: its wire tap could truncate the
    // captured conversation before replay has read it.
    crate::replay::parse_replay_arg()
        .and_then(|path| crate::replay::load(&path, crate::replay::parse_replay_agent_arg()))
}

pub(super) fn after_startup(cx: &mut gpui::App) {
    #[cfg(feature = "screenshot")]
    if let Some(path) = crate::screenshot::parse_screenshot_arg() {
        if crate::screenshot::parse_terminal_widen_flag() {
            crate::screenshot::schedule_terminal_widen_capture(path, cx);
        } else {
            crate::screenshot::schedule_capture(
                path,
                crate::screenshot::parse_scenario_arg(),
                crate::screenshot::parse_themes_arg(),
                crate::screenshot::parse_size_arg(),
                cx,
            );
        }
    }
    // Smoke observes the same initialized desktop as an interactive launch.
    if crate::smoke::requested() {
        crate::smoke::schedule(cx);
    }
}
