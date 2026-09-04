//! Layout probe for the agent-chat tool-card diff editor: reproduces the exact
//! `diff_body` structure (wrapper and `Input` both pinned to
//! `bounded_embed_height`, embedded read-only `embedded_code_viewer`) and
//! measures what actually paints — `display_rows()` drift across frames, the
//! painted wrapper height, and the row range the editor really shaped
//! (`visible_rows()`).
//!
//! A whole-file `Write` diff is as long as the file, so `visible_rows()` guards
//! it from both sides exactly as it does the output embed: too many rows means
//! the height bound is gone and `calculate_visible_range` counts every row as
//! visible again (the linear paint cost), too few means the editor collapsed to
//! its one-row minimum inside the reserved box.

use gpui::{
    AppContext as _, AvailableSpace, Context, Entity, Focusable, TestAppContext, Window, div,
    point, prelude::*, px, size,
};

use crate::test_support::init_gpui_component;
use crate::ui::theme;
use crate::workspace::main_area::agent_chat_pane::output_editor::bounded_embed_height;

/// Six display rows: a hunk header + five added lines — the shape of the
/// screenshot repro (`Write /tmp/daruda_word_diff_repro.rs`, `@@ -1,0 +1,5 @@`).
const DIFF_TEXT: &str = "@@ -1,0 +1,5 @@\nuse std::collections::{HashMap, HashSet, BTreeMap, BTreeSet};\nlet a = 1;\nlet b = 2;\nlet c = 3;\nlet d = 4;";
const DIFF_ROWS: usize = 6;
/// Far more rows than the cap can show — a whole-file `Write` of a large file.
const LARGE_ROWS: usize = 400;

/// How many rows the cap can actually show.
fn capped_rows() -> usize {
    theme::AGENT_CHAT_DIFF_EMBED_MAX_ROWS
}

fn default_row_height() -> f32 {
    (theme::AGENT_CHAT_MSG_FONT_SIZE * theme::MD_VIEW_LINE_HEIGHT).ceil()
}

/// The row count `calculate_visible_range` may report at most: a full viewport
/// plus its `extra_rows = 1` and the row that straddles the bottom edge.
fn max_visible_rows() -> usize {
    capped_rows() + 2
}

struct DiffProbe {
    editor: Entity<crate::ui::InputState>,
    focus_handle: gpui::FocusHandle,
}

