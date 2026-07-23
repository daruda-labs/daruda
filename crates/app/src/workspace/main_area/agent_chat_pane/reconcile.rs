//! Async reconcilers for the agent chat pane — the two passes that turn
//! conversation content into GPU-ready artifacts: read-only diff editors for
//! tool-call file edits (rebuilt whenever the underlying diff content
//! changes), and rasterized mermaid diagrams for ` ```mermaid ` fences
//! (build-once).
//!
//! Split out of [`view`](super::view) because both are distinct, async-heavy
//! responsibilities (window re-entry to build editor entities; background-executor
//! rasterization that can panic) with their own failure/logging paths — separate
//! from the view's synchronous state-transition ops. They stay `impl
//! AgentChatView` methods (they read/fill the view's `diff_editors` /
//! `mermaid_images` caches and `cx.notify()` the view), driven from
//! [`AgentChatView::apply_event`](super::view::AgentChatView::apply_event) gated
//! on `touched_tool` / `touched_text`.

use daruda_acp::{ChatItem, ToolOutputBlock};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::Context;

use super::agent_chat_helpers::{
    DiffStat, build_diff_view_model, chat_item_markdown, create_diff_editor, diff_editor_key,
    diff_editor_language, diff_source_fingerprint, mermaid_key, mermaid_sources, tool_image_key,
};
use super::view::AgentChatView;
use crate::workspace::main_area::file_view_pane::diff_editor::{DiffColors, DiffEditorModel};
use crate::workspace::main_area::file_view_pane::markdown_viewer::mermaid_with_theme;
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
    /// tool card's Output section).
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
        let mut pending: Vec<(String, u64, String, DiffEditorModel, DiffStat)> = Vec::new();
        // A diff that converged to no hunks since its editor was built (the
        // rare reverted-mid-stream case) needs that stale editor dropped so
        // the body falls back to the "no changes" / inline render instead of
        // keeping a frozen one around forever.
        let mut stale: Vec<String> = Vec::new();
        for item in &self.items {
            let ChatItem::ToolCall(tc) = item else {
                continue;
            };
            for (di, diff) in tc.diffs.iter().enumerate() {
                let key = diff_editor_key(&tc.id, di);
                let fingerprint = diff_source_fingerprint(diff);
                if self.diff_editor_sources.get(&key) == Some(&fingerprint) {
                    continue;
                }
                let Some((model, stat)) =
                    build_diff_view_model(diff, syntax_theme, is_light, &colors)
                else {
                    if self.diff_editors.contains_key(&key) {
                        stale.push(key);
                    }
                    continue;
                };
                let language = diff_editor_language(diff).to_owned();
                pending.push((key, fingerprint, language, model, stat));
            }
        }
        if pending.is_empty() && stale.is_empty() {
            return;
        }

        for key in stale {
            self.diff_editors.remove(&key);
            self.diff_stats.remove(&key);
            self.diff_editor_sources.remove(&key);
        }

        let window_handle = self.window_handle;
        let pane_id = self.pane_id;
        for (key, fingerprint, language, model, stat) in pending {
            if let Some(editor) = create_diff_editor(cx, window_handle, pane_id, &language, model) {
                // Cache the stat under the same key as the editor so the fold
                // summary (`+N −M`) reads it back via `diff_editor_key`. Stored
                // only when the editor builds — a no-change diff yields no
                // editor and no stat (absent ≡ `0/0`).
                self.diff_stats.insert(key.clone(), stat);
                self.diff_editor_sources.insert(key.clone(), fingerprint);
                self.diff_editors.insert(key, editor);
            }
        }
        // A tool card just grew (or a stale editor was dropped/replaced), and
        // the touched call may sit mid-list (a `ToolCallUpdate` to an earlier
        // call), so `sync_list_after`'s tail-only remeasure isn't enough — do a
        // full one. Once per diff-bearing event.
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
                if self.tool_images.lock().unwrap().contains_key(&key)
                    || self.tool_image_inflight.contains(&key)
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
            self.tool_image_inflight.insert(*key);
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
                    view.tool_image_inflight.remove(&key);
                    // Convert the raster to a GPU-ready image here (main
                    // thread), same as `reconcile_mermaid` — once, so the
                    // render hook clones the same `CachedImage` each frame.
                    // `None` (decode failed, or the conversion itself failed)
                    // is cached too, so a malformed payload renders a failure
                    // label once instead of retrying forever.
                    let cached = raster.and_then(|r| CachedImage::from_raster(&r));
                    view.tool_images.lock().unwrap().insert(key, cached);
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
