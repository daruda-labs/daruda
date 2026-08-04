//! Layout probe for the agent-chat tool-card output editor: renders the shipped
//! `render/embed.rs::bounded_editor_embed` around a read-only editor built the
//! way `output_editor::create_output_editor` builds one, and measures what
//! actually paints — the painted embed height, `display_rows()` drift across
//! frames, and the row range the editor really shaped (`visible_rows()`).
//!
//! `visible_rows()` is the regression guard for the whole bounded-embed fix, and
//! it is asserted from **both** sides. Too many rows means the height bound is
//! gone and `calculate_visible_range` counts every row as visible again — the
//! linear paint cost, i.e. the 99%-CPU defect. Too few means the editor
//! collapsed to its one-row minimum inside the reserved box, so the output is
//! present but unreadable. Only the band between them is the intended behaviour.

use gpui::{
    AppContext as _, AvailableSpace, Context, Entity, Focusable, TestAppContext, Window, div,
    point, prelude::*, px, size,
};

use crate::test_support::init_gpui_component;
use crate::ui::theme;
use crate::workspace::main_area::agent_chat_pane::output_editor::bounded_embed_height;
use crate::workspace::main_area::agent_chat_pane::render::embed::bounded_editor_embed;

/// Far more rows than the cap can show, standing in for `seq 1 200000`'s
/// 6-bytes-per-line output that fits inside the 64 KiB byte cap.
const LARGE_ROWS: usize = 5_000;
/// Fewer rows than the cap, so the embed measures its content instead.
const SMALL_ROWS: usize = 5;
/// Enough numbered lines that the fenced block around them exceeds
/// `daruda_acp`'s 64 KiB cap (109 KiB: 88,894 digits + 19,999 newlines + the two
/// fence lines), so the block reaches the render truncated from the tail.
const TRUNCATING_ROWS: usize = 20_000;

/// `id` handed to [`bounded_editor_embed`], and the debug selector it derives.
/// `debug_bounds` takes a `&'static str`, so the derived name is spelled out.
const PROBE_ID: &str = "probe";
const PROBE_EMBED: &str = "agent-chat-out-embed-probe";

/// How many rows the cap can actually show.
fn capped_rows() -> usize {
    (theme::AGENT_CHAT_EMBED_MAX_H / theme::AGENT_CHAT_EMBED_ROW_H) as usize
}

/// The row count `calculate_visible_range` may report at most: a full viewport
/// plus its `extra_rows = 1` and the row that straddles the bottom edge.
fn max_visible_rows() -> usize {
    capped_rows() + 2
}

