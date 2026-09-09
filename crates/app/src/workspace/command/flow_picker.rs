//! Flow picker — the list of flows in the active lane, opened by either
//! `Run Flow…` or `Validate Flow…`.
//!
//! Mirrors [`super::lane_switcher`]: a pure state snapshot plus a
//! [`RenderOnce`] overlay, so the Workspace render path carries no
//! state-transition logic. Candidates are read from disk when the picker
//! opens; the overlay only reads that snapshot.
//!
//! Unlike the lane switcher this is an enum rather than an `is_open`
//! flag beside fields that only mean something while open — the picker
//! also carries *which* of the two entries opened it, and a closed
//! picker has no answer to that.

use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    App, IntoElement, MouseButton, MouseDownEvent, RenderOnce, SharedString, Window, div,
    prelude::*, px,
};

use super::picker::{PickerKey, PickerState};
use crate::surface::strings;
use crate::ui::theme;

/// One line of the overlay's list. The tag is separate from the label so
/// the query only ever matches the name.
pub(in crate::workspace) struct Row {
    pub label: SharedString,
    tag: Option<SharedString>,
}

/// What the picked flow is for. The three entries share one list and one
/// overlay, and differ only in what Enter does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum FlowPurpose {
    /// Static checks only — no session, no lock, no run directory.
    Validate,
    Run,
    /// Draw the flow. Reads the file and nothing else — no session, no
    /// lock, and no profile question: a graph is the file's shape, and
    /// which profile a *run* merged under is a question the run answers.
    Graph,
}

impl FlowPurpose {
    /// Whether a run already going in this lane stands in the way.
    ///
    /// Only a second run is in its way: the lock is what a run holds while
    /// it owns the lane's working tree, and two schedulers over one tree is
    /// the thing it exists to prevent. Reading the file to check it or to
    /// draw it takes nothing the running one holds.
    pub(in crate::workspace) fn blocked_by_a_running_flow(self) -> bool {
        matches!(self, FlowPurpose::Run)
    }

    /// Whether the file's `profiles` are a question worth asking.
    ///
    /// A profile is a layer merged over `defaults`, so it decides what a run
    /// does and therefore what a check has to check — neither can be
    /// answered without knowing which one. A graph is the file's shape, and
    /// no layer moves that.
    pub(in crate::workspace) fn asks_about_profiles(self) -> bool {
        matches!(self, FlowPurpose::Run | FlowPurpose::Validate)
    }
}

/// One flow file, captured when the picker opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct FlowCandidate {
    pub path: PathBuf,
    /// The file name as it is on disk, extension included. A stem would
    /// render two different files (`a.yaml`, `a.yml`) as one row.
    pub label: String,
    pub origin: crate::workspace::flow_paths::FlowOrigin,
}

impl FlowCandidate {
    pub fn from_found(found: crate::workspace::flow_paths::FoundFlow) -> Self {
        let label = crate::workspace::flow_paths::flow_label(&found.path);
        Self {
            path: found.path,
            label,
            origin: found.origin,
        }
    }

    /// The tag beside the name, or none for the ordinary case. Kept out of
    /// `label` on purpose: the query matches against the label, and a
    /// searchable "global" would put every one of them in front of the
    /// person typing the name of a repo flow.
    fn tag(&self) -> Option<SharedString> {
        match self.origin {
            // The ordinary case is now two: a flow committed with the repo and
            // one this machine keeps for it. Neither needs a tag — what the tag
            // is for is the one that belongs to no project.
            crate::workspace::flow_paths::FlowOrigin::Repo
            | crate::workspace::flow_paths::FlowOrigin::Project => None,
            crate::workspace::flow_paths::FlowOrigin::Global => {
                Some(SharedString::from(strings::flow_picker_global()))
            }
        }
    }
}

