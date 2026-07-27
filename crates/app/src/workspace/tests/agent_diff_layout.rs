//! Layout probe for the agent-chat tool-card diff editor: reproduces the
//! exact `diff_body` structure (wrapper reserved at `rows × row height`,
//! embedded read-only `file_viewer_editor`) and measures what actually
//! paints — `display_rows()` drift across frames and the editor's painted
//! viewport height (via its public `scroll_handle()` bounds).

use gpui::{
    AppContext as _, AvailableSpace, Context, Entity, Focusable, TestAppContext, Window, div,
    point, prelude::*, px, size,
};

use crate::test_support::init_gpui_component;
use crate::ui::theme;

/// Six display rows: a hunk header + five added lines — the shape of the
/// screenshot repro (`Write /tmp/daruda_word_diff_repro.rs`, `@@ -1,0 +1,5 @@`).
const DIFF_TEXT: &str = "@@ -1,0 +1,5 @@\nuse std::collections::{HashMap, HashSet, BTreeMap, BTreeSet};\nlet a = 1;\nlet b = 2;\nlet c = 3;\nlet d = 4;";
const DIFF_ROWS: usize = 6;

struct DiffProbe {
    editor: Entity<crate::ui::InputState>,
    focus_handle: gpui::FocusHandle,
}

