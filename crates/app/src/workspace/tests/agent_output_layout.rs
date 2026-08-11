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

/// Far more rows than the cap can show, standing in for large command output
/// that fits inside the 64 KiB byte cap.
const LARGE_ROWS: usize = 200;

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

    // The agent-chat virtual list lays items out as
    // `Definite(width) × MinContent` (gpui `list.rs` `available_item_space`).
    // The windowed probe above sizes the root against a definite window height;
    // the list does not, so a height that only resolves against a definite
    // ancestor must not collapse here.
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

/// Drive a **codex-shaped** shell tool call: its output arrives only through
/// `raw_output.formatted_output`, which `daruda_acp` maps to
/// `ToolOutputBlock::RawText` — the shape the CPU repro (`seq 1 200000` in a
/// codex pane) actually takes. `trailing_newline` mirrors a real command's
/// output, which terminates its last line.
fn render_shell_output_card(
    cx: &mut TestAppContext,
    rows: usize,
    trailing_newline: bool,
) -> (
    gpui::AnyWindowHandle,
    Entity<crate::workspace::main_area::agent_chat_pane::view::AgentChatView>,
) {
    use crate::workspace::main_area::pane::PaneContent;
    use agent_client_protocol::schema::v1::{
        SessionUpdate, ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
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

    let mut printed = numbered_lines(rows);
    if trailing_newline {
        printed.push('\n');
    }
    view.update(cx, |v, cx| {
        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::Completed);
        fields.raw_output = Some(serde_json::json!({
            "formatted_output": printed,
            "exit_code": 0,
        }));
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
    fold_in_window(cx, window_handle.into(), &view, |v, window, cx| {
        v.set_all_folds(true, window, cx)
    });
    cx.run_until_parked();
    cx.update_window(window_handle.into(), |_, window, _| window.refresh())
        .unwrap();
    cx.run_until_parked();
    (window_handle.into(), view)
}

/// Append `count` further completed shell cards to `view`, so the transcript
/// list has scrollable range of its own.
///
/// Scroll-chaining assertions are worthless without it: with one card the list
/// cannot move, so "the transcript did not scroll" holds for free.
fn push_filler_cards(
    cx: &mut TestAppContext,
    window_handle: gpui::AnyWindowHandle,
    view: &Entity<crate::workspace::main_area::agent_chat_pane::view::AgentChatView>,
    count: usize,
) {
    use agent_client_protocol::schema::v1::{
        SessionUpdate, ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
    };

    for n in 0..count {
        let id = format!("filler{n}");
        view.update(cx, |v, cx| {
            let mut fields = ToolCallUpdateFields::default();
            fields.status = Some(ToolCallStatus::Completed);
            fields.raw_output = Some(serde_json::json!({
                "formatted_output": "filler\n",
                "exit_code": 0,
            }));
            v.apply_event(
                daruda_acp::AcpEvent::Update(Box::new(SessionUpdate::ToolCall(
                    ToolCall::new(id.clone(), "Bash filler").kind(ToolKind::Execute),
                ))),
                "",
                false,
                cx,
            );
            v.apply_event(
                daruda_acp::AcpEvent::Update(Box::new(SessionUpdate::ToolCallUpdate(
                    ToolCallUpdate::new(id, fields),
                ))),
                "",
                false,
                cx,
            );
        });
    }
    cx.run_until_parked();
    fold_in_window(cx, window_handle, view, |v, window, cx| {
        v.set_all_folds(true, window, cx);
        // The transcript follows its tail, so the filler would push the card
        // under test out of the virtualized window and it would never paint.
        v.list_state.scroll_to(gpui::ListOffset {
            item_ix: 0,
            offset_in_item: px(0.),
        });
        cx.notify();
    });
    cx.run_until_parked();
}

/// The transcript list's own scrollable range, and how many items it holds —
/// the pair a chaining test must check before trusting either outcome.
fn transcript_scroll_range(
    cx: &mut TestAppContext,
    view: &Entity<crate::workspace::main_area::agent_chat_pane::view::AgentChatView>,
) -> (usize, gpui::Pixels) {
    view.read_with(cx, |v, _| {
        (v.items.len(), v.list_state.max_offset_for_scrollbar().y)
    })
}

/// One downward wheel notch over the embed's centre.
fn wheel_down_over_embed(vcx: &mut gpui::VisualTestContext, embed: gpui::Bounds<gpui::Pixels>) {
    use gpui::{ScrollDelta, ScrollWheelEvent};
    vcx.simulate_event(ScrollWheelEvent {
        position: embed.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-120.))),
        ..Default::default()
    });
    vcx.run_until_parked();
}