/// One profile a flow declares, plus the file's own `defaults`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct ProfileCandidate {
    /// `None` is the flow as written. Offered because declaring a profile
    /// must not take away the ability to run the base file — the profiles
    /// are layers over `defaults`, not replacements for it.
    pub name: Option<String>,
    pub label: String,
}

impl ProfileCandidate {
    fn defaults() -> Self {
        Self {
            name: None,
            label: strings::flow_picker_profile_defaults(),
        }
    }

    fn named(name: String) -> Self {
        Self {
            label: name.clone(),
            name: Some(name),
        }
    }
}

/// Which question the open picker is asking. The rows live inside the
/// stage rather than beside it, so a list of profiles cannot be shown
/// while a pick would be read as a flow.
#[derive(Clone, Debug)]
pub(in crate::workspace) enum Stage {
    Flows {
        candidates: Vec<FlowCandidate>,
    },
    /// Which profile to run `flow` under. Only reached for a flow that
    /// declares any — a file with none is run the moment it is picked.
    Profiles {
        flow: PathBuf,
        /// How far to run and what to reuse, as the surface that opened this
        /// asked for it. Carried rather than re-derived on the way out: the
        /// graph pane's selection can have moved while the list was up.
        selection: crate::workspace::flow_request::FlowSelection,
        candidates: Vec<ProfileCandidate>,
    },
}

impl Stage {
    /// What the query matches against — the names only, never the tag.
    fn labels(&self) -> Vec<&str> {
        match self {
            Stage::Flows { candidates } => candidates.iter().map(|c| c.label.as_str()).collect(),
            Stage::Profiles { candidates, .. } => {
                candidates.iter().map(|c| c.label.as_str()).collect()
            }
        }
    }

    pub(in crate::workspace) fn row(&self, index: usize) -> Option<Row> {
        match self {
            Stage::Flows { candidates } => candidates.get(index).map(|c| Row {
                label: SharedString::from(c.label.clone()),
                tag: c.tag(),
            }),
            Stage::Profiles { candidates, .. } => candidates.get(index).map(|c| Row {
                label: SharedString::from(c.label.clone()),
                tag: None,
            }),
        }
    }
}

/// The picker while it is showing a list.
#[derive(Clone, Debug)]
pub(in crate::workspace) struct Choosing {
    pub purpose: FlowPurpose,
    /// The worktree the answer will run in. Held here rather than read from
    /// the workspace when the answer comes back: the picker may have been
    /// opened for a worktree that is not the one on screen, and the second
    /// question is asked *after* the first was answered.
    pub lane: daruda_store::project::LaneRef,
    pub stage: Stage,
    /// The typed query and the keyboard selection, shared with the other
    /// two pickers. Private to this module: a list key reaches it through
    /// [`FlowPicker::on_key`], which is also the one place that knows the
    /// stop prompt has no list to key against.
    picker: PickerState,
}

/// What Enter acted on. Two stages, so a pick says which question it
/// answered — the host runs nothing until it holds the second.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum FlowPick {
    /// A flow. Whether a profile is asked for next is the host's to decide,
    /// because only it can read the file.
    Flow {
        lane: daruda_store::project::LaneRef,
        purpose: FlowPurpose,
        path: PathBuf,
    },
    /// A profile for the flow already picked. `profile: None` is the file as
    /// written.
    Profile {
        lane: daruda_store::project::LaneRef,
        purpose: FlowPurpose,
        path: PathBuf,
        selection: crate::workspace::flow_request::FlowSelection,
        profile: Option<String>,
    },
}

#[derive(Clone, Debug, Default)]
pub(in crate::workspace) enum FlowPicker {
    #[default]
    Closed,
    Choosing(Choosing),
    /// A run already holds `lane`, so the list is not the question — whether
    /// to stop it is. Derived from the lock rather than from a field, which
    /// is what lets a run started before this app launched be recognised at
    /// all. Carries the worktree because the answer stops *that* run, not
    /// whichever one is on screen when Enter lands.
    Stopping {
        lane: daruda_store::project::LaneRef,
    },
}