impl DiffProbe {
    /// Mirrors `agent_chat_helpers::create_diff_editor` construction:
    /// multi-line, no soft wrap, code-editor mode, `rows` seeded to the
    /// display-row count, value set, then read-only.
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx_state| {
            let mut state = crate::ui::InputState::new(window, cx_state)
                .multi_line(true)
                .soft_wrap(false)
                .code_editor("rust")
                .rows(DIFF_ROWS);
            state.set_value(DIFF_TEXT, window, cx_state);
            state.set_disabled(true, cx_state);
            state
        });
        Self {
            editor,
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Focusable for DiffProbe {
    fn focus_handle(&self, _cx: &gpui::App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DiffProbe {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Mirrors `render/diff.rs::diff_body`'s editor branch: wrapper reserves
        // `rows × AGENT_CHAT_DIFF_ROW_H`, editor embedded via `code_diff_viewer`
        // (the agent-chat diff wrapper) pinned to the same height.
        let rows = self.editor.read(cx).display_rows().max(1);
        let height = px(rows as f32 * theme::AGENT_CHAT_DIFF_ROW_H);
        div().flex().flex_col().w_full().child(
            div()
                .id("diff-wrapper")
                .debug_selector(|| "diff-wrapper".into())
                .flex()
                .w_full()
                .h(height)
                .child(crate::ui::code_diff_viewer(&self.editor, cx).h(height)),
        )
    }
}

#[gpui::test]
async fn diff_editor_keeps_seeded_rows_and_paints_full_height(cx: &mut TestAppContext) {
    init_gpui_component(cx);
    let (probe, cx) = cx.add_window_view(DiffProbe::new);
    cx.run_until_parked();

    let rows_frame1 = probe.read_with(cx, |p, cx| p.editor.read(cx).display_rows());

    // Draw a second frame: the first paint runs `set_input_bounds` /
    // `set_font` on the wrapper, whose `cx.notify()` schedules a re-render —
    // any rows drift shows from frame 2 on.
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let rows_frame2 = probe.read_with(cx, |p, cx| p.editor.read(cx).display_rows());
    let wrapper = cx
        .debug_bounds("diff-wrapper")
        .expect("wrapper painted (debug bounds recorded)");
    let editor_bounds = probe.read_with(cx, |p, cx| p.editor.read(cx).last_bounds());

    assert_eq!(
        rows_frame1, DIFF_ROWS,
        "frame 1: display_rows must stay at the seeded row count"
    );
    assert_eq!(
        rows_frame2, DIFF_ROWS,
        "frame 2 (post first paint): display_rows drifted — rows seed clobbered"
    );
    let expected = px(DIFF_ROWS as f32 * theme::AGENT_CHAT_DIFF_ROW_H);
    assert_eq!(
        wrapper.size.height, expected,
        "the reserved wrapper height matches rows × row height"
    );
    let editor_h = editor_bounds
        .expect("the editor text element painted at least once")
        .size
        .height;
    assert!(
        editor_h >= expected,
        "editor paints a {editor_h:?} viewport inside a {expected:?} wrapper — collapsed"
    );
}

/// The diff editor is built read-only via `set_disabled(true)`, but
/// `Input::render` rewrites `state.disabled = self.disabled` every frame, so a
/// `file_viewer_editor` left at the builder default would clobber it back to
/// editable on the first paint. Rendering through the wrapper must keep the
/// state disabled across paints.
#[gpui::test]
async fn diff_editor_stays_read_only_across_paints(cx: &mut TestAppContext) {
    init_gpui_component(cx);
    let (probe, cx) = cx.add_window_view(DiffProbe::new);
    cx.run_until_parked();

    assert!(
        probe.read_with(cx, |p, cx| p.editor.read(cx).is_disabled()),
        "frame 1: read-only flag survived the first paint"
    );

    // A second paint runs `Input::render` again; the disabled reconciliation
    // must not flip the state back to editable.
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(
        probe.read_with(cx, |p, cx| p.editor.read(cx).is_disabled()),
        "frame 2: read-only flag clobbered by Input::render"
    );
}

/// Same structure, but laid out the way the agent-chat virtual list lays out
/// its items: `Definite(width) × MinContent` (see gpui `list.rs`
/// `available_item_space`). The windowed probe above sizes the root against a
/// definite window height; the list does not — a collapse that only happens
/// under min-content sizing shows here.
#[gpui::test]
async fn diff_editor_fills_wrapper_under_min_content_constraint(cx: &mut TestAppContext) {
    init_gpui_component(cx);
    let (probe, cx) = cx.add_window_view(DiffProbe::new);
    cx.run_until_parked();
    let editor = probe.read_with(cx, |p, _| p.editor.clone());

    cx.draw(
        point(px(0.), px(0.)),
        size(
            AvailableSpace::Definite(px(800.)),
            AvailableSpace::MinContent,
        ),
        |_, _| gpui::AnyView::from(probe.clone()),
    );

    let wrapper = cx
        .debug_bounds("diff-wrapper")
        .expect("wrapper painted under min-content sizing");
    let editor_h = editor
        .read_with(cx, |e, _| e.last_bounds())
        .expect("editor text element painted")
        .size
        .height;

    let expected = px(DIFF_ROWS as f32 * theme::AGENT_CHAT_DIFF_ROW_H);
    assert_eq!(
        wrapper.size.height, expected,
        "wrapper keeps reserved height"
    );
    assert!(
        editor_h >= expected,
        "editor paints {editor_h:?} inside a {expected:?} wrapper under min-content — collapsed"
    );
}

/// Full-pipeline repro of the screenshot bug: a `Write` tool call streams a
/// partial 1-line diff (editor built + painted from that snapshot), then the
/// final `ToolCallUpdate` replaces it with the full 5-line content. The
/// rebuilt editor must report the full display-row count and paint the full
/// reserved height — not stay clipped at the partial snapshot's size.
#[gpui::test]
async fn streaming_write_diff_rebuild_paints_full_editor(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane::PaneContent;
    use agent_client_protocol::schema::v1::{
        Content, ContentBlock, Diff, SessionUpdate, TextContent, ToolCall, ToolCallContent,
        ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
    };

    let (window_handle, workspace) = super::build_workspace(cx);
    cx.run_until_parked();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| ws.open_agent_chat_pane(window, cx));
    })
    .unwrap();
    cx.run_until_parked();

    let view = workspace.read_with(cx, |ws, _| {
        ws.active_runtime()
            .panes
            .iter()
            .find_map(|p| match &p.content {
                PaneContent::AgentChat(ac) => Some(ac.view.clone()),
                _ => None,
            })
            .expect("agent chat pane present")
    });

    const PARTIAL: &str = "use std::collections::{HashMap, HashSet, BTreeMap, BTreeSet};\n";
    const FULL: &str = "use std::collections::{HashMap, HashSet, BTreeMap, BTreeSet};\nlet a = 1;\nlet b = 2;\nlet c = 3;\nlet d = 4;\n";

    // 1. Streaming write begins: partial 1-line snapshot.
    view.update(cx, |v, cx| {
        v.apply_event(
            daruda_acp::AcpEvent::Update(Box::new(SessionUpdate::ToolCall(
                ToolCall::new("w1", "Write /tmp/daruda_word_diff_repro.rs")
                    .kind(ToolKind::Edit)
                    .content(vec![ToolCallContent::Diff(Diff::new(
                        "/tmp/daruda_word_diff_repro.rs",
                        PARTIAL,
                    ))]),
            ))),
            "",
            false,
            cx,
        );
    });
    // Paint a frame with the partial-snapshot editor (the real repro's timing:
    // the first editor has painted before the final content arrives).
    cx.run_until_parked();

    // 2. Final update: full content + completed + output text.
    view.update(cx, |v, cx| {
        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::Completed);
        fields.content = Some(vec![
            ToolCallContent::Diff(Diff::new("/tmp/daruda_word_diff_repro.rs", FULL)),
            ToolCallContent::Content(Content::new(ContentBlock::Text(TextContent::new(
                "File created successfully at: /tmp/daruda_word_diff_repro.rs",
            )))),
        ]);
        v.apply_event(
            daruda_acp::AcpEvent::Update(Box::new(SessionUpdate::ToolCallUpdate(
                ToolCallUpdate::new("w1", fields),
            ))),
            "",
            false,
            cx,
        );
    });
    cx.run_until_parked();
    // Completed flips the tool card to its collapsed default; the screenshot
    // repro has the card expanded — expand everything before measuring.
    view.update(cx, |v, cx| v.set_all_folds(true, cx));
    cx.run_until_parked();
    cx.update_window(window_handle.into(), |_, window, _| window.refresh())
        .unwrap();
    cx.run_until_parked();

    let (rows, bounds) = view.read_with(cx, |v, cx| {
        let editor = v
            .assets
            .diff_editors
            .get("w1#0")
            .expect("diff editor built for the tool call");
        (
            editor.read(cx).display_rows(),
            editor.read(cx).last_bounds(),
        )
    });

    // Full content = hunk header + 5 added lines = 6 display rows.
    assert_eq!(
        rows, 6,
        "rebuilt editor reports the full content's display rows"
    );
    let expected = px(6.0 * theme::AGENT_CHAT_DIFF_ROW_H);
    let painted = bounds.expect("editor painted after the final update").size;
    assert!(
        painted.height >= expected,
        "editor paints {painted:?} — expected at least {expected:?} (full diff visible)"
    );
    // The rounded/overflow-hidden container must not clip the diff: its
    // painted height covers the foldable header row plus the reserved body.
    // (`overflow_hidden` zeroes the container's automatic minimum size in
    // flex layout, so an over-constrained ancestor squeezes exactly this
    // node — the screenshot's clipped-diff shape.)
    let mut vcx = gpui::VisualTestContext::from_window(window_handle.into(), cx);
    let container = vcx
        .debug_bounds("agent-chat-diff-container-w1#0")
        .expect("diff container painted");
    assert!(
        container.size.height >= expected,
        "diff container is {:?} tall — clipped below its {expected:?} body",
        container.size.height
    );
}