impl DiffProbe {
    /// Mirrors `agent_chat_helpers::create_diff_editor` construction:
    /// multi-line, no soft wrap, code-editor mode, `rows` seeded to the
    /// display-row count, value set, then read-only.
    fn new(text: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let rows = text.lines().count().max(1);
        let text = text.to_owned();
        let editor = cx.new(|cx_state| {
            let mut state = crate::ui::InputState::new(window, cx_state)
                .multi_line(true)
                .soft_wrap(false)
                .code_editor("rust")
                .rows(rows);
            state.set_value(text, window, cx_state);
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
        // Mirrors `render/embed.rs::bounded_editor_embed`, which
        // `render/diff.rs::diff_body` embeds the diff through: the capped height
        // goes on both the wrapper and the `Input`.
        let rows = self.editor.read(cx).display_rows().max(1);
        let height = bounded_embed_height(
            rows,
            theme::AGENT_CHAT_DIFF_EMBED_MAX_ROWS,
            theme::agent_chat_embed_row_height(cx),
        );
        let surface = crate::ui::theme::agent_chat_bg(cx);
        div().flex().flex_col().w_full().child(
            div()
                .id("diff-wrapper")
                .debug_selector(|| "diff-wrapper".into())
                .flex()
                .w_full()
                .flex_none()
                .h(height)
                .child(crate::ui::embedded_code_viewer(&self.editor, surface, cx).h(height)),
        )
    }
}

#[gpui::test]
async fn diff_editor_keeps_seeded_rows_and_paints_full_height(cx: &mut TestAppContext) {
    init_gpui_component(cx);
    let (probe, cx) = cx.add_window_view(|window, cx| DiffProbe::new(DIFF_TEXT, window, cx));
    cx.run_until_parked();

    let rows_frame1 = probe.read_with(cx, |p, cx| p.editor.read(cx).display_rows());
    assert!(
        probe.read_with(cx, |p, cx| p.editor.read(cx).is_disabled()),
        "frame 1: read-only flag survived the first paint"
    );

    // Draw a second frame: the first paint runs `set_input_bounds` /
    // `set_font` on the wrapper, whose `cx.notify()` schedules a re-render —
    // any rows drift shows from frame 2 on. `Input::render` also reconciles
    // disabled state every frame, so this covers the read-only regression too.
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let rows_frame2 = probe.read_with(cx, |p, cx| p.editor.read(cx).display_rows());
    assert!(
        probe.read_with(cx, |p, cx| p.editor.read(cx).is_disabled()),
        "frame 2: read-only flag clobbered by Input::render"
    );
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
    assert!(
        DIFF_ROWS < capped_rows(),
        "the six-row case must stay under the cap"
    );
    let expected = bounded_embed_height(
        DIFF_ROWS,
        theme::AGENT_CHAT_DIFF_EMBED_MAX_ROWS,
        default_row_height(),
    );
    assert_eq!(
        wrapper.size.height, expected,
        "an uncapped diff measures its content plus the thumb strip"
    );
    let editor_h = editor_bounds
        .expect("the editor text element painted at least once")
        .size
        .height;
    assert!(
        editor_h >= expected,
        "editor paints a {editor_h:?} viewport inside a {expected:?} wrapper — collapsed"
    );

    // The agent-chat virtual list lays items out as
    // `Definite(width) × MinContent` (see gpui `list.rs`
    // `available_item_space`). The windowed probe above sizes the root against
    // a definite window height; the list does not, so a collapse that only
    // happens under min-content sizing must show here too.
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
/// reserved height — not stay clipped at the partial snapshot's size. The same
/// pane then covers the folded-header shape of an `Edit` card so both
/// screenshot regressions share one workspace fixture.
#[gpui::test]
async fn streaming_write_diff_rebuilds_and_collapsed_header_keeps_height(cx: &mut TestAppContext) {
    use crate::workspace::main_area::agent_chat_pane::fold::FoldKey;
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
    fold_in_window(cx, window_handle.into(), &view, |v, window, cx| {
        v.set_all_folds(true, window, cx)
    });
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
    let expected = px(6.0 * default_row_height());
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

    drop(vcx);
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
    fold_in_window(cx, window_handle.into(), &view, |v, window, cx| {
        v.set_all_folds(true, window, cx)
    });
    cx.run_until_parked();
    fold_in_window(cx, window_handle.into(), &view, |v, window, cx| {
        v.toggle_fold(FoldKey::Diff("e1#0".into()), window, cx)
    });
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
    let min_header = px(default_row_height());
    assert!(
        container.size.height >= min_header,
        "collapsed diff header is {:?} tall — clipped below one row ({min_header:?})",
        container.size.height
    );
}

/// Full pipeline, not the probe: a `Write` of a large file produces a whole-file
/// diff (gutter line numbers on — `build_diff_view_model` enables them exactly
/// when `old_text` is absent), and `diff_body` must embed it at the cap. The
/// probe above proves the shape is bounded; this proves the render actually uses
/// it, decorations and all.
#[gpui::test]
async fn a_large_write_diff_renders_through_the_capped_embed(cx: &mut TestAppContext) {
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

    let mut new_text = String::new();
    for n in 1..=LARGE_ROWS {
        new_text.push_str(&format!("let v{n} = {n};\n"));
    }
    view.update(cx, |v, cx| {
        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::Completed);
        v.apply_event(
            daruda_acp::AcpEvent::Update(Box::new(SessionUpdate::ToolCall(
                ToolCall::new("w1", "Write /tmp/daruda_large_write.rs")
                    .kind(ToolKind::Edit)
                    .content(vec![ToolCallContent::Diff(Diff::new(
                        "/tmp/daruda_large_write.rs",
                        new_text,
                    ))]),
            ))),
            "",
            false,
            cx,
        );
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
    // Completed collapses the card by default; the embed only renders expanded.
    fold_in_window(cx, window_handle.into(), &view, |v, window, cx| {
        v.set_all_folds(true, window, cx)
    });
    cx.run_until_parked();
    cx.update_window(window_handle.into(), |_, window, _| window.refresh())
        .unwrap();
    cx.run_until_parked();

    let (rows, visible, scroll_h) = view.read_with(cx, |v, cx| {
        let state = v
            .assets
            .diff_editors
            .get("w1#0")
            .expect("diff editor built for the write")
            .read(cx);
        (
            state.display_rows(),
            state.visible_rows(),
            state.scroll_size().height,
        )
    });

    let mut vcx = gpui::VisualTestContext::from_window(window_handle.into(), cx);
    let embed = vcx
        .debug_bounds("agent-chat-out-embed-diff-w1#0")
        .expect("the bounded embed painted — diff_body did not embed the editor");

    assert!(
        rows > capped_rows(),
        "the write's {rows}-row diff must exceed the {}-row cap",
        capped_rows()
    );
    assert_eq!(
        embed.size.height,
        bounded_embed_height(
            rows,
            theme::AGENT_CHAT_DIFF_EMBED_MAX_ROWS,
            default_row_height(),
        ),
        "the diff embed in a real card is capped"
    );
    let visible = visible.expect("the embedded editor painted");
    assert!(
        visible.len() <= max_visible_rows(),
        "the real card's diff embed shaped {} of {rows} rows",
        visible.len()
    );
    assert!(
        visible.len() >= capped_rows(),
        "the real card's diff embed shaped only {} rows — expected a full {}-row \
         viewport",
        visible.len(),
        capped_rows()
    );
    assert!(
        scroll_h >= px(rows as f32 * default_row_height()),
        "scroll extent {scroll_h:?} does not cover the full diff — rows lost"
    );
}

/// Drive a fold the way the app does — from inside the window's own update
/// cycle, where `cx.listener` handlers run. Materializing a card's embed
/// editors needs that live `Window`; resolving one by handle there fails.
fn fold_in_window<R>(
    cx: &mut TestAppContext,
    window_handle: gpui::AnyWindowHandle,
    view: &gpui::Entity<crate::workspace::main_area::agent_chat_pane::view::AgentChatView>,
    f: impl FnOnce(
        &mut crate::workspace::main_area::agent_chat_pane::view::AgentChatView,
        &mut gpui::Window,
        &mut gpui::Context<crate::workspace::main_area::agent_chat_pane::view::AgentChatView>,
    ) -> R,
) -> R {
    cx.update_window(window_handle, |_, window, cx| {
        view.update(cx, |v, cx| f(v, window, cx))
    })
    .expect("the window is open")
}