impl FlowPicker {
    pub fn open(
        &mut self,
        lane: daruda_store::project::LaneRef,
        purpose: FlowPurpose,
        found: Vec<crate::workspace::flow_paths::FoundFlow>,
    ) {
        *self = FlowPicker::Choosing(Choosing {
            purpose,
            lane,
            stage: Stage::Flows {
                candidates: found.into_iter().map(FlowCandidate::from_found).collect(),
            },
            picker: PickerState::default(),
        });
    }

    /// Ask which profile `flow` runs under. The query and the focus start
    /// over: they were about a different list, and carrying them would
    /// filter profile names by whatever was typed to find the file.
    pub fn ask_profile(
        &mut self,
        lane: daruda_store::project::LaneRef,
        purpose: FlowPurpose,
        flow: PathBuf,
        selection: crate::workspace::flow_request::FlowSelection,
        names: Vec<String>,
    ) {
        *self = FlowPicker::Choosing(Choosing {
            purpose,
            lane,
            stage: Stage::Profiles {
                flow,
                selection,
                candidates: std::iter::once(ProfileCandidate::defaults())
                    .chain(names.into_iter().map(ProfileCandidate::named))
                    .collect(),
            },
            picker: PickerState::default(),
        });
    }

    pub fn close(&mut self) {
        *self = FlowPicker::Closed;
    }

    pub fn is_open(&self) -> bool {
        !matches!(self, FlowPicker::Closed)
    }

    pub fn choosing(&self) -> Option<&Choosing> {
        match self {
            FlowPicker::Choosing(c) => Some(c),
            FlowPicker::Closed | FlowPicker::Stopping { .. } => None,
        }
    }

    fn choosing_mut(&mut self) -> Option<&mut Choosing> {
        match self {
            FlowPicker::Choosing(c) => Some(c),
            FlowPicker::Closed | FlowPicker::Stopping { .. } => None,
        }
    }

    /// What a keystroke means to the picker, in the two halves the enum
    /// makes of the question.
    ///
    /// Escape and Enter are the overlay's, whichever state it is in: the
    /// stop prompt has no list and still has to answer both — Enter is the
    /// only way to the stop (see `execute_flow_picker_selection`, which
    /// reads it by `focused_pick` being `None` there). That half is
    /// [`super::picker::overlay_key`], shared so this type does not carry
    /// a second copy of it. Everything else is a list key, so it reaches
    /// [`PickerState`] only when there is a list; the stop prompt refuses
    /// it outright rather than no-opping on a list that is not on screen.
    pub fn on_key(&mut self, key: &str, ch: Option<char>) -> PickerKey {
        if let Some(k) = super::picker::overlay_key(key) {
            return k;
        }
        match self.choosing_mut() {
            Some(c) => {
                // Already capped by `visible`, which is where the cap
                // lives.
                let visible_len = c.visible().len();
                c.picker.on_key(key, ch, visible_len)
            }
            None => PickerKey::Unchanged,
        }
    }

    /// Move the focus to a row the mouse named. Clicking is the same
    /// gesture as arrowing there and pressing Enter, so it goes through the
    /// same field rather than a second path to the same decision.
    pub fn focus(&mut self, index: usize) {
        if let Some(c) = self.choosing_mut() {
            c.picker.focus(index);
        }
    }

    /// The line over the list. Decided here rather than at the render
    /// site: it follows from which question is being asked, and that is
    /// this type's to know.
    pub fn prompt(&self) -> String {
        let Some(c) = self.choosing() else {
            return strings::flow_picker_prompt_run();
        };
        match (&c.stage, c.purpose) {
            (Stage::Profiles { flow, .. }, _) => {
                strings::flow_picker_prompt_profile(&crate::workspace::flow_paths::flow_label(flow))
            }
            (Stage::Flows { .. }, FlowPurpose::Validate) => strings::flow_picker_prompt_validate(),
            (Stage::Flows { .. }, FlowPurpose::Run) => strings::flow_picker_prompt_run(),
            (Stage::Flows { .. }, FlowPurpose::Graph) => strings::flow_picker_prompt_graph(),
        }
    }

