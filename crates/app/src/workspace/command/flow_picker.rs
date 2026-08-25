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

use crate::fuzzy::fuzzy_match;
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
    pub stage: Stage,
    pub query: String,
    pub focused_index: usize,
}

/// What Enter acted on. Two stages, so a pick says which question it
/// answered — the host runs nothing until it holds the second.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum FlowPick {
    /// A flow. Whether a profile is asked for next is the host's to decide,
    /// because only it can read the file.
    Flow(FlowPurpose, PathBuf),
    /// A profile for the flow already picked. `None` is the file as written.
    Profile(
        FlowPurpose,
        PathBuf,
        crate::workspace::flow_request::FlowSelection,
        Option<String>,
    ),
}

#[derive(Clone, Debug, Default)]
pub(in crate::workspace) enum FlowPicker {
    #[default]
    Closed,
    Choosing(Choosing),
    /// A run already holds this lane, so the list is not the question —
    /// whether to stop it is. Derived from the lock rather than from a
    /// field, which is what lets a run started before this app launched be
    /// recognised at all.
    Stopping,
}

impl FlowPicker {
    pub fn open(
        &mut self,
        purpose: FlowPurpose,
        found: Vec<crate::workspace::flow_paths::FoundFlow>,
    ) {
        *self = FlowPicker::Choosing(Choosing {
            purpose,
            stage: Stage::Flows {
                candidates: found.into_iter().map(FlowCandidate::from_found).collect(),
            },
            query: String::new(),
            focused_index: 0,
        });
    }

    /// Ask which profile `flow` runs under. The query and the focus start
    /// over: they were about a different list, and carrying them would
    /// filter profile names by whatever was typed to find the file.
    pub fn ask_profile(
        &mut self,
        purpose: FlowPurpose,
        flow: PathBuf,
        selection: crate::workspace::flow_request::FlowSelection,
        names: Vec<String>,
    ) {
        *self = FlowPicker::Choosing(Choosing {
            purpose,
            stage: Stage::Profiles {
                flow,
                selection,
                candidates: std::iter::once(ProfileCandidate::defaults())
                    .chain(names.into_iter().map(ProfileCandidate::named))
                    .collect(),
            },
            query: String::new(),
            focused_index: 0,
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
            FlowPicker::Closed | FlowPicker::Stopping => None,
        }
    }

    fn choosing_mut(&mut self) -> Option<&mut Choosing> {
        match self {
            FlowPicker::Choosing(c) => Some(c),
            FlowPicker::Closed | FlowPicker::Stopping => None,
        }
    }

    pub fn append(&mut self, ch: char) {
        if let Some(c) = self.choosing_mut() {
            c.query.push(ch);
            c.focused_index = 0;
        }
    }

    pub fn backspace(&mut self) {
        if let Some(c) = self.choosing_mut() {
            c.query.pop();
            c.focused_index = 0;
        }
    }

    pub fn move_up(&mut self) {
        if let Some(c) = self.choosing_mut()
            && c.focused_index > 0
        {
            c.focused_index -= 1;
        }
    }

    pub fn move_down(&mut self) {
        let Some(c) = self.choosing_mut() else { return };
        let cap = c.filtered().len().min(theme::PALETTE_MAX_VISIBLE);
        if cap > 0 && c.focused_index < cap - 1 {
            c.focused_index += 1;
        }
    }

    /// Move the focus to a row the mouse named. Clicking is the same
    /// gesture as arrowing there and pressing Enter, so it goes through the
    /// same field rather than a second path to the same decision.
    pub fn focus(&mut self, index: usize) {
        if let Some(c) = self.choosing_mut() {
            c.focused_index = index;
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
        let &index = c.filtered().get(c.focused_index)?;
        match &c.stage {
            Stage::Flows { candidates } => Some(FlowPick::Flow(
                c.purpose,
                candidates.get(index)?.path.clone(),
            )),
            Stage::Profiles {
                flow,
                selection,
                candidates,
            } => Some(FlowPick::Profile(
                c.purpose,
                flow.clone(),
                selection.clone(),
                candidates.get(index)?.name.clone(),
            )),
        }
    }
}

impl Choosing {
    /// Candidate indices matching `query`, best match first. An empty
    /// query yields every candidate in original order.
    pub fn filtered(&self) -> Vec<usize> {
        fuzzy_match(&self.query, &self.stage.labels())
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

        // Both open states are the same panel over a different list: the
        // flows to pick from, or the single thing there is to do about a
        // run that is already going.
        let (prompt, rows) = match &self.state {
            FlowPicker::Closed => unreachable!("returned above"),
            FlowPicker::Stopping => (
                self.stop_prompt.clone(),
                vec![Row {
                    label: self.stop_action.clone(),
                    tag: None,
                }],
            ),
            FlowPicker::Choosing(state) => (
                if state.query.is_empty() {
                    self.prompt.clone()
                } else {
                    SharedString::from(state.query.clone())
                },
                state
                    .filtered()
                    .iter()
                    .take(theme::PALETTE_MAX_VISIBLE)
                    .filter_map(|&i| state.stage.row(i))
                    .collect(),
            ),
        };
        let focused_index = self.state.choosing().map_or(0, |c| c.focused_index);

        let input = div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .px(px(theme::PALETTE_INPUT_PAD_X))
            .py(px(theme::PALETTE_INPUT_PAD_Y))
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .text_size(px(theme::PALETTE_QUERY_FONT_SIZE))
                    .text_color(t.text_primary)
                    .child(prompt),
            );

        let entries = div()
            .flex()
            .flex_col()
            .max_h(px(theme::PALETTE_MAX_HEIGHT))
            .overflow_hidden()
            .children(rows.iter().enumerate().map(|(index, row)| {
                let is_focused = index == focused_index;
                let on_pick = self.on_pick.clone();
                div()
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        on_pick(&index, window, cx);
                    })
                    .hover(|d| d.bg(t.palette_focused_bg))
                    .flex()
                    .flex_row()
                    .items_center()
                    .w_full()
                    .px(px(theme::PALETTE_ENTRY_PAD_X))
                    .py(px(theme::PALETTE_ENTRY_PAD_Y))
                    .text_size(px(theme::PALETTE_ENTRY_FONT_SIZE))
                    // Reserve the same-width transparent border on unfocused
                    // rows so the label does not shift when the accent rule
                    // appears — same idiom as the lane rows in the left dock.
                    .border_l(px(theme::PALETTE_FOCUS_BORDER_W))
                    .border_color(theme::TRANSPARENT)
                    .when(is_focused, |d| {
                        d.bg(t.palette_focused_bg)
                            .text_color(t.text_primary)
                            .border_color(theme::PRIMARY)
                    })
                    .when(!is_focused, |d| d.text_color(t.text_body))
                    .child(div().flex_1().min_w_0().truncate().child(row.label.clone()))
                    .children(row.tag.clone().map(|tag| {
                        div()
                            .flex_none()
                            .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
                            .text_color(t.text_subtle)
                            .child(tag)
                    }))
            }));