fn numbered_lines(rows: usize) -> String {
    (1..=rows)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

struct OutputProbe {
    editor: Entity<crate::ui::InputState>,
    focus_handle: gpui::FocusHandle,
}

impl OutputProbe {
    /// Mirrors `output_editor::create_output_editor`: multi-line, no soft wrap,
    /// code-editor mode with no gutter, value set, then read-only.
    fn new(rows: usize, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = numbered_lines(rows);
        let editor = cx.new(|cx_state| {
            let mut state = crate::ui::InputState::new(window, cx_state)
                .multi_line(true)
                .soft_wrap(false)
                .code_editor(crate::ui::highlighter::PLAIN_LANGUAGE)
                .line_number(false);
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

impl Focusable for OutputProbe {
    fn focus_handle(&self, _cx: &gpui::App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for OutputProbe {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The shipped builder, not a copy of it: a bound lost inside
        // `bounded_editor_embed` has to fail these probes.
        let t = theme::current(cx).clone();
        div().flex().flex_col().w_full().child(bounded_editor_embed(
            PROBE_ID,
            &self.editor,
            None,
            &t,
            0.,
            cx,
        ))
    }
}

#[gpui::test]
async fn large_output_embed_is_capped_and_shapes_only_visible_rows(cx: &mut TestAppContext) {
    init_gpui_component(cx);
    let (probe, cx) = cx.add_window_view(|window, cx| OutputProbe::new(LARGE_ROWS, window, cx));
    cx.run_until_parked();

    let rows_frame1 = probe.read_with(cx, |p, cx| p.editor.read(cx).display_rows());

    // The first paint runs `set_input_bounds` / `set_font`, whose `cx.notify()`
    // schedules a re-render — any rows drift shows from frame 2 on.
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let (rows_frame2, visible, scroll_h, editor_h) = probe.read_with(cx, |p, cx| {
        let state = p.editor.read(cx);
        (
            state.display_rows(),
            state.visible_rows(),
            state.scroll_size().height,
            state.last_bounds().map(|b| b.size.height),
        )
    });
    let wrapper = cx
        .debug_bounds(PROBE_EMBED)
        .expect("wrapper painted (debug bounds recorded)");

    assert_eq!(
        rows_frame1, LARGE_ROWS,
        "frame 1: display_rows must report the full content"
    );
    assert_eq!(
        rows_frame2, LARGE_ROWS,
        "frame 2 (post first paint): display_rows drifted"
    );
    // Pinned at the cap, not `rows × row height` — the embed's height must not
    // grow with the output.
    assert_eq!(
        wrapper.size.height,
        bounded_embed_height(LARGE_ROWS),
        "the embed is pinned at the cap plus the thumb strip"
    );
    assert!(
        wrapper.size.height < px(LARGE_ROWS as f32 * theme::AGENT_CHAT_EMBED_ROW_H),
        "a capped embed must be far shorter than its uncapped content height"
    );

    let editor_h = editor_h.expect("the editor text element painted at least once");
    assert!(
        editor_h >= px(theme::AGENT_CHAT_EMBED_MAX_H),
        "editor paints only {editor_h:?} inside a {:?} embed — collapsed",
        wrapper.size.height
    );

    let visible = visible.expect("the editor text element painted at least once");
    assert!(
        visible.len() <= max_visible_rows(),
        "editor shaped {} of {LARGE_ROWS} rows — expected at most {}; the height \
         bound is gone and every row counts as visible again",
        visible.len(),
        max_visible_rows()
    );
    assert!(
        visible.len() >= capped_rows(),
        "editor shaped only {} rows — expected a full {}-row viewport",
        visible.len(),
        capped_rows()
    );
    // The hidden rows are reachable by scrolling, not dropped.
    assert!(
        scroll_h >= px(LARGE_ROWS as f32 * theme::AGENT_CHAT_EMBED_ROW_H),
        "scroll extent {scroll_h:?} does not cover the full content — rows lost"
    );
}

/// Same structure, laid out the way the agent-chat virtual list lays out its
/// items: `Definite(width) × MinContent` (gpui `list.rs` `available_item_space`).
/// The windowed probe above sizes the root against a definite window height; the
/// list does not, so a height that only resolves against a definite ancestor
/// collapses here.
#[gpui::test]
async fn large_output_embed_fills_the_cap_under_min_content_constraint(cx: &mut TestAppContext) {
    init_gpui_component(cx);
    let (probe, cx) = cx.add_window_view(|window, cx| OutputProbe::new(LARGE_ROWS, window, cx));
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
        .debug_bounds(PROBE_EMBED)
        .expect("wrapper painted under min-content sizing");
    let (editor_h, visible) = editor.read_with(cx, |e, _| (e.last_bounds(), e.visible_rows()));
    let editor_h = editor_h.expect("editor text element painted").size.height;
    let visible = visible.expect("editor text element painted");

    assert_eq!(
        wrapper.size.height,
        bounded_embed_height(LARGE_ROWS),
        "wrapper keeps the capped height under min-content"
    );
    assert!(
        editor_h >= px(theme::AGENT_CHAT_EMBED_MAX_H),
        "editor paints {editor_h:?} inside a {:?} embed under min-content — collapsed",
        wrapper.size.height
    );
    assert!(
        visible.len() >= capped_rows() && visible.len() <= max_visible_rows(),
        "editor shaped {} rows under min-content — expected about {}",
        visible.len(),
        capped_rows()
    );
}

#[gpui::test]
async fn small_output_embed_measures_its_content_and_shows_every_row(cx: &mut TestAppContext) {
    init_gpui_component(cx);
    let (probe, cx) = cx.add_window_view(|window, cx| OutputProbe::new(SMALL_ROWS, window, cx));
    cx.run_until_parked();

    let rows_frame1 = probe.read_with(cx, |p, cx| p.editor.read(cx).display_rows());
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let (rows_frame2, visible) = probe.read_with(cx, |p, cx| {
        let state = p.editor.read(cx);
        (state.display_rows(), state.visible_rows())
    });
    let wrapper = cx
        .debug_bounds(PROBE_EMBED)
        .expect("wrapper painted (debug bounds recorded)");

    assert_eq!(rows_frame1, SMALL_ROWS, "frame 1: display_rows");
    assert_eq!(
        rows_frame2, SMALL_ROWS,
        "frame 2 (post first paint): display_rows drifted"
    );
    assert!(
        SMALL_ROWS < capped_rows(),
        "the small case must stay under the cap"
    );
    assert_eq!(
        wrapper.size.height,
        bounded_embed_height(SMALL_ROWS),
        "an uncapped embed measures its content plus the thumb strip"
    );

    let visible = visible.expect("the editor text element painted at least once");
    assert!(
        visible.start == 0 && visible.end >= SMALL_ROWS,
        "an uncapped embed must shape all {SMALL_ROWS} rows, shaped {visible:?}"
    );
}

/// Drive a real agent-chat pane to one completed, expanded tool call whose only
/// output block is a bare fence of `rows` numbered lines — the shape the Claude
/// adapter's `markdownEscape` produces for shell output. Returns the window and
/// the chat view. Painted twice: the embed's thumbs read the *previous* paint's
/// editor geometry, so a single frame never has any.
fn render_fenced_output_card(
    cx: &mut TestAppContext,
    rows: usize,
) -> (
    gpui::AnyWindowHandle,
    Entity<crate::workspace::main_area::agent_chat_pane::view::AgentChatView>,
) {
    use crate::workspace::main_area::pane::PaneContent;
    use agent_client_protocol::schema::v1::{
        Content, ContentBlock, SessionUpdate, TextContent, ToolCall, ToolCallContent,
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

    let fenced = format!("```\n{}\n```", numbered_lines(rows));
    view.update(cx, |v, cx| {
        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::Completed);
        fields.content = Some(vec![ToolCallContent::Content(Content::new(
            ContentBlock::Text(TextContent::new(fenced)),
        ))]);
        v.apply_event(
            daruda_acp::AcpEvent::Update(Box::new(SessionUpdate::ToolCall(
                ToolCall::new("b1", "Bash seq").kind(ToolKind::Execute),
            ))),
            "",
            false,
            cx,
        );
        v.apply_event(
            daruda_acp::AcpEvent::Update(Box::new(SessionUpdate::ToolCallUpdate(
                ToolCallUpdate::new("b1", fields),
            ))),
            "",
            false,
            cx,
        );
    });
    cx.run_until_parked();
    // Completed collapses the card by default; the embed only renders expanded.
    view.update(cx, |v, cx| v.set_all_folds(true, cx));
    cx.run_until_parked();
    cx.update_window(window_handle.into(), |_, window, _| window.refresh())
        .unwrap();
    cx.run_until_parked();
    (window_handle.into(), view)
}

/// The embedded editor's painted viewport height and its `scroll_size()` height.
fn output_editor_extents(
    cx: &mut TestAppContext,
    view: &Entity<crate::workspace::main_area::agent_chat_pane::view::AgentChatView>,
) -> (gpui::Pixels, gpui::Pixels) {
    view.read_with(cx, |v, cx| {
        let state = v
            .assets
            .output_editors
            .get("b1#0")
            .expect("output editor built for the bare-fence block")
            .read(cx);
        (
            state
                .last_bounds()
                .expect("the embedded editor painted")
                .size
                .height,
            state.scroll_size().height,
        )
    })
}

/// Full pipeline, not the probe: a real tool call whose output is one bare fence
/// must render through the capped embed rather than the markdown fallback — the
/// wiring `output_block_view` adds between the reconciler's cache and the render.
/// Capped means rows are hidden, so the vertical thumb must say so.
#[gpui::test]
async fn a_bare_fence_output_block_renders_through_the_capped_embed(cx: &mut TestAppContext) {
    let (window_handle, view) = render_fenced_output_card(cx, LARGE_ROWS);

    let visible = view.read_with(cx, |v, cx| {
        v.assets
            .output_editors
            .get("b1#0")
            .expect("output editor built for the bare-fence block")
            .read(cx)
            .visible_rows()
    });

    let mut vcx = gpui::VisualTestContext::from_window(window_handle, cx);
    let embed = vcx
        .debug_bounds("agent-chat-out-embed-b1#0")
        .expect("the bounded embed painted — output_block_view fell back to markdown");
    assert_eq!(
        embed.size.height,
        bounded_embed_height(LARGE_ROWS),
        "the embed in a real card is capped"
    );
    let thumb = vcx
        .debug_bounds("agent-chat-out-vthumb-b1#0")
        .expect("the cap hid rows, so the vertical thumb must be drawn");
    assert!(
        thumb.size.height < embed.size.height,
        "a {LARGE_ROWS}-row output's thumb must be a fraction of the track, got {thumb:?}"
    );
    let visible = visible.expect("the embedded editor painted");
    assert!(
        visible.len() <= max_visible_rows(),
        "the real card's embed shaped {} of {LARGE_ROWS} rows",
        visible.len()
    );
    assert!(
        visible.len() >= capped_rows(),
        "the real card's embed shaped only {} rows — expected a full {}-row \
         viewport, so the editor collapsed inside its reserved box",
        visible.len(),
        capped_rows()
    );
}

/// A fenced block over `daruda_acp`'s 64 KiB cap is cut from the tail, so it
/// arrives with an opening fence and **no closing fence** — the shape the largest
/// outputs always have, and the ones the bounded embed exists for. It must still
/// reach the embed instead of the markdown path that paints every line.
#[gpui::test]
async fn a_truncated_output_block_still_renders_through_the_capped_embed(cx: &mut TestAppContext) {
    let (window_handle, view) = render_fenced_output_card(cx, TRUNCATING_ROWS);

    let (truncated_from, rows, visible) = view.read_with(cx, |v, cx| {
        let block = v
            .items
            .iter()
            .find_map(|item| match item {
                daruda_acp::ChatItem::ToolCall(tc) => tc.output.first(),
                _ => None,
            })
            .expect("the tool call carries one output block");
        let truncated_from = match block {
            daruda_acp::ToolOutputBlock::Text { truncated_from, .. } => *truncated_from,
            _ => panic!("a fenced content block maps to a Text output block"),
        };
        let state = v
            .assets
            .output_editors
            .get("b1#0")
            .expect("output editor built for the truncated block")
            .read(cx);
        (truncated_from, state.display_rows(), state.visible_rows())
    });

    // Without this the case degenerates into the untruncated one and proves
    // nothing about the missing terminator.
    assert!(
        truncated_from.is_some(),
        "{TRUNCATING_ROWS} numbered lines must exceed the byte cap, else the \
         closing fence survives and this is not the truncated shape"
    );
    assert!(
        rows > max_visible_rows(),
        "the surviving {rows} rows must still overflow the cap"
    );

    let mut vcx = gpui::VisualTestContext::from_window(window_handle, cx);
    let embed = vcx
        .debug_bounds("agent-chat-out-embed-b1#0")
        .expect("the bounded embed painted — a truncated block fell back to markdown");
    assert_eq!(
        embed.size.height,
        bounded_embed_height(rows),
        "the truncated block's embed is capped"
    );
    let visible = visible.expect("the embedded editor painted");
    assert!(
        visible.len() >= capped_rows() && visible.len() <= max_visible_rows(),
        "the truncated block's embed shaped {} of {rows} rows — expected about {}",
        visible.len(),
        capped_rows()
    );
}

/// An output that fits under the cap hides nothing, so it must draw no vertical
/// thumb — a permanently-visible one both reads as broken chrome and lies about
/// whether there is more output. The guard is against reading the thumb's content
/// extent off `InputState::scroll_size().height`, which in code-editor mode is
/// padded past the viewport unconditionally (`gpui_component` `element.rs`).
#[gpui::test]
async fn a_short_output_block_draws_no_vertical_thumb(cx: &mut TestAppContext) {
    let (window_handle, view) = render_fenced_output_card(cx, SMALL_ROWS);
    let (viewport_h, scroll_h) = output_editor_extents(cx, &view);

    let mut vcx = gpui::VisualTestContext::from_window(window_handle, cx);
    // Absence is only meaningful once the embed itself is known to have painted.
    let embed = vcx
        .debug_bounds("agent-chat-out-embed-b1#0")
        .expect("the bounded embed painted");
    assert_eq!(
        embed.size.height,
        bounded_embed_height(SMALL_ROWS),
        "a short embed measures its content"
    );
    assert!(
        scroll_h > viewport_h,
        "the upstream bottom pad is what makes this case a trap: scroll extent \
         {scroll_h:?} must exceed the {viewport_h:?} viewport even though every \
         row is visible"
    );
    assert_eq!(
        vcx.debug_bounds("agent-chat-out-vthumb-b1#0"),
        None,
        "a {SMALL_ROWS}-row output hides nothing, so no vertical thumb may be drawn"
    );
}