    /// What Enter acts on, and which of the two questions it answered.
    pub fn focused_pick(&self) -> Option<FlowPick> {
        let c = self.choosing()?;
        let &index = c.visible().get(c.picker.focused_index())?;
        match &c.stage {
            Stage::Flows { candidates } => Some(FlowPick::Flow {
                lane: c.lane,
                purpose: c.purpose,
                path: candidates.get(index)?.path.clone(),
            }),
            Stage::Profiles {
                flow,
                selection,
                candidates,
            } => Some(FlowPick::Profile {
                lane: c.lane,
                purpose: c.purpose,
                path: flow.clone(),
                selection: selection.clone(),
                profile: candidates.get(index)?.name.clone(),
            }),
        }
    }
}

impl Choosing {
    /// Candidate indices for the rows actually drawn, best match first. An
    /// empty query yields every candidate in original order — the order
    /// [`Stage::labels`] lists them in, which is the one the flows and the
    /// profiles are meant to be read in.
    pub fn visible(&self) -> Vec<usize> {
        self.picker.visible(&self.stage.labels())
    }
}

/// GPUI render-once wrapper for the floating overlay. Renders an empty
/// invisible div when the picker is closed.
#[derive(IntoElement)]
pub(in crate::workspace) struct FlowPickerOverlay {
    state: FlowPicker,
    prompt: SharedString,
    empty: SharedString,
    stop_prompt: SharedString,
    stop_action: SharedString,
    #[allow(clippy::type_complexity)]
    on_close: Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>,
    /// Activate the row at this visible index. `Rc` because every row needs
    /// its own handle to it.
    #[allow(clippy::type_complexity)]
    on_pick: Rc<dyn Fn(&usize, &mut Window, &mut App) + 'static>,
}

impl FlowPickerOverlay {
    pub(in crate::workspace) fn new(
        state: FlowPicker,
        prompt: impl Into<SharedString>,
        empty: impl Into<SharedString>,
        stop_prompt: impl Into<SharedString>,
        stop_action: impl Into<SharedString>,
        on_close: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
        on_pick: impl Fn(&usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            state,
            prompt: prompt.into(),
            empty: empty.into(),
            stop_prompt: stop_prompt.into(),
            stop_action: stop_action.into(),
            on_close: Box::new(on_close),
            on_pick: Rc::new(on_pick),
        }
    }
}

/// Full-screen absolute overlay — click-to-dismiss hit target. Mirrors
/// the lane switcher's, which mirrors the palette's; each module keeps
/// its own because the palette's is private to it.
fn backdrop() -> gpui::Div {
    div().absolute().size_full().top_0().left_0()
}

impl RenderOnce for FlowPickerOverlay {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        if matches!(self.state, FlowPicker::Closed) {
            return div().into_any_element();
        }
        let t = theme::current(cx);
        let input_border = t.border;
        let query_text = t.text_primary;
        let tag_text = t.text_subtle;
        let panel_bg = t.palette_bg;
        let panel_border = t.border;

        // Both open states are the same panel over a different list: the
        // flows to pick from, or the single thing there is to do about a
        // run that is already going.
        let (prompt, rows) = match &self.state {
            FlowPicker::Closed => unreachable!("returned above"),
            FlowPicker::Stopping { .. } => (
                self.stop_prompt.clone(),
                vec![Row {
                    label: self.stop_action.clone(),
                    tag: None,
                }],
            ),
            FlowPicker::Choosing(state) => (
                if state.picker.query().is_empty() {
                    self.prompt.clone()
                } else {
                    SharedString::from(state.picker.query().to_string())
                },
                state
                    .visible()
                    .iter()
                    .filter_map(|&i| state.stage.row(i))
                    .collect(),
            ),
        };
        let focused_index = self
            .state
            .choosing()
            .map_or(0, |c| c.picker.focused_index());

