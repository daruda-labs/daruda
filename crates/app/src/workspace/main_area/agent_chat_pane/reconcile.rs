//! Async reconcilers for the agent chat pane — the passes that turn
//! conversation content into GPU-ready artifacts: read-only diff editors for
//! tool-call file edits and read-only editors for verbatim tool output (both
//! rebuilt whenever the underlying content changes), plus rasterized mermaid
//! diagrams for ` ```mermaid ` fences and decoded tool-output images
//! (build-once).
//!
//! Split out of [`view`](super::view) because these are distinct, async-heavy
//! responsibilities (window re-entry to build editor entities; background-executor
//! rasterization that can panic) with their own failure/logging paths — separate
//! from the view's synchronous state-transition ops. They stay `impl
//! AgentChatView` methods (they read/fill the view's `assets` caches and
//! `cx.notify()` the view), driven from
//! [`AgentChatView::apply_event`](super::view::AgentChatView::apply_event) gated
//! on `touched_tool` / `touched_text`.

use std::collections::HashSet;

use daruda_acp::{ChatItem, ToolOutputBlock};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::Context;

use super::agent_chat_helpers::{
    DiffStat, build_diff_view_model, chat_item_mermaid_texts, create_diff_editor, diff_editor_key,
    diff_editor_language, diff_source_fingerprint, mermaid_key, mermaid_sources, tool_image_key,
};
use super::output_editor::{
    create_output_editor, output_editor_key, output_editor_source, output_source_fingerprint,
};
use super::view::AgentChatView;
use crate::workspace::main_area::file_view_pane::diff_editor::{DiffColors, DiffEditorModel};
use crate::workspace::main_area::file_view_pane::mermaid_theme::MermaidPalette;
use crate::workspace::main_area::file_view_pane::render::CachedImage;
use crate::workspace::main_area::file_view_pane::visual;

impl AgentChatView {
    /// Build the read-only diff editor entity for every tool-call file
    /// modification whose current diff content doesn't match the editor
    /// already cached for it. Called from `apply_event` after `items` mutates,
    /// so the (cached) subtree shows the diff through the same editor the File
    /// viewer uses rather than the inline fallback.
    ///
    /// Keyed by `"{tool_call_id}#{diff_index}"` — one editor per file. A diff is
    /// converted to a `DiffEditorModel` purely (no GPUI), then the editor entity
    /// is created + configured inside a single window re-entry against the
    /// stored `window_handle`.
    ///
    /// Rebuilt-on-change, not build-once: `apply_tool_call_update`
    /// (`daruda_acp::mapping`) replaces a tool call's `diffs` wholesale on every
    /// `ToolCallUpdate`, so a streaming write/edit can hand this the same
    /// `{tool_call_id}#{diff_index}` key with growing content across several
    /// events. Comparing `diff_source_fingerprint` against the fingerprint
    /// stored when the cached editor was built catches that case — an
    /// unchanged fingerprint skips the rebuild, a changed one replaces the
    /// stale editor so it doesn't stay frozen on an early partial snapshot
    /// (the diff box then undersizes and its tail visually merges into the
    /// tool card's Output section). A cached key this pass doesn't claim is
    /// dropped from all three maps — see [`stale_keys`].
    pub(in crate::workspace) fn reconcile_diff_editors(
        &mut self,
        syntax_theme: &str,
        is_light: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(colors) = cx
            .try_global::<crate::ui::theme::DarudaTheme>()
            .map(|t| DiffColors::from_agent_chat_theme(t, cx))
        else {
            // Theme global not yet installed (transient cold-start) — skip
            // editor creation; every diff renders via the inline fallback.
            // Logged so the blanket fallback isn't a silent no-op.
            daruda_store::observability::log_writer::LogWriter::log(
                ErrorReport::new("Skipping agent-chat diff editors: theme global absent")
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .dedup(format!(
                        "agent_chat.diff_editor.theme_missing.{}",
                        self.pane_id
                    ))
                    .build(),
            );
            return;
        };

        // Collect the pure work first; entity creation re-enters the window,
        // which can't happen while the immutable `items` borrow is live.
        let mut pending: Vec<(String, u64, gpui::SharedString, DiffEditorModel, DiffStat)> =
            Vec::new();
        let mut live: HashSet<String> = HashSet::new();
        for item in &self.items {
            let ChatItem::ToolCall(tc) = item else {
                continue;
            };
            for (di, diff) in tc.diffs.iter().enumerate() {
                let key = diff_editor_key(&tc.id, di);
                let fingerprint = diff_source_fingerprint(diff);
                if self.assets.diff_editor_sources.get(&key) == Some(&fingerprint) {
                    // Unchanged still counts as live, or the next pass would
                    // destroy and rebuild its editor.
                    live.insert(key);
                    continue;
                }
                let Some((model, stat)) =
                    build_diff_view_model(diff, syntax_theme, is_light, &colors)
                else {
                    // Converged to no hunks (the rare reverted-mid-stream case):
                    // left unclaimed so any cached editor is dropped below and
                    // the body falls back to the "no changes" render.
                    continue;
                };
                let language = diff_editor_language(diff);
                live.insert(key.clone());
                pending.push((key, fingerprint, language, model, stat));
            }
        }
        let stale = stale_keys(self.assets.diff_editors.keys(), &live);
        if pending.is_empty() && stale.is_empty() {
            return;
        }

        for key in stale {
            self.assets.diff_editors.remove(&key);
            self.assets.diff_stats.remove(&key);
            self.assets.diff_editor_sources.remove(&key);
        }

        let window_handle = self.window_handle;
        let pane_id = self.pane_id;
        for (key, fingerprint, language, model, stat) in pending {
            if let Some(editor) = create_diff_editor(cx, window_handle, pane_id, &language, model) {
                // Cache the stat under the same key as the editor so the fold
                // summary (`+N −M`) reads it back via `diff_editor_key`. Stored
                // only when the editor builds — a no-change diff yields no
                // editor and no stat (absent ≡ `0/0`).
                self.assets.diff_stats.insert(key.clone(), stat);
                self.assets
                    .diff_editor_sources
                    .insert(key.clone(), fingerprint);
                self.assets.diff_editors.insert(key, editor);
            }
        }
        // A tool card just grew (or a stale editor was dropped/replaced), and
        // the touched call may sit mid-list (a `ToolCallUpdate` to an earlier
        // call), so `sync_list_after`'s tail-only remeasure isn't enough — do a
        // full one. Once per diff-bearing event.
        self.list_state.remeasure();
    }

