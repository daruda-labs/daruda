//! Switching tabs must not make an AgentChat pane recompile tree-sitter
//! queries for the code blocks it already showed.
//!
//! A pane in an inactive tab is not rendered, and gpui drops every element
//! state that a frame did not touch (`Frame::finish`), including the
//! `TextViewState` that holds a markdown body's parse. So switching back
//! re-parses every visible body **synchronously, in `request_layout`** — and
//! that parse used to build a whole `SyntaxHighlighter`, i.e. compile the
//! language's tree-sitter queries, once per fenced code block. Measured at
//! 22 ms per visible fence (66 ms for three) against a 1 ms ordinary repaint.
//!
//! The fix caches compiled queries per language for the life of the process
//! (vendored `gpui_component::highlighter`), so the test asserts the thing that
//! actually broke — the compile count — rather than a wall-clock budget, which
//! would be flaky and would not say *why* it regressed.

use gpui::{AppContext as _, TestAppContext};

use super::build_workspace;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

use daruda_acp::{ChatItem, ToolCallItem, ToolKindView, ToolOutputBlock, ToolStatusView};

/// Fenced code blocks per assistant reply. More than one so a per-block
/// recompile (the regression) shows up as a multiple, not a single stray.
const FENCES_PER_REPLY: usize = 3;

/// Fence language for this test, registered as a private clone of `rust`.
///
/// The compile counter and the query cache are both process-global, and the
/// test binary runs tests in parallel, so counting `rust` would race every
/// other test that renders Rust. A language only this test names cannot be
/// touched by anything else, which makes both the cold-start assertion and the
/// no-recompile assertion deterministic regardless of test order.
const PROBE_LANGUAGE: &str = "darudaswitchprobe";

/// Register [`PROBE_LANGUAGE`] as a copy of `rust` under its own name, so it
/// gets its own cache entry and its own compile count.
fn register_probe_language() {
    let registry = crate::ui::highlighter::LanguageRegistry::singleton();
    let rust = registry.language("rust").expect("rust is registered");
    let probe = crate::ui::highlighter::LanguageConfig {
        name: PROBE_LANGUAGE.into(),
        ..rust
    };
    registry.register(PROBE_LANGUAGE, &probe);
}

/// One turn: a prompt, a markdown reply with fenced code, two tool calls with
/// output — the shape a real transcript repeats.
fn turn(n: usize) -> Vec<ChatItem> {
    let mut reply = format!(
        "Turn {n}: here is a **finding** with a list\n\n\
         - first point about `foo_{n}`\n- second point\n\n"
    );
    for f in 0..FENCES_PER_REPLY {
        reply.push_str(&format!(
            "```{PROBE_LANGUAGE}\nfn item_{n}_{f}() -> usize {{\n    let x = {n};\n    x * 2\n}}\n```\n\n"
        ));
    }
    reply.push_str(
        "And a closing paragraph long enough to wrap across more than one line \
         at any realistic pane width, like a real reply.",
    );
    let mut v = vec![
        ChatItem::UserText(format!("prompt number {n} please do the thing")),
        ChatItem::AssistantText {
            text: reply,
            streaming: false,
            message_id: None,
        },
    ];
    for k in 0..2 {
        v.push(ChatItem::ToolCall(ToolCallItem {
            id: format!("t{n}_{k}"),
            title: format!("Read src/file_{n}_{k}.rs"),
            kind: ToolKindView::Read,
            tool_name: None,
            status: ToolStatusView::Completed,
            diffs: Vec::new(),
            output: vec![ToolOutputBlock::RawText {
                text: (0..40)
                    .map(|l| format!("line {l} of tool output for {n}/{k}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                truncated_from: None,
            }],
            raw_input: None,
            parent_tool_id: None,
            exit: None,
        }));
    }
    v
}

fn frame(cx: &mut TestAppContext, w: gpui::WindowHandle<gpui_component::Root>) {
    cx.update_window(w.into(), |_, window, _| window.refresh())
        .unwrap();
    cx.run_until_parked();
}

fn go_to_tab(
    cx: &mut TestAppContext,
    w: gpui::WindowHandle<gpui_component::Root>,
    ws: &gpui::Entity<Workspace>,
    ix: usize,
) {
    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.activate_tab(ix, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui::test]
async fn switching_back_to_a_chat_pane_recompiles_no_queries(cx: &mut TestAppContext) {
    register_probe_language();
    let (w, ws) = build_workspace(cx);
    cx.run_until_parked();

    let pane_id: PaneId = cx
        .update_window(w.into(), |_, window, cx| {
            ws.update(cx, |ws, cx| {
                ws.open_agent_chat_pane(window, cx);
                let id = ws.active_runtime().panes.last().expect("pane").id;
                let view = super::agent_chat::agent_view(ws, id);
                view.update(cx, |v, cx| {
                    v.items = (0..20).flat_map(turn).collect();
                    // Expanded: a settled transcript collapses its cards, and a
                    // collapsed card renders no body — which would make this
                    // test pass for the wrong reason.
                    v.set_all_folds(true, window, cx);
                });
                // A second tab to switch away to.
                ws.add_tab(window, cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    let (chat_tab, away_tab) = ws.read_with(cx, |ws, _| {
        let rt = ws.active_runtime();
        let chat = rt
            .tabs
            .iter()
            .position(|t| t.layout.pane_ids().contains(&pane_id))
            .expect("chat tab");
        (chat, (0..rt.tabs.len()).find(|i| *i != chat).expect("away"))
    });

    // First paint: the conversation is on screen and its code blocks compile
    // the probe language once. This is the once-per-language cost the cache
    // exists to bound, and it must happen or the loop below proves nothing.
    let before_first = crate::ui::highlighter::query_compilations(PROBE_LANGUAGE);
    go_to_tab(cx, w, &ws, chat_tab);
    frame(cx, w);

    let laid_out = ws.read_with(cx, |ws, cx| {
        super::agent_chat::agent_view(ws, pane_id)
            .read(cx)
            .list_bounds
            .is_some()
    });
    assert!(
        laid_out,
        "the chat list never laid out, so nothing was highlighted — harness bug"
    );

    let after_first = crate::ui::highlighter::query_compilations(PROBE_LANGUAGE);
    assert!(
        after_first > before_first,
        "no query was compiled on the first paint ({before_first} → {after_first}); \
         the code blocks are not reaching the highlighter, so this test would \
         pass vacuously"
    );

    // Now the part that regressed: leaving and re-entering the tab drops the
    // pane's element state and re-parses every visible body. Not one query may
    // be compiled again.
    for round in 0..3 {
        go_to_tab(cx, w, &ws, away_tab);
        go_to_tab(cx, w, &ws, chat_tab);
        let now = crate::ui::highlighter::query_compilations(PROBE_LANGUAGE);
        assert_eq!(
            now,
            after_first,
            "tab switch #{round} recompiled {} tree-sitter quer(y/ies) — the \
             per-language cache is gone, and every visible fenced code block \
             pays a full `Query::new` on the switch frame",
            now - after_first
        );
    }
}
