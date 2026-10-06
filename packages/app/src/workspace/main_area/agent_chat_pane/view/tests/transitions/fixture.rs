//! Drives production handlers and compares their retained list with a cold list.

use agent_client_protocol::schema::v1::{
    SessionUpdate, ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};
use daruda_acp::{AcpEvent, ChatItem};
use gpui::{
    EntityId, FollowMode, ListAlignment, ListOffset, ListState, Pixels, TestAppContext,
    WindowHandle, px, size,
};

use super::super::super::{AgentChatView, AgentSessionStatus};
use super::super::{FoldKey, RowKind, add_test_view};
use super::observation::{Snapshot, snapshot};

const WINDOW_WIDTH: f32 = 800.;
const WINDOW_HEIGHT: f32 = 460.;
const INITIAL_LINES: usize = 3;
const TOOL_COUNT: usize = 12;

#[derive(Clone, Copy, Debug)]
pub(super) enum Step {
    Create(usize),
    Output { tool: usize, lines: usize },
    ExpandAll,
    Toggle(usize),
    Scroll { tool: usize, offset: f32 },
}

fn tool_id(tool: usize) -> String {
    format!("transition-{tool}")
}

fn tool_row(view: &AgentChatView, tool: usize) -> usize {
    let id = tool_id(tool);
    view.rows
        .iter()
        .position(|row| match row.kind {
            RowKind::AgentItem(ix) => {
                matches!(&view.items[ix], ChatItem::ToolCall(call) if call.id == id)
            }
            _ => false,
        })
        .expect("tool owns a projected row")
}

fn apply(window: WindowHandle<AgentChatView>, step: Step, cx: &mut TestAppContext) {
    let update = match step {
        Step::Create(tool) => SessionUpdate::ToolCall(
            ToolCall::new(tool_id(tool), "Shell output").kind(ToolKind::Execute),
        ),
        Step::Output { tool, lines } => {
            let text = (0..lines)
                .map(|line| format!("output {tool}: {line}\n"))
                .collect::<String>();
            let mut fields = ToolCallUpdateFields::default();
            fields.status = Some(ToolCallStatus::Completed);
            // Explicit replacement clears the previous body before raw fallback.
            fields.content = Some(Vec::new());
            fields.raw_output = Some(serde_json::json!({
                "formatted_output": text,
                "exit_code": 0,
            }));
            SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(tool_id(tool), fields))
        }
        Step::ExpandAll => {
            window
                .update(cx, |view, window, cx| view.set_all_folds(true, window, cx))
                .unwrap();
            return;
        }
        Step::Toggle(tool) => {
            window
                .update(cx, |view, window, cx| {
                    view.toggle_fold(FoldKey::Tool(tool_id(tool)), window, cx)
                })
                .unwrap();
            return;
        }
        Step::Scroll { tool, offset } => {
            window
                .update(cx, |view, _, cx| {
                    view.list_state.scroll_to(ListOffset {
                        item_ix: tool_row(view, tool),
                        offset_in_item: px(offset),
                    });
                    cx.notify();
                })
                .unwrap();
            return;
        }
    };
    // ACP delivery happens outside a window update; the handler resolves
    // that window itself when building editors.
    window.root(cx).unwrap().update(cx, |view, cx| {
        view.apply_event(AcpEvent::Update(Box::new(update)), "", false, cx);
    });
}

fn new_window(cx: &mut TestAppContext) -> WindowHandle<AgentChatView> {
    let handle = add_test_view(cx);
    handle
        .update(cx, |view, _, _| {
            view.agent_id = "codex-acp".into();
            view.status = AgentSessionStatus::Connected;
        })
        .expect("offline view created");
    cx.simulate_window_resize(handle.into(), size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)));
    handle
}

pub(super) struct Fixture {
    window: WindowHandle<AgentChatView>,
    steps: Vec<Step>,
}

impl Fixture {
    pub fn new(cx: &mut TestAppContext) -> Self {
        crate::test_support::init_gpui_component(cx);
        let mut fixture = Self {
            window: new_window(cx),
            steps: Vec::new(),
        };
        for tool in 0..TOOL_COUNT {
            fixture.step(Step::Create(tool), cx);
            fixture.step(
                Step::Output {
                    tool,
                    lines: INITIAL_LINES,
                },
                cx,
            );
        }
        fixture.step(Step::ExpandAll, cx);
        cx.run_until_parked();
        fixture
            .window
            .read_with(cx, |view, _| {
                assert!(view.list_state.max_offset_for_scrollbar().y > px(0.));
                assert_eq!(view.assets.output_editors.len(), TOOL_COUNT);
            })
            .unwrap();
        fixture
    }

    pub fn step(&mut self, step: Step, cx: &mut TestAppContext) {
        self.steps.push(step);
        apply(self.window, step, cx);
        if let Step::Output { tool, lines } = step {
            self.window
                .read_with(cx, |view, cx| {
                    if let Some(editor) = view
                        .assets
                        .output_editors
                        .get(&format!("{}#0", tool_id(tool)))
                    {
                        assert_eq!(
                            editor.read(cx).display_rows(),
                            lines,
                            "updated editor rows after {step:?}"
                        );
                    }
                })
                .unwrap();
        }
    }

    pub fn snapshot(&self, cx: &TestAppContext) -> Snapshot {
        snapshot(self.window, cx)
    }

    pub fn tool_height(&self, tool: usize, cx: &TestAppContext) -> Pixels {
        self.window
            .read_with(cx, |view, _| {
                let row = tool_row(view, tool);
                assert!(!view.rows[row].hidden, "target tool must not be hidden");
                let bounds = view
                    .list_state
                    .bounds_for_item(row)
                    .expect("target tool must be measured");
                let viewport = view.list_state.viewport_bounds();
                assert!(
                    bounds.bottom() > viewport.top() && bounds.top() < viewport.bottom(),
                    "target tool must be in the viewport"
                );
                assert!(
                    bounds.size.height > px(0.),
                    "target card has positive height"
                );
                bounds.size.height
            })
            .unwrap()
    }

    pub fn assert_tool_above_viewport(&self, tool: usize, cx: &TestAppContext) {
        self.window
            .read_with(cx, |view, _| {
                assert!(
                    tool_row(view, tool) < view.list_state.logical_scroll_top().item_ix,
                    "target tool must have left the viewport before its update"
                );
            })
            .unwrap();
    }

    pub fn close(self, cx: &mut TestAppContext) {
        self.window
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
        cx.run_until_parked();
    }

    pub fn editor_id(&self, tool: usize, cx: &TestAppContext) -> EntityId {
        self.window
            .read_with(cx, |view, _| {
                view.assets.output_editors[&format!("{}#0", tool_id(tool))].entity_id()
            })
            .unwrap()
    }

    #[track_caller]
    pub fn assert_matches_fresh(&self, cx: &mut TestAppContext) {
        let actual = self.snapshot(cx);
        let fresh = new_window(cx);
        for step in &self.steps {
            apply(fresh, *step, cx);
        }
        // Replaying reconstructs logical state, but no retained measurements
        // may enter the reference. ListState::clone would share the same cache.
        fresh
            .update(cx, |view, _, cx| {
                view.list_state = ListState::new(view.rows.len(), ListAlignment::Top, px(512.));
                if actual.following {
                    view.list_state.set_follow_mode(FollowMode::Tail);
                } else {
                    view.list_state.scroll_to(actual.scroll);
                }
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        let expected = snapshot(fresh, cx);
        fresh
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();

        actual.assert_matches(&expected, &format!("steps: {:?}", self.steps));
    }
}
