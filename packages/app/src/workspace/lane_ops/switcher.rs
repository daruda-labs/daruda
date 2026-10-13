//! The fuzzy lane switcher (Cmd+P): open / close, key handling, row pick,
//! and the screenshot seam that overflows a label.

use daruda_store::project::LaneRef;
use gpui::{Context, Window};

use super::lane_label;

use crate::workspace::ToggleLaneSwitcher;
use crate::workspace::Workspace;
use crate::workspace::command::lane_switcher::LaneCandidate;
use crate::workspace::command::picker::PickerKey;
use crate::workspace::command::picker_key::picker_keystroke;

/// Synthetic label for the `lane-switcher` screenshot scenario. Not
/// user-facing, so it does not go through `surface/strings/`.
#[cfg(feature = "screenshot")]
const LANE_SWITCHER_LONG_LABEL_SAMPLE: &str = "daruda-extremely-long-project-name-for-clipping-verification / feature/an-extremely-long-branch-name-meant-to-overflow-the-popup-width";

impl Workspace {
    // ---- Lane switcher (Cmd+P) ----

    /// Toggle the fuzzy Lane switcher. On open, snapshot one candidate
    /// per lane across every project so the overlay render reads a frozen
    /// list (MVU: render never reaches into live project state).
    pub(in crate::workspace) fn on_toggle_lane_switcher(
        &mut self,
        _: &ToggleLaneSwitcher,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlays.lane_switcher.is_open {
            self.overlays.lane_switcher.close();
        } else {
            let candidates = self.lane_switcher_candidates();
            self.overlays.lane_switcher.open(candidates);
        }
        cx.notify();
    }

    /// One [`LaneCandidate`] per lane across every project, labelled by
    /// [`lane_label`].
    fn lane_switcher_candidates(&self) -> Vec<LaneCandidate> {
        self.projects
            .lanes()
            .map(|(lane_ref, project, lane)| LaneCandidate {
                lane_ref,
                label: lane_label(&project.name, lane),
            })
            .collect()
    }

    /// [`lane_label`] for a lane named only by its ref. A row can outlive
    /// the lane it names, so a stale ref still answers with the project it
    /// came from rather than with nothing.
    pub(in crate::workspace) fn lane_label_for(&self, lane_ref: LaneRef) -> String {
        let Some(project) = self.project_for(lane_ref.project) else {
            return String::new();
        };
        match project.lane(lane_ref.lane) {
            Some(lane) => lane_label(&project.name, lane),
            None => project.name.clone(),
        }
    }

    /// Close the switcher without activating anything — the backdrop click.
    pub(in crate::workspace) fn close_lane_switcher(&mut self, cx: &mut Context<Self>) {
        self.overlays.lane_switcher.close();
        cx.notify();
    }

    /// Activate the row the mouse named. Focusing it first is what makes a
    /// click the same gesture as arrowing there and pressing Enter.
    pub(in crate::workspace) fn pick_lane_switcher_row(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.overlays.lane_switcher.picker.focus(ix);
        self.execute_lane_switcher_selection(window, cx);
    }

    /// Activate the focused lane and close the switcher.
    pub(in crate::workspace) fn execute_lane_switcher_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.overlays.lane_switcher.focused_lane_ref();
        self.close_lane_switcher(cx);
        if let Some(target) = target {
            self.activate_lane(target, window, cx);
        }
    }

    /// Route one keystroke into the open Lane switcher. The view decides
    /// *whether* to attach this (its `.when(is_open, …)` wrapper); the
    /// re-check here covers a listener firing on a frame where the
    /// switcher has already closed.
    pub(in crate::workspace) fn on_lane_switcher_key(
        &mut self,
        ev: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.overlays.lane_switcher.is_open {
            return;
        }
        let Some((key, ch)) = picker_keystroke(ev) else {
            return;
        };
        let visible_len = self.overlays.lane_switcher.visible().len();
        match self
            .overlays
            .lane_switcher
            .picker
            .on_key(key, ch, visible_len)
        {
            PickerKey::Confirm => self.execute_lane_switcher_selection(window, cx),
            PickerKey::Dismiss => self.close_lane_switcher(cx),
            PickerKey::Changed => cx.notify(),
            PickerKey::Unchanged => {}
        }
        cx.stop_propagation();
    }

    /// Open the Lane switcher with its real candidates, the first row's
    /// label swapped for [`LANE_SWITCHER_LONG_LABEL_SAMPLE`] (why, on
    /// [`crate::workspace::screenshot_scenario::ScreenshotScenario::LaneSwitcher`]).
    /// Reuses that candidate's real `lane_ref` rather than fabricating one,
    /// so picking the row still activates a lane that exists; no-ops when
    /// the workspace has no lanes.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_lane_switcher_long_label_for_shot(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let mut candidates = self.lane_switcher_candidates();
        let Some(first) = candidates.first_mut() else {
            return;
        };
        first.label = LANE_SWITCHER_LONG_LABEL_SAMPLE.to_string();
        self.overlays.lane_switcher.open(candidates);
        cx.notify();
    }
}