    /// Build the read-only editor entity for every verbatim tool-output block
    /// whose current content doesn't match the editor already cached for it.
    /// Called from `apply_event` after `items` mutates, so a long shell output
    /// paints through `InputState` (visible rows only) instead of gpui's
    /// per-line text walk.
    ///
    /// Keyed by `"{tool_call_id}#{block_index}"` — one editor per output block.
    /// Unlike [`Self::reconcile_diff_editors`] this takes no theme input: the
    /// editor colours through `gpui_component`'s built-in tree-sitter path,
    /// which reads the palette at paint time, so there is no theme-derived model
    /// to pre-compute here and no `DarudaTheme` global read to guard.
    ///
    /// Rebuilt-on-change, not build-once: `apply_tool_call_update`
    /// (`daruda_acp::mapping`) replaces a tool call's `output` wholesale on
    /// every `ToolCallUpdate`, so a streaming shell command hands this the same
    /// key with growing text. A key whose fingerprint moved is rebuilt; a cached
    /// key the walk no longer visits is dropped, which covers both a block that
    /// stopped qualifying and an index a shrunken `output` vec no longer reaches.
    pub(in crate::workspace) fn reconcile_output_editors(&mut self, cx: &mut Context<Self>) {
        // Collect the pure work first; entity creation re-enters the window,
        // which can't happen while the immutable `items` borrow is live.
        let mut pending: Vec<(String, u64, String, Option<String>)> = Vec::new();
        let mut live: HashSet<String> = HashSet::new();
        for item in &self.items {
            let ChatItem::ToolCall(tc) = item else {
                continue;
            };
            for (bi, block) in tc.output.iter().enumerate() {
                let Some(src) = output_editor_source(block) else {
                    continue;
                };
                let key = output_editor_key(&tc.id, bi);
                let fingerprint = output_source_fingerprint(&src);
                // Owning the body is what costs — up to the 64 KiB output cap —
                // so it happens only past the skip. A key that is merely
                // unchanged still counts as live, or the next pass would destroy
                // and rebuild its editor.
                if self.assets.output_editor_sources.get(&key) == Some(&fingerprint) {
                    live.insert(key);
                    continue;
                }
                let text = src.text.to_owned();
                let language = src.language.map(str::to_owned);
                live.insert(key.clone());
                pending.push((key, fingerprint, text, language));
            }
        }
        let stale = stale_keys(self.assets.output_editors.keys(), &live);
        if pending.is_empty() && stale.is_empty() {
            return;
        }

        for key in stale {
            self.assets.output_editors.remove(&key);
            self.assets.output_editor_sources.remove(&key);
        }

        let window_handle = self.window_handle;
        let pane_id = self.pane_id;
        for (key, fingerprint, text, language) in pending {
            if let Some(editor) =
                create_output_editor(cx, window_handle, pane_id, text, language.as_deref())
            {
                self.assets
                    .output_editor_sources
                    .insert(key.clone(), fingerprint);
                self.assets.output_editors.insert(key, editor);
            }
        }
        // A tool card just changed height (an embed appeared, grew, or was
        // dropped), and the touched call may sit mid-list (a `ToolCallUpdate` to
        // an earlier call), so `sync_list_after`'s tail-only remeasure isn't
        // enough — do a full one. Once per output-bearing event.
        self.list_state.remeasure();
    }

