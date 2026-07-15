//! Async reconcilers for the agent chat pane — the two build-once passes that
//! turn conversation content into GPU-ready artifacts: read-only diff editors
//! for tool-call file edits, and rasterized mermaid diagrams for ` ```mermaid `
//! fences.
//!
//! Split out of [`view`](super::view) because both are distinct, async-heavy
//! responsibilities (window re-entry to build editor entities; background-executor
//! rasterization that can panic) with their own failure/logging paths — separate
//! from the view's synchronous state-transition ops. They stay `impl
//! AgentChatView` methods (they read/fill the view's `diff_editors` /
//! `mermaid_images` caches and `cx.notify()` the view), driven from
//! [`AgentChatView::apply_event`](super::view::AgentChatView::apply_event) gated
//! on `touched_tool` / `touched_text`.

use daruda_acp::ChatItem;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::Context;

use super::agent_chat_helpers::{
    DiffStat, build_diff_view_model, chat_item_markdown, create_diff_editor, diff_editor_key,
    diff_editor_language, mermaid_key, mermaid_sources,
};
use super::view::AgentChatView;
use crate::workspace::main_area::file_view_pane::diff_editor::{DiffColors, DiffEditorModel};
use crate::workspace::main_area::file_view_pane::markdown_viewer::mermaid_with_theme;
use crate::workspace::main_area::file_view_pane::render::CachedImage;
use crate::workspace::main_area::file_view_pane::visual;

impl AgentChatView {
    /// Build the read-only diff editor entity for every tool-call file
    /// modification that does not yet have one. Called from `apply_event` after
    /// `items` mutates, so the (cached) subtree shows the diff through the same
    /// editor the File viewer uses rather than the inline fallback.
    ///
    /// Keyed by `"{tool_call_id}#{diff_index}"` — one editor per file. A diff is
    /// converted to a `DiffEditorModel` purely (no GPUI), then the editor entity
    /// is created + configured inside a single window re-entry against the
    /// stored `window_handle`. Build-once: keys are only filled when absent.
    pub(in crate::workspace) fn reconcile_diff_editors(
        &mut self,
        syntax_theme: &str,
        is_light: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(colors) = cx
            .try_global::<crate::ui::theme::DarudaTheme>()
            .map(DiffColors::from_theme)
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
        let mut pending: Vec<(String, String, DiffEditorModel, DiffStat)> = Vec::new();
        for item in &self.items {
            let ChatItem::ToolCall(tc) = item else {
                continue;
            };
            for (di, diff) in tc.diffs.iter().enumerate() {
                let key = diff_editor_key(&tc.id, di);
                if self.diff_editors.contains_key(&key) {
                    continue;
                }
                let Some((model, stat)) =
                    build_diff_view_model(diff, syntax_theme, is_light, &colors)
                else {
                    continue;
                };
                let language = diff_editor_language(diff).to_owned();
                pending.push((key, language, model, stat));
            }
        }
        if pending.is_empty() {
            return;
        }

        let window_handle = self.window_handle;
        let pane_id = self.pane_id;
        for (key, language, model, stat) in pending {
            if let Some(editor) = create_diff_editor(cx, window_handle, pane_id, &language, model) {
                // Cache the stat under the same key as the editor so the fold
                // summary (`+N −M`) reads it back via `diff_editor_key`. Stored
                // only when the editor builds — a no-change diff yields no
                // editor and no stat (absent ≡ `0/0`).
                self.diff_stats.insert(key.clone(), stat);
                self.diff_editors.insert(key, editor);
            }
        }
        // A tool card just grew, and the touched call may sit mid-list (a
        // `ToolCallUpdate` to an earlier call), so `sync_list_after`'s tail-only
        // remeasure isn't enough — do a full one. Once per diff-bearing event.
        self.list_state.remeasure();
    }

    /// Rasterize every ` ```mermaid ` fence in the conversation that does not
    /// yet have a cached bitmap (and isn't already being rendered). Collect the
    /// pure work first, then spawn each rasterization on the background executor
    /// (selkie is CPU-heavy and can panic), and re-enter the view to fill the
    /// cache + `cx.notify()` when it lands.
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
            let Some(text) = chat_item_markdown(item) else {
                continue;
            };
            for source in mermaid_sources(text) {
                let key = mermaid_key(&source, dark);
                if self.mermaid_images.lock().unwrap().contains_key(&key)
                    || self.mermaid_inflight.contains(&key)
                    || pending.iter().any(|(k, _)| *k == key)
                {
                    continue;
                }
                pending.push((key, source));
            }
        }
        if pending.is_empty() {
            return;
        }

        // Mark all pending keys in-flight before spawning so a second event
        // arriving before any task resolves doesn't re-spawn the same source.
        for (key, _) in &pending {
            self.mermaid_inflight.insert(*key);
        }

        for (key, source) in pending {
            cx.spawn(async move |this, cx| {
                let raster = cx
                    .background_executor()
                    .spawn(async move {
                        let themed = mermaid_with_theme(&source, dark);
                        // selkie is a young reimplementation; guard against a
                        // panic on malformed input so one bad diagram can't take
                        // the executor down — on panic / error we drop it and the
                        // fence keeps the default code rendering.
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            selkie::render::render_text(&themed)
                                .ok()
                                .and_then(|svg| visual::rasterize_svg(&svg).ok())
                        }))
                        .ok()
                        .flatten()
                    })
                    .await;
                // SILENT-OK: view/window dropped before the raster resolved — nothing left to cache it on.
                let _ = this.update(cx, |view, cx| {
                    view.mermaid_inflight.remove(&key);
                    // Convert the raster to a GPU-ready image once, here, so the
                    // render hook clones the same `CachedImage` each frame and
                    // gpui reuses the uploaded texture.
                    if let Some(image) = raster.and_then(|r| CachedImage::from_raster(&r)) {
                        view.mermaid_images.lock().unwrap().insert(key, image);
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
}
