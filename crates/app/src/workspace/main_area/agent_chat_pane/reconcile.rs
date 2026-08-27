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
use gpui::{Context, Window};

use super::agent_chat_helpers::{
    DiffStat, TurnBoundary, build_diff_view_model, chat_item_mermaid_texts, create_diff_editor,
    diff_build_fingerprint, diff_editor_key, diff_editor_language, diff_theme_fingerprint,
    fold_context_at, mermaid_key, mermaid_sources, tool_fold_key, tool_image_key,
};
use super::output_editor::{
    create_output_editor, output_editor_key, output_editor_source, output_source_fingerprint,
};
use super::view::AgentChatView;
use super::window_access::WindowAccess;
use crate::workspace::main_area::file_view_pane::diff_editor::{DiffColors, DiffEditorModel};
use crate::workspace::main_area::file_view_pane::mermaid_theme::MermaidPalette;
use crate::workspace::main_area::file_view_pane::render::CachedImage;
use crate::workspace::main_area::file_view_pane::visual;

/// Which tool calls a reconcile pass must revisit.
///
/// The passes below detect "did this content change" by fingerprinting it, so
/// visiting a call is not free — it costs the call's whole diff / output text.
/// A streamed `ToolCallUpdate` replaces exactly one call (`apply_tool_call_update`
/// in `daruda_acp::mapping`) and names it, so re-fingerprinting the rest of the
/// conversation buys nothing and makes a long turn quadratic: every chunk
/// re-hashed every diff the turn had produced so far.
#[derive(Debug)]
pub(in crate::workspace) enum ReconcileScope {
    /// Every tool call. Required whenever `items` moved as a whole (connect,
    /// `session/load` catch-up) or every card's fold moved at once (expand-all /
    /// collapse-all) — only a full pass can see that a call left the
    /// conversation.
    All,
    /// Just this call. Nothing else in `items` changed, so no other cached key
    /// can have gone stale.
    Tool(String),
}

impl ReconcileScope {
    /// Whether this pass visits `tool_id`.
    fn covers(&self, tool_id: &str) -> bool {
        match self {
            ReconcileScope::All => true,
            ReconcileScope::Tool(id) => id == tool_id,
        }
    }

    /// Whether a cached key may be evicted by this pass. A scoped pass only
    /// visited its own call, so it can only judge that call's keys — every other
    /// key is unexamined, not stale. Keys are `"{tool_call_id}#{index}"`.
    fn owns_key(&self, key: &str) -> bool {
        match self {
            ReconcileScope::All => true,
            ReconcileScope::Tool(id) => key
                .rsplit_once('#')
                .is_some_and(|(tool_id, _)| tool_id == id),
        }
    }
}

impl AgentChatView {
    /// Whether `tc`'s card body is on screen, and so whether its embed editors
    /// need to exist at all.
    ///
    /// Each embed editor is an `InputState`, and each `InputState` costs ~2 gpui
    /// focus handles. gpui walks the *whole* focus-handle slotmap on every effect
    /// drain (`App::release_dropped_focus_handles`, once per drained effect), and
    /// `slotmap`'s `retain` visits every slot ever allocated — the map never
    /// shrinks. So a handle that exists for a card nobody can see is a permanent
    /// per-frame tax for the rest of the session.
    ///
    /// A collapsed card renders no body: `FoldRow::block` only runs its body
    /// closure when expanded, and the diff / output blocks (with the `+N −M`
    /// stat) are built inside it. Since `FoldKey::Tool` is `ExpandedWhileActive`,
    /// every settled past card is collapsed — which is most of a long
    /// conversation. Mirrors the render's own gate exactly
    /// (`fold.is_expanded(&tool_fold_key(tc), fold_context_at(..))`); a nested
    /// subagent child is judged by its own key alone, which can only *over*-build
    /// (a child expanded under a collapsed parent), never leave a rendered body
    /// without its editor.
    ///
    /// `boundary` is resolved once per reconcile pass by the caller: deriving it
    /// per item would make a pass over `items` cost a scan of `items` each time.
    fn tool_body_on_screen(&self, ix: usize, item: &ChatItem, boundary: TurnBoundary) -> bool {
        let ChatItem::ToolCall(tc) = item else {
            return false;
        };
        let key = tool_fold_key(tc);
        self.fold
            .is_expanded(&key, fold_context_at(&key, ix, &self.items, boundary))
    }