/// A capped embed owns the wheel while it still has rows to reveal, then passes
/// the gesture to the transcript once it is already at its end.
#[gpui::test]
async fn capped_embed_wheel_chains_only_after_the_embed_is_exhausted(cx: &mut TestAppContext) {
    let (window_handle, view) = render_shell_output_card(cx, LARGE_ROWS, true);
    push_filler_cards(cx, window_handle, &view, 16);
    cx.update_window(window_handle, |_, window, _| window.refresh())
        .unwrap();
    cx.run_until_parked();
    let (items, max_offset) = transcript_scroll_range(cx, &view);
    assert_eq!(items, 17, "every filler card must reach the transcript");
    assert!(
        max_offset > px(0.),
        "the transcript has no scrollable range, so the fixture is vacuous"
    );

    let before = view.read_with(cx, |v, _| v.list_state.scroll_px_offset_for_scrollbar());
    let mut vcx = gpui::VisualTestContext::from_window(window_handle, cx);
    let embed = vcx
        .debug_bounds("agent-chat-out-embed-b1#0")
        .expect("the bounded embed painted");
    let (is_raw, visible) = view.read_with(&vcx, |v, cx| {
        let raw = v.items.iter().any(|item| match item {
            daruda_acp::ChatItem::ToolCall(tc) => tc
                .output
                .first()
                .is_some_and(|b| matches!(b, daruda_acp::ToolOutputBlock::RawText { .. })),
            _ => false,
        });
        let visible = v
            .assets
            .output_editors
            .get("b1#0")
            .expect("output editor built for the codex shell block")
            .read(cx)
            .visible_rows();
        (raw, visible)
    });
    assert!(
        is_raw,
        "the fixture must exercise the RawText path, not a fenced Text block"
    );
    assert_eq!(
        embed.size.height,
        bounded_embed_height(LARGE_ROWS),
        "a capped shell output embed is pinned to the cap"
    );
    let visible = visible.expect("the embedded editor painted");
    assert!(
        visible.len() <= max_visible_rows(),
        "shaped {} of {LARGE_ROWS} rows — the height bound is gone",
        visible.len()
    );
    assert!(
        visible.len() >= capped_rows(),
        "shaped only {} rows — the editor collapsed inside its reserved box",
        visible.len()
    );
    wheel_down_over_embed(&mut vcx, embed);

    let (after, editor_offset) = view.read_with(&vcx, |v, cx| {
        (
            v.list_state.scroll_px_offset_for_scrollbar(),
            v.assets
                .output_editors
                .get("b1#0")
                .expect("output editor built")
                .read(cx)
                .scroll_handle()
                .offset()
                .y,
        )
    });
    assert!(
        editor_offset < px(0.),
        "the embed had rows to reveal, so it must have scrolled — got {editor_offset:?}"
    );
    assert_eq!(
        after, before,
        "the transcript scrolled too — the embed did not claim the gesture"
    );

    let mut offsets = vec![editor_offset];
    for _ in 0..3 {
        wheel_down_over_embed(&mut vcx, embed);
        offsets.push(view.read_with(&vcx, |v, cx| {
            v.assets
                .output_editors
                .get("b1#0")
                .expect("output editor built")
                .read(cx)
                .scroll_handle()
                .offset()
                .y
        }));
    }
    for (n, pair) in offsets.windows(2).enumerate() {
        assert!(
            pair[1] < pair[0],
            "notch {} did not scroll further: {:?} → {:?} (full run {offsets:?})",
            n + 2,
            pair[0],
            pair[1]
        );
    }

    let editor = view.read_with(&vcx, |v, _| {
        v.assets
            .output_editors
            .get("b1#0")
            .expect("output editor built")
            .clone()
    });
    // A wildly out-of-range value: `set_scroll_offset` clamps to the content, so
    // this lands exactly at the end whatever the content height is.
    editor.update(&mut vcx, |state, cx| {
        state.set_scroll_offset(point(px(0.), px(-1_000_000.)), cx);
    });
    vcx.run_until_parked();
    let end = editor.read_with(&vcx, |state, _| state.scroll_handle().offset().y);
    assert!(
        end < px(0.),
        "the embed should have scrolled somewhere, got {end:?}"
    );

    let before = view.read_with(&vcx, |v, _| v.list_state.scroll_px_offset_for_scrollbar());
    wheel_down_over_embed(&mut vcx, embed);

    let (after, still_at_end) = view.read_with(&vcx, |v, cx| {
        (
            v.list_state.scroll_px_offset_for_scrollbar(),
            v.assets
                .output_editors
                .get("b1#0")
                .expect("output editor built")
                .read(cx)
                .scroll_handle()
                .offset()
                .y,
        )
    });
    assert_eq!(
        still_at_end, end,
        "the embed was already at its end and must not have moved further"
    );
    assert_ne!(
        after, before,
        "the embed had nothing left to give, so the transcript must take the \
         gesture — it stopped dead instead"
    );

    drop(vcx);
    const SHORT_ROWS: usize = 5;
    let (window_handle, view) = render_shell_output_card(cx, SHORT_ROWS, true);
    let (viewport_h, scroll_h) = output_editor_extents(cx, &view);

    let rows = view.read_with(cx, |v, cx| {
        v.assets
            .output_editors
            .get("b1#0")
            .expect("output editor built for the codex shell block")
            .read(cx)
            .display_rows()
    });
    assert_eq!(
        rows,
        SHORT_ROWS,
        "the terminator was counted as a {}th row",
        SHORT_ROWS + 1
    );
    assert!(
        scroll_h <= viewport_h,
        "scroll extent {scroll_h:?} exceeds the {viewport_h:?} viewport even though \
         every row is visible — the embed can be dragged into blank space"
    );

    let mut vcx = gpui::VisualTestContext::from_window(window_handle, cx);
    let embed = vcx
        .debug_bounds("agent-chat-out-embed-b1#0")
        .expect("the bounded embed painted");
    assert_eq!(
        embed.size.height,
        bounded_embed_height(SHORT_ROWS),
        "an uncapped embed measures its output, not its output plus a blank row"
    );
    assert!(
        vcx.debug_bounds("agent-chat-out-vthumb-b1#0").is_none(),
        "a {SHORT_ROWS}-row output hides nothing, so no vertical thumb may be drawn"
    );
}