    /// Rasterize every ` ```mermaid ` fence in the conversation that does not
    /// yet have a cached bitmap (and isn't already being rendered). Collect the
    /// pure work first, then spawn each rasterization on the background executor
    /// (mermaid rendering is CPU-heavy and can panic), and re-enter the view to
    /// fill the cache + `cx.notify()` when it lands.
    ///
    /// `dark` matches the diagram theme to the host appearance so edges stay
    /// visible. Theme-switch staleness (a cached raster keeps its colour after a
    /// light/dark toggle) is out of scope; the cache is only ever added to.
    pub(in crate::workspace) fn reconcile_mermaid(&mut self, dark: bool, cx: &mut Context<Self>) {
        // Collect the not-yet-cached, not-in-flight sources first; the spawn
        // re-enters the view, which can't happen while the `items` borrow is
        // live.
        let mut pending: Vec<(u64, String)> = Vec::new();
        for item in &self.items {
            for text in chat_item_mermaid_texts(item) {
                for source in mermaid_sources(text) {
                    let key = mermaid_key(&source, dark);
                    if self
                        .assets
                        .mermaid_images
                        .lock()
                        .unwrap()
                        .contains_key(&key)
                        || self.assets.mermaid_inflight.contains(&key)
                        || pending.iter().any(|(k, _)| *k == key)
                    {
                        continue;
                    }
                    pending.push((key, source));
                }
            }
        }
        if pending.is_empty() {
            return;
        }

        // Mark all pending keys in-flight before spawning so a second event
        // arriving before any task resolves doesn't re-spawn the same source.
        for (key, _) in &pending {
            self.assets.mermaid_inflight.insert(*key);
        }

        // Resolved here (main thread) so the background rasterizer never
        // touches `Hsla` / the `DarudaTheme` global — see `MermaidPalette`.
        let palette = MermaidPalette::from_agent_chat(cx);

        for (key, source) in pending {
            let palette = palette.clone();
            cx.spawn(async move |this, cx| {
                let raster = cx
                    .background_executor()
                    .spawn(async move {
                        // On panic / error the raster is dropped and the fence
                        // keeps rendering as a default code block.
                        visual::render_mermaid_raster(&source, &palette)
                    })
                    .await;
                // SILENT-OK: view/window dropped before the raster resolved — nothing left to cache it on.
                let _ = this.update(cx, |view, cx| {
                    view.assets.mermaid_inflight.remove(&key);
                    // Convert the raster to a GPU-ready image once, here, so the
                    // render hook clones the same `CachedImage` each frame and
                    // gpui reuses the uploaded texture.
                    if let Some(image) = raster.and_then(|r| CachedImage::from_raster(&r)) {
                        view.assets
                            .mermaid_images
                            .lock()
                            .unwrap()
                            .insert(key, image);
                        // The fence grew from a code block to a diagram, so its
                        // cached height is stale — remeasure before repainting or
                        // it clips. Index is unknown here, so this is a full
                        // remeasure, but it's one-shot per landed raster.
                        view.list_state.remeasure();
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }

    /// Decode every tool-output `Image` block in the conversation that does not
    /// yet have a cached bitmap (and isn't already being decoded). Mirrors
    /// [`Self::reconcile_mermaid`]'s collect-then-spawn shape, minus the `dark`
    /// theming (tool images are rendered as-is, not re-themed) — collect the
    /// pure work first, then decode each on the background executor
    /// (`image::load_from_memory` can be costly for a large screenshot), and
    /// re-enter the view to fill the cache + `cx.notify()` when it lands.
    ///
    /// A decode failure caches `None` under the key rather than leaving it
    /// absent, so a malformed payload renders a failure label once instead of
    /// retrying forever.
    pub(in crate::workspace) fn reconcile_tool_images(&mut self, cx: &mut Context<Self>) {
        // Collect the not-yet-cached, not-in-flight images first; the spawn
        // re-enters the view, which can't happen while the `items` borrow is
        // live.
        let mut pending: Vec<(u64, String)> = Vec::new();
        for item in &self.items {
            let ChatItem::ToolCall(tc) = item else {
                continue;
            };
            for block in &tc.output {
                let ToolOutputBlock::Image { data, .. } = block else {
                    continue;
                };
                let key = tool_image_key(data);
                if self.assets.tool_images.lock().unwrap().contains_key(&key)
                    || self.assets.tool_image_inflight.contains(&key)
                    || pending.iter().any(|(k, _)| *k == key)
                {
                    continue;
                }
                pending.push((key, data.clone()));
            }
        }
        if pending.is_empty() {
            return;
        }

        // Mark all pending keys in-flight before spawning so a second event
        // arriving before any task resolves doesn't re-spawn the same image.
        for (key, _) in &pending {
            self.assets.tool_image_inflight.insert(*key);
        }

        for (key, data) in pending {
            cx.spawn(async move |this, cx| {
                let raster = cx
                    .background_executor()
                    .spawn(async move {
                        // The decoder is a third-party crate over untrusted
                        // agent-supplied bytes; guard against a panic on
                        // malformed input so one bad payload can't take the
                        // executor down — on panic / error we drop it (cached
                        // as a failure below) instead of retrying.
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            use base64::Engine as _;
                            base64::engine::general_purpose::STANDARD
                                .decode(&data)
                                .ok()
                                .and_then(|bytes| visual::decode_image(&bytes).ok())
                        }))
                        .ok()
                        .flatten()
                    })
                    .await;
                // SILENT-OK: view/window dropped before the decode resolved — nothing left to cache it on.
                let _ = this.update(cx, |view, cx| {
                    view.assets.tool_image_inflight.remove(&key);
                    // Convert the raster to a GPU-ready image here (main
                    // thread), same as `reconcile_mermaid` — once, so the
                    // render hook clones the same `CachedImage` each frame.
                    // `None` (decode failed, or the conversion itself failed)
                    // is cached too, so a malformed payload renders a failure
                    // label once instead of retrying forever.
                    let cached = raster.and_then(|r| CachedImage::from_raster(&r));
                    view.assets.tool_images.lock().unwrap().insert(key, cached);
                    // The block just resolved from a pending placeholder to a
                    // decoded image (or a failure label), so its cached height
                    // is stale — remeasure before repainting or it clips.
                    // Index is unknown here, so this is a full remeasure, but
                    // it's one-shot per landed decode.
                    view.list_state.remeasure();
                    cx.notify();
                });
            })
            .detach();
        }
    }
}

/// The cached keys `live` (everything the pass claimed) leaves behind — an
/// entry whose backing content is gone: a shrunken `output` / `diffs` vec, a
/// tool call that left `items`, or a block that stopped qualifying. Shared so
/// both reconcilers invalidate by the same rule; left behind, an entry would
/// either freeze the old content on screen or leak for the session.
fn stale_keys<'a>(cached: impl Iterator<Item = &'a String>, live: &HashSet<String>) -> Vec<String> {
    cached.filter(|key| !live.contains(*key)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use daruda_acp::{
        ChatItem, DiffView, ToolCallItem, ToolKindView, ToolOutputBlock, ToolStatusView,
    };
    use gpui::TestAppContext;

    use super::super::view::tests::make_test_view;
    use super::mermaid_key;

    const KEY: &str = "call_1#0";

    /// A syntax theme id the diff reconciler's highlight pass can resolve.
    const SYNTAX_THEME: &str = "base16-ocean.dark";

    fn tool_call(output: Vec<ToolOutputBlock>) -> ChatItem {
        tool_call_with(output, Vec::new())
    }

    fn diff_tool_call(diffs: Vec<DiffView>) -> ChatItem {
        tool_call_with(Vec::new(), diffs)
    }

    fn tool_call_with(output: Vec<ToolOutputBlock>, diffs: Vec<DiffView>) -> ChatItem {
        ChatItem::ToolCall(ToolCallItem {
            id: "call_1".to_string(),
            title: "Bash".to_string(),
            kind: ToolKindView::Execute,
            tool_name: None,
            status: ToolStatusView::Completed,
            diffs,
            output,
            raw_input: None,
            parent_tool_id: None,
            exit: None,
        })
    }

    fn diff(path: &str, new_text: &str) -> DiffView {
        DiffView {
            path: path.into(),
            old_text: Some("old\n".to_string()),
            new_text: new_text.to_string(),
        }
    }

    fn raw(text: &str) -> ToolOutputBlock {
        ToolOutputBlock::RawText {
            text: text.to_string(),
            truncated_from: None,
        }
    }

    /// The reconciler owns the whole editor lifecycle: one entity per
    /// qualifying output block, reused while the content is unchanged, rebuilt
    /// when a streamed output grows, and dropped once the block (or its index)
    /// is gone. Driven through the view entity rather than `window.update` —
    /// editor creation re-enters the window, which a held window update blocks.
    #[gpui::test]
    fn output_editors_are_built_reused_rebuilt_and_dropped(cx: &mut TestAppContext) {
        let window = make_test_view(cx);
        let view = window.root(cx).expect("the view is the window root");

        let reconcile_with = |cx: &mut TestAppContext, output: Vec<ToolOutputBlock>| {
            view.update(cx, |v, cx| {
                v.items = vec![tool_call(output)];
                v.reconcile_output_editors(cx);
            });
        };
        let editor_id = |cx: &mut TestAppContext| {
            view.read_with(cx, |v, _| {
                v.assets.output_editors.get(KEY).map(|e| e.entity_id())
            })
        };
        let editor_text = |cx: &mut TestAppContext| {
            view.read_with(cx, |v, cx| {
                v.assets.output_editors[KEY].read(cx).value().to_string()
            })
        };

        reconcile_with(cx, vec![raw("line 1\nline 2")]);
        let built = editor_id(cx).expect("a verbatim block gets an editor");
        assert_eq!(editor_text(cx), "line 1\nline 2");
        view.read_with(cx, |v, _| {
            assert_eq!(v.assets.output_editors.len(), 1);
            assert!(v.assets.output_editor_sources.contains_key(KEY));
        });

        reconcile_with(cx, vec![raw("line 1\nline 2")]);
        assert_eq!(
            editor_id(cx),
            Some(built),
            "unchanged content reuses the cached editor"
        );

        reconcile_with(cx, vec![raw("line 1\nline 2\nline 3")]);
        assert_ne!(
            editor_id(cx),
            Some(built),
            "a grown output must not stay frozen on the partial snapshot"
        );
        assert_eq!(editor_text(cx), "line 1\nline 2\nline 3");

        // A block that stopped qualifying drops its editor so the markdown /
        // monospace fallback renders again.
        reconcile_with(
            cx,
            vec![ToolOutputBlock::Image {
                data: "AAAA".to_string(),
                mime: "image/png".to_string(),
            }],
        );
        view.read_with(cx, |v, _| {
            assert!(v.assets.output_editors.is_empty());
            assert!(v.assets.output_editor_sources.is_empty());
        });

        // An index the replacement output vec no longer reaches is dropped too.
        reconcile_with(cx, vec![raw("back again")]);
        assert!(editor_id(cx).is_some());
        reconcile_with(cx, Vec::new());
        view.read_with(cx, |v, _| {
            assert!(
                v.assets.output_editors.is_empty() && v.assets.output_editor_sources.is_empty(),
                "a shrunken output vec must not leak editors under stale indexes"
            );
        });
    }

    /// The diff reconciler drops by the same rule: an entry whose diff the pass
    /// no longer reaches — a shrunken `diffs` vec, or a tool call gone from
    /// `items` — leaves all three parallel maps, while a diff that is merely
    /// unchanged keeps the editor it already has.
    #[gpui::test]
    fn diff_editors_are_dropped_when_their_diff_is_gone(cx: &mut TestAppContext) {
        let window = make_test_view(cx);
        let view = window.root(cx).expect("the view is the window root");

        let reconcile_with = |cx: &mut TestAppContext, items: Vec<ChatItem>| {
            view.update(cx, |v, cx| {
                v.items = items;
                v.reconcile_diff_editors(SYNTAX_THEME, false, cx);
            });
        };
        let cached = |cx: &mut TestAppContext, key: &str| {
            let key = key.to_string();
            view.read_with(cx, |v, _| {
                (
                    v.assets.diff_editors.get(&key).map(|e| e.entity_id()),
                    v.assets.diff_stats.contains_key(&key),
                    v.assets.diff_editor_sources.contains_key(&key),
                )
            })
        };

        reconcile_with(
            cx,
            vec![diff_tool_call(vec![
                diff("a.rs", "first\n"),
                diff("b.rs", "second\n"),
            ])],
        );
        let first = cached(cx, "call_1#0");
        assert!(
            first.0.is_some() && first.1 && first.2,
            "each diff gets an editor plus its stat and fingerprint"
        );
        assert!(cached(cx, "call_1#1").0.is_some());

        // The second diff is gone from the replacement vec, so nothing keyed to
        // it may survive — but the first is unchanged and keeps its editor.
        reconcile_with(cx, vec![diff_tool_call(vec![diff("a.rs", "first\n")])]);
        assert_eq!(
            cached(cx, "call_1#0"),
            first,
            "an unchanged diff must keep the editor it already built"
        );
        assert_eq!(
            cached(cx, "call_1#1"),
            (None, false, false),
            "a shrunken diffs vec must not leak an editor, stat or fingerprint"
        );

        // The whole tool call left the conversation.
        reconcile_with(cx, Vec::new());
        view.read_with(cx, |v, _| {
            assert!(
                v.assets.diff_editors.is_empty()
                    && v.assets.diff_stats.is_empty()
                    && v.assets.diff_editor_sources.is_empty(),
                "a tool call that left `items` must not leak its diff entries"
            );
        });
    }

    /// A mermaid fence inside a subagent-launch's `prompt` (rendered as
    /// markdown in the "Instructions" section, `render/tool.rs`) must be
    /// scanned the same as one in assistant text or tool output — proven
    /// end-to-end through the real reconciler, not just the pure
    /// `chat_item_mermaid_texts` helper it calls. Checked via
    /// `mermaid_inflight` (set synchronously, before the background
    /// rasterization task is even spawned) rather than `mermaid_images`, so
    /// the test doesn't depend on the real rasterizer completing.
    #[gpui::test]
    fn reconcile_mermaid_scans_a_subagent_prompt(cx: &mut TestAppContext) {
        let window = make_test_view(cx);
        let view = window.root(cx).expect("the view is the window root");

        let source = "flowchart TD\n  A-->B";
        view.update(cx, |v, cx| {
            v.items = vec![ChatItem::ToolCall(ToolCallItem {
                id: "call_1".to_string(),
                title: "Implement Task 2".to_string(),
                kind: ToolKindView::Think,
                tool_name: None,
                status: ToolStatusView::InProgress,
                diffs: Vec::new(),
                output: Vec::new(),
                raw_input: Some(serde_json::json!({
                    "subagent_type": "general-purpose",
                    "prompt": format!("Draw it:\n```mermaid\n{source}\n```"),
                })),
                parent_tool_id: None,
                exit: None,
            })];
            v.reconcile_mermaid(false, cx);
        });

        let key = mermaid_key(source, false);
        view.read_with(cx, |v, _| {
            assert!(
                v.assets.mermaid_inflight.contains(&key),
                "the prompt's mermaid fence must be picked up for rasterization"
            );
        });
    }
}