    /// Re-run the embed reconcilers after a fold change, which is the other way
    /// (besides an ACP event) a card body can arrive on or leave the screen.
    ///
    /// `scope` narrows it to the toggled card when the caller knows which one it
    /// was; expand-all / collapse-all pass [`ReconcileScope::All`]. Diff embeds
    /// need the Workspace-resolved syntax theme, so they are skipped until the
    /// view has seen one — until then those diffs render through the inline
    /// fallback, which is correct, just unhighlighted.
    pub(in crate::workspace) fn reconcile_embeds_after_fold(
        &mut self,
        scope: &ReconcileScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A fold click runs inside the window's own update cycle, so the editors
        // have to be built against the borrow the caller already holds — see
        // [`WindowAccess`].
        let mut access = WindowAccess::Live(window);
        self.reconcile_output_editors(scope, &mut access, cx);
        if let Some(theme) = self.syntax_theme().map(str::to_owned) {
            let is_light = crate::ui::theme::agent_chat_syntax_is_light(cx);
            self.reconcile_diff_editors(&theme, is_light, scope, &mut access, cx);
        }
    }

    /// Rebuild the theme-dependent embeds after a UI theme swap.
    ///
    /// Only the diff embeds need it. They bake their palette into
    /// `set_highlight_override`, which is what the editor reads *instead of*
    /// resolving `cx.theme().highlight_theme` per paint — so a built diff embed
    /// cannot follow a swap on its own, and a dark→light switch would otherwise
    /// leave dark foregrounds on light hunk rows for the rest of the
    /// conversation. An output embed keeps the built-in highlighter and is
    /// already theme-live; rebuilding one would only churn it and drop its
    /// scroll position. Mermaid rasters are re-themed by their own pass.
    ///
    /// The swap reaches the reconciler as a moved fingerprint (see
    /// [`diff_theme_fingerprint`]), so this is a plain full pass — every embed
    /// whose inputs actually changed rebuilds, and nothing else does.
    ///
    /// Runs outside any window update: a global observer and the config-reload
    /// path both fire from `flush_effects`, after gpui has put the window back.
    pub(in crate::workspace) fn reconcile_embeds_after_theme_change(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let Some(theme) = self.syntax_theme().map(str::to_owned) else {
            // No syntax theme seen yet, so no diff embed has been built either.
            return;
        };
        let is_light = crate::ui::theme::agent_chat_syntax_is_light(cx);
        let mut access = WindowAccess::ByHandle(self.window_handle);
        self.reconcile_diff_editors(&theme, is_light, &ReconcileScope::All, &mut access, cx);
    }

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
    /// events. Comparing `diff_build_fingerprint` against the fingerprint
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
        scope: &ReconcileScope,
        access: &mut WindowAccess<'_>,
        cx: &mut Context<Self>,
    ) {
        let Some(colors) = cx.try_global::<crate::ui::theme::DarudaTheme>().map(|t| {
            let surface = crate::ui::theme::PaneSurfaceTokens::agent_chat(cx);
            DiffColors::from_agent_chat_surface(t, surface)
        }) else {
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

        // Shared by every diff in this pass, so hashed once rather than per diff.
        let theme = diff_theme_fingerprint(syntax_theme, is_light, &colors);

        // Collect the pure work first; entity creation re-enters the window,
        // which can't happen while the immutable `items` borrow is live.
        let mut pending: Vec<(String, u64, gpui::SharedString, DiffEditorModel, DiffStat)> =
            Vec::new();
        let mut live: HashSet<String> = HashSet::new();
        let boundary = TurnBoundary::of(&self.items);
        for (ix, item) in self.items.iter().enumerate() {
            let ChatItem::ToolCall(tc) = item else {
                continue;
            };
            if !scope.covers(&tc.id) || !self.tool_body_on_screen(ix, item, boundary) {
                continue;
            }
            for (di, diff) in tc.diffs.iter().enumerate() {
                let key = diff_editor_key(&tc.id, di);
                let fingerprint = diff_build_fingerprint(diff, theme);
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
        let stale = stale_keys(self.assets.diff_editors.keys(), &live, scope);
        if pending.is_empty() && stale.is_empty() {
            return;
        }

        for key in stale {
            self.assets.diff_editors.remove(&key);
            self.assets.diff_stats.remove(&key);
            self.assets.diff_editor_sources.remove(&key);
        }

        let pane_id = self.pane_id;
        for (key, fingerprint, language, model, stat) in pending {
            if let Some(editor) = create_diff_editor(cx, access, pane_id, &language, model) {
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
    pub(in crate::workspace) fn reconcile_output_editors(
        &mut self,
        scope: &ReconcileScope,
        access: &mut WindowAccess<'_>,
        cx: &mut Context<Self>,
    ) {
        // Collect the pure work first; entity creation re-enters the window,
        // which can't happen while the immutable `items` borrow is live.
        let mut pending: Vec<(String, u64, String, Option<String>)> = Vec::new();
        let mut live: HashSet<String> = HashSet::new();
        let boundary = TurnBoundary::of(&self.items);
        for (ix, item) in self.items.iter().enumerate() {
            let ChatItem::ToolCall(tc) = item else {
                continue;
            };
            if !scope.covers(&tc.id) || !self.tool_body_on_screen(ix, item, boundary) {
                continue;
            }
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
        let stale = stale_keys(self.assets.output_editors.keys(), &live, scope);
        if pending.is_empty() && stale.is_empty() {
            return;
        }

        for key in stale {
            self.assets.output_editors.remove(&key);
            self.assets.output_editor_sources.remove(&key);
        }

        let pane_id = self.pane_id;
        for (key, fingerprint, text, language) in pending {
            if let Some(editor) =
                create_output_editor(cx, access, pane_id, text, language.as_deref())
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
    pub(in crate::workspace) fn reconcile_tool_images(
        &mut self,
        scope: &ReconcileScope,
        cx: &mut Context<Self>,
    ) {
        // Collect the not-yet-cached, not-in-flight images first; the spawn
        // re-enters the view, which can't happen while the `items` borrow is
        // live.
        let mut pending: Vec<(u64, String)> = Vec::new();
        for item in &self.items {
            let ChatItem::ToolCall(tc) = item else {
                continue;
            };
            if !scope.covers(&tc.id) {
                continue;
            }
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
fn stale_keys<'a>(
    cached: impl Iterator<Item = &'a String>,
    live: &HashSet<String>,
    scope: &ReconcileScope,
) -> Vec<String> {
    cached
        .filter(|key| scope.owns_key(key) && !live.contains(*key))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use daruda_acp::{
        ChatItem, DiffView, ToolCallItem, ToolKindView, ToolOutputBlock, ToolStatusView,
    };
    use gpui::TestAppContext;

    use super::super::view::tests::make_test_view;
    use super::super::window_access::WindowAccess;
    use super::{ReconcileScope, mermaid_key};
    use crate::workspace::main_area::file_view_pane::diff_editor::DiffColors;

    const KEY: &str = "call_1#0";

    /// The reconcilers' non-fold caller (an async ACP event) resolves the window
    /// from the stored handle. `window_handle` is `Copy`, so this borrows nothing
    /// the caller still needs.
    fn by_handle(v: &super::AgentChatView) -> WindowAccess<'static> {
        WindowAccess::ByHandle(v.window_handle)
    }

    /// A syntax theme id the diff reconciler's highlight pass can resolve.
    const SYNTAX_THEME: &str = "base16-ocean.dark";

    /// A theme swap has to reach the diff embeds, and reach *only* them.
    ///
    /// A diff embed bakes its palette in at build time: `set_highlight_override`
    /// replaces the editor's own highlighter, and that override is what
    /// `input/element.rs` reads instead of resolving
    /// `cx.theme().highlight_theme` per paint. So nothing about a swap reaches a
    /// built diff embed unless the reconciler rebuilds it — a dark→light swap
    /// would otherwise leave dark foregrounds on light hunk rows for the rest of
    /// the conversation. An output embed keeps the built-in highlighter and is
    /// already theme-live, so rebuilding one would be pure churn (and would drop
    /// its scroll position).
    #[gpui::test]
    fn a_theme_swap_rebuilds_diff_embeds_and_leaves_output_embeds_alone(cx: &mut TestAppContext) {
        let window = make_test_view(cx);
        let view = window.root(cx).expect("the view is the window root");

        cx.update(|cx| crate::ui::theme::set_agent_chat_bg(cx, 0, 0, 0));
        view.update(cx, |v, cx| {
            v.set_syntax_theme(SYNTAX_THEME);
            v.items = vec![tool_call_with(
                vec![raw("out")],
                vec![diff("a.rs", "first\n")],
            )];
            let is_light = crate::ui::theme::agent_chat_syntax_is_light(cx);
            v.reconcile_diff_editors(
                SYNTAX_THEME,
                is_light,
                &ReconcileScope::All,
                &mut by_handle(v),
                cx,
            );
            v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
        });
        let built = |cx: &mut TestAppContext| {
            view.read_with(cx, |v, _| {
                (
                    v.assets.diff_editors[KEY].entity_id(),
                    v.assets.output_editors[KEY].entity_id(),
                )
            })
        };
        let dark = built(cx);

        // Same diff, different theme — the content-only rule would skip this.
        cx.update(|cx| crate::ui::theme::set_agent_chat_bg(cx, 255, 255, 255));
        view.update(cx, |v, cx| v.reconcile_embeds_after_theme_change(cx));
        let light = built(cx);

        assert_ne!(
            dark.0, light.0,
            "a diff embed's baked palette only tracks a theme swap through a rebuild"
        );
        assert_eq!(
            dark.1, light.1,
            "an output embed resolves colours per paint, so a swap must not churn it"
        );
    }

    /// And the swap has to *reach* that pass on its own. A UI-preset switch goes
    /// through `apply_ui_theme`, which only re-sets the `DarudaTheme` global — so
    /// the view's own observer is the whole delivery path.
    #[gpui::test]
    fn setting_the_theme_global_rebuilds_diff_embeds_through_the_observer(cx: &mut TestAppContext) {
        let window = make_test_view(cx);
        let view = window.root(cx).expect("the view is the window root");

        view.update(cx, |v, cx| {
            v.set_syntax_theme(SYNTAX_THEME);
            v.items = vec![diff_tool_call(vec![diff("a.rs", "first\n")])];
            let is_light = crate::ui::theme::agent_chat_syntax_is_light(cx);
            v.reconcile_diff_editors(
                SYNTAX_THEME,
                is_light,
                &ReconcileScope::All,
                &mut by_handle(v),
                cx,
            );
        });
        let before = view.read_with(cx, |v, _| v.assets.diff_editors[KEY].entity_id());

        cx.update(|cx| {
            // Any diff colour the palette snapshot carries will do; this one is
            // read straight into `DiffColors::add_bg`.
            cx.set_global(crate::ui::theme::DarudaTheme {
                file_diff_add_bg: gpui::hsla(0.5, 0.5, 0.5, 1.0),
                ..Default::default()
            });
        });
        cx.run_until_parked();

        assert_ne!(
            before,
            view.read_with(cx, |v, _| v.assets.diff_editors[KEY].entity_id()),
            "replacing the theme global must reach the diff embeds through the \
             view's own observer — nothing else delivers a UI-preset switch"
        );
    }

    fn tool_call(output: Vec<ToolOutputBlock>) -> ChatItem {
        tool_call_with(output, Vec::new())
    }

    fn diff_tool_call(diffs: Vec<DiffView>) -> ChatItem {
        tool_call_with(Vec::new(), diffs)
    }

    /// Live (`InProgress`), so `ExpandedWhileActive` leaves the card expanded and
    /// its body — the diff / output blocks the embeds back — is on screen. A
    /// settled card is collapsed and by design has no embeds at all (see
    /// `a_collapsed_tool_card_gets_no_embed_editors`), so the editor-lifecycle
    /// tests below have to start from a live card.
    fn tool_call_with(output: Vec<ToolOutputBlock>, diffs: Vec<DiffView>) -> ChatItem {
        ChatItem::ToolCall(ToolCallItem {
            id: "call_1".to_string(),
            title: "Bash".to_string(),
            kind: ToolKindView::Execute,
            tool_name: None,
            status: ToolStatusView::InProgress,
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

    /// Settled, so its card is collapsed and renders no body.
    fn settled_tool(id: &str, output: Vec<ToolOutputBlock>) -> ChatItem {
        let ChatItem::ToolCall(mut tc) = tool_named(id, output) else {
            unreachable!("tool_named builds a ToolCall")
        };
        tc.status = ToolStatusView::Completed;
        ChatItem::ToolCall(tc)
    }

    /// Each embed editor is an `InputState`, and each `InputState` costs ~2 gpui
    /// focus handles whose slot gpui's `release_dropped_focus_handles` walks on
    /// every effect drain — a per-conversation, never-shrinking tax. A collapsed
    /// tool card renders no body at all (`FoldRow::block` only runs the body
    /// closure when expanded), so an editor built for one is pure cost.
    #[gpui::test]
    fn a_collapsed_tool_card_gets_no_embed_editors(cx: &mut TestAppContext) {
        let window = make_test_view(cx);
        let view = window.root(cx).expect("the view is the window root");

        view.update(cx, |v, cx| {
            // Settled → `ExpandedWhileActive` derives collapsed, and no user
            // override says otherwise.
            v.items = vec![settled_tool("call_a", vec![raw("big output")])];
            v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
        });
        view.read_with(cx, |v, _| {
            assert!(
                v.assets.output_editors.is_empty(),
                "a collapsed card renders no body, so it needs no editor"
            );
        });
    }

    /// The counterpart: a card whose body *is* on screen must have its editor, or
    /// the body falls back to the inline per-line text walk the embed exists to
    /// avoid. A live card is expanded by `ExpandedWhileActive`.
    #[gpui::test]
    fn a_live_tool_cards_editors_are_materialized(cx: &mut TestAppContext) {
        let window = make_test_view(cx);
        let view = window.root(cx).expect("the view is the window root");

        view.update(cx, |v, cx| {
            v.items = vec![tool_named("call_a", vec![raw("streaming output")])];
            v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
        });
        view.read_with(cx, |v, _| {
            assert!(v.assets.output_editors.contains_key("call_a#0"));
        });
    }

    /// Materialization has to follow the fold both ways: expanding a settled card
    /// builds its editors, collapsing it again releases them (and their focus
    /// handles).
    ///
    /// Driven through `window.update`, which is where the app drives it from: a
    /// fold click arrives via `cx.listener`, i.e. from *inside* the window's own
    /// update cycle. gpui takes the window out of `App::windows` for the duration
    /// of an update, so an editor built by re-entering `update_window` there is
    /// silently dropped ("window not found") and the body falls back to the
    /// markdown walk the embed exists to avoid. Toggling through `Entity::update`
    /// instead is outside that cycle and cannot see it.
    #[gpui::test]
    fn folding_a_tool_card_materializes_and_releases_its_editors(cx: &mut TestAppContext) {
        use super::super::fold::FoldKey;

        let window = make_test_view(cx);
        let view = window.root(cx).expect("the view is the window root");

        view.update(cx, |v, cx| {
            v.items = vec![settled_tool("call_a", vec![raw("out")])];
            v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
        });
        view.read_with(cx, |v, _| assert!(v.assets.output_editors.is_empty()));

        let click = |cx: &mut TestAppContext| {
            window
                .update(cx, |v, window, cx| {
                    v.toggle_fold(FoldKey::Tool("call_a".into()), window, cx);
                })
                .expect("the window is open");
        };

        click(cx);
        view.read_with(cx, |v, _| {
            assert!(
                v.assets.output_editors.contains_key("call_a#0"),
                "expanding a card must build the editor its body needs"
            );
        });

        click(cx);
        view.read_with(cx, |v, _| {
            assert!(
                v.assets.output_editors.is_empty(),
                "collapsing releases the editor again"
            );
        });
    }

    /// The property the fix is for: live editors — and so live focus handles —
    /// track what is on screen, not how long the conversation has run.
    #[gpui::test]
    fn live_embed_editors_stay_bounded_as_the_conversation_grows(cx: &mut TestAppContext) {
        let window = make_test_view(cx);
        let view = window.root(cx).expect("the view is the window root");

        let mut items: Vec<ChatItem> = (0..200)
            .map(|i| settled_tool(&format!("call_{i}"), vec![raw("settled output")]))
            .collect();
        items.push(tool_named("call_live", vec![raw("streaming")]));
        view.update(cx, |v, cx| {
            v.items = items;
            v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
        });
        view.read_with(cx, |v, _| {
            assert_eq!(
                v.assets.output_editors.len(),
                1,
                "only the in-flight card's body is on screen"
            );
        });
    }

    fn tool_named(id: &str, output: Vec<ToolOutputBlock>) -> ChatItem {
        let ChatItem::ToolCall(mut tc) = tool_call(output) else {
            unreachable!("tool_call builds a ToolCall")
        };
        tc.id = id.to_string();
        ChatItem::ToolCall(tc)
    }

    /// A `ToolCallUpdate` names the one call it replaced, so a reconcile pass
    /// scoped to it must not touch — least of all evict — any other call's cached
    /// editors. Getting this wrong is the failure mode scoping introduces: the
    /// full pass derived staleness from a whole-conversation `live` set, and a
    /// scoped pass that kept doing so would drop every other tool's editor.
    #[gpui::test]
    fn a_scoped_reconcile_leaves_other_tools_editors_alone(cx: &mut TestAppContext) {
        let window = make_test_view(cx);
        let view = window.root(cx).expect("the view is the window root");

        view.update(cx, |v, cx| {
            v.items = vec![
                tool_named("call_a", vec![raw("a output")]),
                tool_named("call_b", vec![raw("b output")]),
            ];
            v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
        });
        let a_editor = view.read_with(cx, |v, _| {
            v.assets
                .output_editors
                .get("call_a#0")
                .map(|e| e.entity_id())
                .expect("the full pass builds both editors")
        });

        // Only call_b streamed more output.
        view.update(cx, |v, cx| {
            v.items = vec![
                tool_named("call_a", vec![raw("a output")]),
                tool_named("call_b", vec![raw("b output\nmore")]),
            ];
            v.reconcile_output_editors(
                &ReconcileScope::Tool("call_b".into()),
                &mut by_handle(v),
                cx,
            );
        });
        view.read_with(cx, |v, cx| {
            assert_eq!(
                v.assets
                    .output_editors
                    .get("call_a#0")
                    .map(|e| e.entity_id()),
                Some(a_editor),
                "an untouched tool keeps its editor entity"
            );
            assert_eq!(
                v.assets.output_editors["call_b#0"].read(cx).value(),
                "b output\nmore",
                "the scoped tool still rebuilds on grown content"
            );
        });
    }

    /// Scoping narrows *which* calls are visited, not the eviction guarantee for
    /// the call being visited: a shrunken `output` vec must still drop the keys
    /// its indexes no longer reach.
    #[gpui::test]
    fn a_scoped_reconcile_still_evicts_its_own_vanished_keys(cx: &mut TestAppContext) {
        let window = make_test_view(cx);
        let view = window.root(cx).expect("the view is the window root");

        view.update(cx, |v, cx| {
            v.items = vec![
                tool_named("call_a", vec![raw("keep me")]),
                tool_named("call_b", vec![raw("first"), raw("second")]),
            ];
            v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
        });
        view.read_with(cx, |v, _| assert_eq!(v.assets.output_editors.len(), 3));

        view.update(cx, |v, cx| {
            v.items = vec![
                tool_named("call_a", vec![raw("keep me")]),
                tool_named("call_b", vec![raw("first")]),
            ];
            v.reconcile_output_editors(
                &ReconcileScope::Tool("call_b".into()),
                &mut by_handle(v),
                cx,
            );
        });
        view.read_with(cx, |v, _| {
            assert!(
                !v.assets.output_editors.contains_key("call_b#1"),
                "an index the shrunken output vec no longer reaches is dropped"
            );
            assert!(v.assets.output_editors.contains_key("call_b#0"));
            assert!(
                v.assets.output_editors.contains_key("call_a#0"),
                "the untouched tool is unaffected by the eviction"
            );
        });
    }

    /// `All` is what a wholesale `items` replacement (connect / load catch-up)
    /// needs: a tool that left the conversation entirely takes its editors with
    /// it. No scoped pass can see that, which is why the variant exists.
    #[gpui::test]
    fn the_all_scope_evicts_a_tool_that_left_the_conversation(cx: &mut TestAppContext) {
        let window = make_test_view(cx);
        let view = window.root(cx).expect("the view is the window root");

        view.update(cx, |v, cx| {
            v.items = vec![
                tool_named("call_a", vec![raw("a")]),
                tool_named("call_b", vec![raw("b")]),
            ];
            v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
        });
        view.update(cx, |v, cx| {
            v.items = vec![tool_named("call_a", vec![raw("a")])];
            v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
        });
        view.read_with(cx, |v, _| {
            assert!(v.assets.output_editors.contains_key("call_a#0"));
            assert!(
                !v.assets.output_editors.contains_key("call_b#0"),
                "a vanished tool's editors are dropped by the full pass"
            );
        });
    }

    /// The cost this scoping exists to remove: the full pass fingerprints every
    /// diff's whole `old_text` + `new_text` in the conversation, so a streamed
    /// `ToolCallUpdate` re-hashed every diff ever produced — quadratic over a
    /// turn. A scoped pass must touch one call's content, not all of it.
    #[gpui::test]
    fn a_scoped_diff_reconcile_does_not_rehash_the_whole_conversation(cx: &mut TestAppContext) {
        let window = make_test_view(cx);
        let view = window.root(cx).expect("the view is the window root");

        // 200 settled calls, ~128 KiB of diff text each: ~25 MiB the full pass
        // re-fingerprints per event.
        let body = "fn main() { let x = 1; }\n".repeat(5_000);
        let seeded: Vec<ChatItem> = (0..200)
            .map(|i| {
                let ChatItem::ToolCall(mut tc) = diff_tool_call(vec![diff("a.rs", &body)]) else {
                    unreachable!("diff_tool_call builds a ToolCall")
                };
                tc.id = format!("call_{i}");
                ChatItem::ToolCall(tc)
            })
            .collect();
        // The pass will fingerprint each diff against this same palette, so the
        // pre-seed below has to use it too or every key would look changed.
        let theme = cx.update(|cx| {
            let surface = crate::ui::theme::PaneSurfaceTokens::agent_chat(cx);
            let colors = DiffColors::from_agent_chat_surface(
                cx.global::<crate::ui::theme::DarudaTheme>(),
                surface,
            );
            super::diff_theme_fingerprint(SYNTAX_THEME, false, &colors)
        });
        view.update(cx, |v, _| {
            v.items = seeded;
            // Pre-seed matching fingerprints so neither pass builds an editor.
            // What is left is exactly the change-detection walk — the cost
            // scoping exists to remove, isolated from the one legitimate rebuild
            // a real update would also do.
            for item in &v.items {
                let ChatItem::ToolCall(tc) = item else {
                    unreachable!("every seeded item is a tool call")
                };
                for (di, d) in tc.diffs.iter().enumerate() {
                    v.assets.diff_editor_sources.insert(
                        super::diff_editor_key(&tc.id, di),
                        super::diff_build_fingerprint(d, theme),
                    );
                }
            }
        });

        let time = |cx: &mut TestAppContext, scope: ReconcileScope| {
            let started = std::time::Instant::now();
            view.update(cx, |v, cx| {
                v.reconcile_diff_editors(SYNTAX_THEME, false, &scope, &mut by_handle(v), cx);
            });
            started.elapsed()
        };
        let full = time(cx, ReconcileScope::All);
        let scoped = time(cx, ReconcileScope::Tool("call_0".into()));
        assert!(
            scoped * 10 < full,
            "scoped {scoped:?} vs full {full:?} — the scoped pass is still \
             walking the whole conversation"
        );
    }

    /// The reconciler owns the whole editor lifecycle: one entity per
    /// qualifying output block, reused while the content is unchanged, rebuilt
    /// when a streamed output grows, and dropped once the block (or its index)
    /// is gone. Driven the way an ACP event drives it — outside any window
    /// update, resolving the window from the stored handle. The fold-click path
    /// (inside the cycle) is covered by
    /// `a_fold_click_inside_the_window_update_cycle_builds_its_editors`.
    #[gpui::test]
    fn output_editors_are_built_reused_rebuilt_and_dropped(cx: &mut TestAppContext) {
        let window = make_test_view(cx);
        let view = window.root(cx).expect("the view is the window root");

        let reconcile_with = |cx: &mut TestAppContext, output: Vec<ToolOutputBlock>| {
            view.update(cx, |v, cx| {
                v.items = vec![tool_call(output)];
                v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
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
                v.reconcile_diff_editors(
                    SYNTAX_THEME,
                    false,
                    &ReconcileScope::All,
                    &mut by_handle(v),
                    cx,
                );
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