/// Rows of `cat -n`-numbered Rust, the shape a `Read` tool's body arrives in.
/// Terminated like a real file, so the trailing newline must not count as a row.
/// `{n:>4}` keeps every line 19 bytes so the block stays under `daruda_acp`'s
/// 64 KiB cap and this stays the *untruncated* case.
fn numbered_source(rows: usize) -> String {
    (1..=rows)
        .map(|n| format!("{n:>4}\tlet a = {n};\n"))
        .collect()
}

/// Drive a real agent-chat pane to one completed, expanded `Read` tool call
/// whose only output block is `body` — delivered *unfenced*, the shape an
/// adapter that does not markdown-escape sends. `path` is what the call names in
/// its raw input, and therefore the only source of the language.
fn render_read_output_card(
    cx: &mut TestAppContext,
    path: &str,
    body: String,
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

    let raw_input = serde_json::json!({ "file_path": path });
    view.update(cx, |v, cx| {
        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::Completed);
        fields.content = Some(vec![ToolCallContent::Content(Content::new(
            ContentBlock::Text(TextContent::new(body)),
        ))]);
        v.apply_event(
            daruda_acp::AcpEvent::Update(Box::new(SessionUpdate::ToolCall(
                ToolCall::new("t1", "Read")
                    .kind(ToolKind::Read)
                    .raw_input(raw_input),
            ))),
            "",
            false,
            cx,
        );
        v.apply_event(
            daruda_acp::AcpEvent::Update(Box::new(SessionUpdate::ToolCallUpdate(
                ToolCallUpdate::new("t1", fields),
            ))),
            "",
            false,
            cx,
        );
    });
    cx.run_until_parked();
    fold_in_window(cx, window_handle.into(), &view, |v, window, cx| {
        v.set_all_folds(true, window, cx)
    });
    cx.run_until_parked();
    cx.update_window(window_handle.into(), |_, window, _| window.refresh())
        .unwrap();
    cx.run_until_parked();
    (window_handle.into(), view)
}

/// The sole output block of the `t1` tool call.
fn only_output_block(
    cx: &mut TestAppContext,
    view: &Entity<crate::workspace::main_area::agent_chat_pane::view::AgentChatView>,
) -> daruda_acp::ToolOutputBlock {
    view.read_with(cx, |v, _| {
        let blocks = v
            .items
            .iter()
            .find_map(|item| match item {
                daruda_acp::ChatItem::ToolCall(tc) => Some(&tc.output),
                _ => None,
            })
            .expect("the tool call is present");
        assert_eq!(blocks.len(), 1, "fixture must produce exactly one block");
        blocks[0].clone()
    })
}