        let no_results = rows.is_empty().then(|| {
            div()
                .px(px(theme::PALETTE_ENTRY_PAD_X))
                .py(px(theme::PALETTE_EMPTY_PAD_Y))
                .text_size(px(theme::PALETTE_ENTRY_FONT_SIZE))
                .text_color(t.text_subtle)
                .child(self.empty.clone())
        });

        let panel = div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .mx_auto()
            .mt(px(theme::PALETTE_TOP_OFFSET))
            .w(px(theme::PALETTE_WIDTH))
            .bg(t.palette_bg)
            .border_1()
            .border_color(t.border)
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

    fn opened(purpose: FlowPurpose, names: &[&str]) -> FlowPicker {
        let mut picker = FlowPicker::default();
        picker.open(
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
        let FlowPick::Flow(purpose, path) = picker.focused_pick().expect("a pick") else {
            panic!("the first question is which flow");
        };
        assert_eq!(purpose, FlowPurpose::Validate);
        assert!(path.ends_with("ship.yaml"));
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
        picker.move_down();
        let FlowPick::Flow(_, path) = picker.focused_pick().expect("a pick") else {
            panic!("the first question is which flow");
        };
        assert!(path.ends_with("02-broken.yaml"), "picked {path:?}");
    }

    /// A closed picker has no query, no selection and no purpose — the
    /// reason this is an enum and not a flag beside four fields.
    #[test]
    fn a_closed_picker_holds_nothing() {
        let mut picker = opened(FlowPurpose::Run, &["ship.yaml"]);
        picker.append('s');
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
        picker.append('r');
        picker.append('v');
        let FlowPick::Flow(_, path) = picker.focused_pick().expect("a pick") else {
            panic!("the first question is which flow");
        };
        assert!(path.ends_with("review.yaml"), "{path:?}");
    }

    /// Moving down cannot walk off the end of what is actually drawn.
    #[test]
    fn the_selection_stays_inside_the_visible_list() {
        let mut picker = opened(FlowPurpose::Run, &["a.yaml", "b.yaml"]);
        for _ in 0..5 {
            picker.move_down();
        }
        let FlowPick::Flow(_, path) = picker.focused_pick().expect("a pick") else {
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
}