/// The folded-diff shape of the screenshot's Edit card: an `Edit` tool call
/// whose diff the user collapses after completion. The collapsed diff block
/// must keep its full header-row height — not get squeezed by an ancestor
/// (the clipped `+1 −1` strip in the repro screenshot).
#[gpui::test]
async fn collapsed_diff_header_keeps_row_height(cx: &mut TestAppContext) {
    use crate::workspace::main_area::agent_chat_pane::fold::FoldKey;
    use crate::workspace::main_area::pane::PaneContent;
    use agent_client_protocol::schema::v1::{
        Diff, SessionUpdate, ToolCall, ToolCallContent, ToolCallStatus, ToolCallUpdate,
        ToolCallUpdateFields, ToolKind,
    };

    let (window_handle, workspace) = super::build_workspace(cx);
    cx.run_until_parked();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| ws.open_agent_chat_pane(window, cx));
    })
    .unwrap();
    cx.run_until_parked();

    let view = workspace.read_with(cx, |ws, _| {
        ws.active_runtime()
            .panes
            .iter()
            .find_map(|p| match &p.content {
                PaneContent::AgentChat(ac) => Some(ac.view.clone()),
                _ => None,
            })
            .expect("agent chat pane present")
    });

    view.update(cx, |v, cx| {
        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::Completed);
        v.apply_event(
            daruda_acp::AcpEvent::Update(Box::new(SessionUpdate::ToolCall(
                ToolCall::new("e1", "Edit /tmp/daruda_word_diff_repro.rs")
                    .kind(ToolKind::Edit)
                    .content(vec![ToolCallContent::Diff(
                        Diff::new(
                            "/tmp/daruda_word_diff_repro.rs",
                            "use std::collections::{HashMap, HashSet};\n",
                        )
                        .old_text("use std::collections::HashMap;\n"),
                    )]),
            ))),
            "",
            false,
            cx,
        );
        v.apply_event(
            daruda_acp::AcpEvent::Update(Box::new(SessionUpdate::ToolCallUpdate(
                ToolCallUpdate::new("e1", fields),
            ))),
            "",
            false,
            cx,
        );
    });
    cx.run_until_parked();
    // Expand the completed card (screenshot state), then collapse just the
    // diff — the reported clipping fold state.
    view.update(cx, |v, cx| v.set_all_folds(true, cx));
    cx.run_until_parked();
    view.update(cx, |v, cx| v.toggle_fold(FoldKey::Diff("e1#0".into()), cx));
    cx.run_until_parked();
    cx.update_window(window_handle.into(), |_, window, _| window.refresh())
        .unwrap();
    cx.run_until_parked();

    let mut vcx = gpui::VisualTestContext::from_window(window_handle.into(), cx);
    let container = vcx
        .debug_bounds("agent-chat-diff-container-e1#0")
        .expect("collapsed diff container painted");
    // The collapsed block is exactly the header row: chevron + path on the
    // hunk-bg chrome. It must be at least one text row tall — the clipped
    // repro painted it at roughly half a row.
    let min_header = px(theme::AGENT_CHAT_DIFF_ROW_H);
    assert!(
        container.size.height >= min_header,
        "collapsed diff header is {:?} tall — clipped below one row ({min_header:?})",
        container.size.height
    );
}