/// A `Read` whose body carries no fence used to be rejected by the fence-shaped
/// classifier and fall to the markdown path — unbounded paint, `cat -n` gutter
/// intact, no highlighting. It must reach the capped embed, hold the file's own
/// bytes, and highlight as the extension's language.
#[gpui::test]
async fn read_tool_outputs_render_through_the_capped_embed(cx: &mut TestAppContext) {
    const ROWS: usize = 200;
    let body = numbered_source(ROWS);
    // Fixture validity: without both of these the case degenerates into the
    // already-covered fenced one and proves nothing.
    assert!(
        !body.starts_with("```"),
        "the fixture must be unfenced, else it is the fenced case"
    );
    assert!(
        body.starts_with("   1\tlet a = 1;"),
        "the fixture must carry a `cat -n` gutter to strip, got {:?}",
        &body[..20.min(body.len())]
    );

    let (window_handle, view) = render_read_output_card(cx, "probe.rs", body);

    let block = only_output_block(cx, &view);
    let daruda_acp::ToolOutputBlock::SourceText {
        text,
        language,
        truncated_from,
    } = &block
    else {
        panic!("an unfenced read must map to a SourceText block, got {block:?}");
    };
    assert_eq!(
        language.as_deref(),
        Some("rust"),
        "the language must come from the read target's extension"
    );
    assert_eq!(
        truncated_from, &None,
        "{ROWS} rows must stay under the byte cap, else this is the truncated case"
    );
    assert!(
        text.starts_with("let a = 1;\n"),
        "the `cat -n` gutter survived into the block: {:?}",
        &text[..20.min(text.len())]
    );

    let (editor_language, value, rows, visible) = view.read_with(cx, |v, cx| {
        let state = v
            .assets
            .output_editors
            .get("t1#0")
            .expect("no output editor built — the read fell back to markdown")
            .read(cx);
        (
            state.code_editor_language().cloned(),
            state.value().to_string(),
            state.display_rows(),
            state.visible_rows(),
        )
    });
    assert_eq!(
        editor_language.as_deref(),
        Some("rust"),
        "the editor highlights with something other than the file's language"
    );
    assert!(
        !value.contains('\t'),
        "the editor still holds `cat -n` tab-prefixed rows"
    );
    assert_eq!(
        rows, ROWS,
        "the editor's row count must match the file's lines, terminator excluded"
    );

    let mut vcx = gpui::VisualTestContext::from_window(window_handle, cx);
    let embed = vcx
        .debug_bounds("agent-chat-out-embed-t1#0")
        .expect("the bounded embed painted — the read fell back to markdown");
    assert_eq!(
        embed.size.height,
        bounded_embed_height(ROWS),
        "a read's embed is capped exactly like a shell command's"
    );
    let visible = visible.expect("the embedded editor painted");
    assert!(
        visible.len() >= capped_rows() && visible.len() <= max_visible_rows(),
        "the read's embed shaped {} of {ROWS} rows — expected about {}",
        visible.len(),
        capped_rows()
    );
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

/// A descendant that stops mouse-up propagation must not strand the transcript
/// container's drag lifetime: its mouse-down starts the selection-autoscroll
/// poll, so if the matching mouse-up never reaches it the poll keeps ticking.
#[gpui::test]
async fn clicking_a_floating_button_still_ends_the_selection_drag(cx: &mut TestAppContext) {
    let (window_handle, view) = render_shell_output_card(cx, 3, true);
    push_filler_cards(cx, window_handle, &view, 30);
    cx.update_window(window_handle, |_, window, _| window.refresh())
        .unwrap();
    cx.run_until_parked();
    let mut vcx = gpui::VisualTestContext::from_window(window_handle, cx);
    vcx.run_until_parked();

    let button = vcx
        .debug_bounds("agent-chat-scroll-bottom")
        .expect("scrolled up off the bottom, so the jump-to-bottom button paints");
    vcx.simulate_mouse_move(button.center(), None, Default::default());
    vcx.run_until_parked();
    vcx.simulate_mouse_down(button.center(), gpui::MouseButton::Left, Default::default());
    vcx.run_until_parked();
    assert!(
        view.read_with(&vcx, |v, _| v.selection_drag_active),
        "the container's mouse-down starts the drag, so the fixture is vacuous \
         if it never does"
    );

    vcx.simulate_mouse_up(button.center(), gpui::MouseButton::Left, Default::default());
    vcx.run_until_parked();
    assert!(
        !view.read_with(&vcx, |v, _| v.selection_drag_active),
        "the drag outlived its mouse-up, so the autoscroll poll is still running"
    );
}