        let input = div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .px(px(theme::PALETTE_INPUT_PAD_X))
            .py(px(theme::PALETTE_INPUT_PAD_Y))
            .border_b_1()
            .border_color(input_border)
            .child(
                div()
                    .text_size(px(theme::PALETTE_QUERY_FONT_SIZE))
                    .text_color(query_text)
                    .child(prompt),
            );

        let entries = div()
            .flex()
            .flex_col()
            .max_h(px(theme::PALETTE_MAX_HEIGHT))
            .overflow_hidden()
            .children(rows.iter().enumerate().map(|(index, row)| {
                let tag = row.tag.clone().map(|tag| {
                    div()
                        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
                        .text_color(tag_text)
                        .child(tag)
                        .into_any_element()
                });
                let on_pick = self.on_pick.clone();
                crate::ui::picker_row(
                    index == focused_index,
                    row.label.clone(),
                    tag,
                    move |window, cx| on_pick(&index, window, cx),
                    cx,
                )
            }));

        let no_results = rows
            .is_empty()
            .then(|| crate::ui::picker_empty(self.empty.clone(), cx));

        let panel = div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .mx_auto()
            .mt(px(theme::PALETTE_TOP_OFFSET))
            .w(px(theme::PALETTE_WIDTH))
            .bg(panel_bg)
            .border_1()
            .border_color(panel_border)
            .rounded(px(theme::PALETTE_RADIUS))
            .shadow_lg()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
            })
            .child(input)
            .child(entries)
            .when_some(no_results, |el, nr| el.child(nr));

        backdrop()
            .on_mouse_down(MouseButton::Left, self.on_close)
            .child(panel)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Any worktree — these tests are about the list, not about where a pick
    /// would run. That the value survives to the pick is
    /// `a_pick_carries_the_worktree_it_was_opened_for`'s business.
    const LANE: daruda_store::project::LaneRef = daruda_store::project::LaneRef {
        project: 0,
        lane: 0,
    };

    fn opened(purpose: FlowPurpose, names: &[&str]) -> FlowPicker {
        let mut picker = FlowPicker::default();
        picker.open(
            LANE,
            purpose,
            names
                .iter()
                .map(|n| crate::workspace::flow_paths::FoundFlow {
                    path: PathBuf::from("/lane/f").join(n),
                    origin: crate::workspace::flow_paths::FlowOrigin::Repo,
                })
                .collect(),
        );
        picker
    }

    /// A global flow is marked and a repository's is not — and the mark
    /// never reaches the query, or typing a repo flow's name would rank
    /// every global one alongside it.
    #[test]
    fn only_a_global_flow_carries_a_tag_and_the_tag_is_not_searchable() {
        let mut picker = FlowPicker::default();
        picker.open(
            LANE,
            FlowPurpose::Run,
            vec![
                crate::workspace::flow_paths::FoundFlow {
                    path: PathBuf::from("/lane/.daruda/flows/ship.yaml"),
                    origin: crate::workspace::flow_paths::FlowOrigin::Repo,
                },
                crate::workspace::flow_paths::FoundFlow {
                    path: PathBuf::from("/home/flows/tidy.yaml"),
                    origin: crate::workspace::flow_paths::FlowOrigin::Global,
                },
            ],
        );
        let stage = &picker.choosing().expect("open").stage;
        assert!(
            stage.row(0).expect("row").tag.is_none(),
            "the repo's own was tagged"
        );
        assert!(
            stage.row(1).expect("row").tag.is_some(),
            "a global flow was not marked"
        );
        // `labels` is what the query is matched against, and it is a
        // different reader of the same field than `row` — asserting the
        // rendered label alone would leave the searchable half open.
        assert_eq!(stage.labels(), ["ship.yaml", "tidy.yaml"]);
    }

    /// Two entries share one list, so the picked flow is meaningless
    /// without knowing which entry opened it. They travel together.
    #[test]
    fn a_pick_carries_what_it_was_opened_for() {
        let picker = opened(FlowPurpose::Validate, &["ship.yaml"]);
        let FlowPick::Flow { purpose, path, .. } = picker.focused_pick().expect("a pick") else {
            panic!("the first question is which flow");
        };
        assert_eq!(purpose, FlowPurpose::Validate);
        assert!(path.ends_with("ship.yaml"));
    }

    /// The worktree has to survive to the pick. A picker opened for a parked
    /// worktree that answered with whichever one is on screen would run the
    /// flow in the wrong place — and the answer comes back after the question,
    /// so the value cannot be re-read then.
    #[test]
    fn a_pick_carries_the_worktree_it_was_opened_for() {
        let elsewhere = daruda_store::project::LaneRef {
            project: 3,
            lane: 7,
        };
        let mut picker = FlowPicker::default();
        picker.open(
            elsewhere,
            FlowPurpose::Run,
            vec![crate::workspace::flow_paths::FoundFlow {
                path: PathBuf::from("/lane/.daruda/flows/ship.yaml"),
                origin: crate::workspace::flow_paths::FlowOrigin::Repo,
            }],
        );
        let FlowPick::Flow { lane, .. } = picker.focused_pick().expect("a pick") else {
            panic!("the first question is which flow");
        };
        assert_eq!(lane, elsewhere);

        // And across the second question, which is where it used to be lost.
        picker.ask_profile(
            elsewhere,
            FlowPurpose::Run,
            PathBuf::from("/lane/.daruda/flows/ship.yaml"),
            crate::workspace::flow_request::FlowSelection::default(),
            vec!["cheap".into()],
        );
        let FlowPick::Profile { lane, .. } = picker.focused_pick().expect("a pick") else {
            panic!("the second question is which profile");
        };
        assert_eq!(lane, elsewhere);
    }

    /// Enter acts on the row the arrow keys walked to. Written when the
    /// picker appeared to run the wrong file: the state was right all
    /// along, and what was missing was any way to *see* which row was
    /// focused (the tint alone is 1.6 lightness points on a near-black
    /// panel). The assertion stays because it is the half that a
    /// screenshot cannot check.
    #[test]
    fn enter_acts_on_the_row_the_arrows_walked_to() {
        let mut picker = opened(
            FlowPurpose::Validate,
            &[
                "01-ok.yaml",
                "02-broken.yaml",
                "03-unknown-agent.yaml",
                "04-stop.yaml",
                "05-notyaml.yaml",
            ],
        );
        picker.on_key("down", None);
        let FlowPick::Flow { path, .. } = picker.focused_pick().expect("a pick") else {
            panic!("the first question is which flow");
        };
        assert!(path.ends_with("02-broken.yaml"), "picked {path:?}");
    }

    /// A closed picker has no query, no selection and no purpose — the
    /// reason this is an enum and not a flag beside four fields.
    #[test]
    fn a_closed_picker_holds_nothing() {
        let mut picker = opened(FlowPurpose::Run, &["ship.yaml"]);
        picker.on_key("s", Some('s'));
        picker.close();
        assert!(!picker.is_open());
        assert!(picker.choosing().is_none());
        assert!(picker.focused_pick().is_none());
    }

    /// Typing narrows and Enter follows the narrowed list, not the
    /// original one — the index is into `filtered`, not `candidates`.
    #[test]
    fn enter_follows_the_filtered_list() {
        let mut picker = opened(FlowPurpose::Run, &["build.yaml", "review.yaml"]);
        picker.on_key("r", Some('r'));
        picker.on_key("v", Some('v'));
        let FlowPick::Flow { path, .. } = picker.focused_pick().expect("a pick") else {
            panic!("the first question is which flow");
        };
        assert!(path.ends_with("review.yaml"), "{path:?}");
    }

    /// Moving down cannot walk off the end of what is actually drawn. The
    /// cap itself is [`PickerState`]'s (and tested there); what this pins is
    /// that the row count handed to it is this picker's own visible list, so
    /// the delegation cannot be wired to an uncapped or unfiltered length.
    #[test]
    fn the_selection_stays_inside_the_visible_list() {
        let mut picker = opened(FlowPurpose::Run, &["a.yaml", "b.yaml"]);
        for _ in 0..5 {
            picker.on_key("down", None);
        }
        let FlowPick::Flow { path, .. } = picker.focused_pick().expect("a pick") else {
            panic!("the first question is which flow");
        };
        assert!(path.ends_with("b.yaml"), "{path:?}");
    }

    /// The table is the specification: one row per purpose, both columns
    /// explicit, so adding a variant means stating its answers here rather
    /// than discovering them from a call site.
    #[test]
    fn each_purpose_states_what_it_requires() {
        use FlowPurpose::*;
        assert_eq!(
            [Run, Validate, Graph]
                .map(|p| (p.blocked_by_a_running_flow(), p.asks_about_profiles())),
            [(true, true), (false, true), (false, false)]
        );
    }

    /// A lane with no flows still opens — with nothing to pick, so Enter
    /// must not reach for a candidate that is not there.
    #[test]
    fn an_empty_lane_opens_a_picker_with_nothing_to_pick() {
        let picker = opened(FlowPurpose::Run, &[]);
        assert!(picker.is_open());
        assert!(picker.focused_pick().is_none());
    }

    /// Escape and Enter belong to the overlay, not to a list: the stop
    /// prompt has none and still has to answer both — Enter is the only way
    /// to the stop itself (`focused_pick` is `None` there, which is what
    /// `execute_flow_picker_selection` reads it by).
    ///
    /// Every list key is refused outright instead. It used to no-op silently
    /// — `Ignored` is the same non-effect said out loud, so the handler stops
    /// repainting for a keystroke that changed nothing.
    #[test]
    fn the_stop_prompt_answers_escape_and_enter_and_refuses_the_list_keys() {
        let mut picker = FlowPicker::Stopping { lane: LANE };
        assert_eq!(picker.on_key("escape", None), PickerKey::Dismiss);
        assert_eq!(picker.on_key("enter", None), PickerKey::Confirm);
        for (key, ch) in [
            ("up", None),
            ("down", None),
            ("backspace", None),
            ("s", Some('s')),
        ] {
            assert_eq!(picker.on_key(key, ch), PickerKey::Unchanged, "{key}");
        }
        // None of it turned the prompt into a list, and Enter still has
        // nothing to pick — the two halves of what makes it a stop.
        assert!(matches!(picker, FlowPicker::Stopping { .. }));
        assert!(picker.focused_pick().is_none());
    }

    /// The second question starts over. Carrying the query would filter
    /// profile names by whatever was typed to find the file, and carrying the
    /// selection would point the highlight at a row of the previous list.
    #[test]
    fn the_profile_question_starts_the_query_and_the_selection_over() {
        let mut picker = opened(FlowPurpose::Run, &["ship.yaml", "review.yaml"]);
        picker.on_key("s", Some('s'));
        picker.on_key("down", None);
        picker.ask_profile(
            LANE,
            FlowPurpose::Run,
            PathBuf::from("/lane/f/ship.yaml"),
            crate::workspace::flow_request::FlowSelection::default(),
            vec!["cheap".to_string()],
        );
        let c = picker.choosing().expect("the second question is up");
        assert_eq!(c.picker.query(), "");
        assert_eq!(c.picker.focused_index(), 0);
        assert_eq!(c.visible().len(), 2, "defaults plus the one profile");
    }
}
