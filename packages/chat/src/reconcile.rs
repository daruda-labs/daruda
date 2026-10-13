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

use daruda_acp::ChatItem;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::Context;

use super::agent_chat_helpers::{
    DiffStat, TurnBoundary, build_diff_view_model, chat_item_mermaid_texts, create_diff_editor,
    diff_build_fingerprint, diff_editor_key, diff_editor_language, diff_theme_fingerprint,
    fold_context_at, mermaid_key, mermaid_sources, tool_fold_key,
};
use super::output_editor::{
    create_output_editor, output_editor_key, output_editor_source, output_source_fingerprint,
};
use super::view::AgentChatView;
use super::view::list_sync::ListSync;
use super::window_access::WindowAccess;
use daruda_content::visual;
use daruda_ui::CachedImage;
use daruda_ui::diff_editor::{DiffColors, DiffEditorModel};
use daruda_ui::mermaid_palette;

/// Which tool calls a reconcile pass must revisit.
///
/// The passes below detect "did this content change" by fingerprinting it, so
/// visiting a call is not free — it costs the call's whole diff / output text.
/// A streamed `ToolCallUpdate` replaces exactly one call (`apply_tool_call_update`
/// in `daruda_acp::mapping`) and names it, so re-fingerprinting the rest of the
/// conversation buys nothing and makes a long turn quadratic: every chunk
/// re-hashed every diff the turn had produced so far.
#[derive(Debug)]
pub(super) enum ReconcileScope {
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

mod tool_images;

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
    /// stat) are built inside it. `FoldKey::Tool` opens only by a fold rule or a
    /// click, so most cards of a long conversation are collapsed. Mirrors the
    /// render's own gate exactly
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
        self.fold.is_expanded(
            &key,
            fold_context_at(&key, ix, &self.items, boundary, &self.live_units),
        )
    }

    /// Re-run the embed reconcilers after a fold change, which is the other way
    /// (besides an ACP event) a card body can arrive on or leave the screen.
    ///
    /// Every caller that moves the fold owes this pass: the renderer only *reads*
    /// the embed caches, so a body that arrives on screen without it renders
    /// through the inline fallbacks — a diff as plain `+`/`-` lines with no
    /// gutter and no syntax colour — for as long as no ACP event happens to
    /// arrive.
    ///
    /// `scope` narrows it to the toggled card when the caller knows which one it
    /// was; a change that moves the whole matrix at once (expand-all /
    /// collapse-all, a fold-mode edit, a reseeded default) passes
    /// [`ReconcileScope::All`].
    ///
    /// `access` is how the caller says which world it is in: a fold *click* runs
    /// inside the window's own update cycle and has to hand over the borrow it
    /// already holds, while the config-reload path fires from `flush_effects`
    /// and resolves one from the stored handle — see [`WindowAccess`].
    pub(super) fn reconcile_embeds_after_fold(
        &mut self,
        scope: &ReconcileScope,
        access: &mut WindowAccess<'_>,
        cx: &mut Context<Self>,
    ) {
        self.reconcile_tool_images(scope, cx);
        self.reconcile_output_editors(scope, access, cx);
        let theme = self.syntax_theme().to_owned();
        let is_light = daruda_ui::theme::agent_chat_syntax_is_light(cx);
        self.reconcile_diff_editors(&theme, is_light, scope, access, cx);
    }

    /// Rebuild every content-derived embed for the whole conversation.
    ///
    /// The `items`-replaced-wholesale counterpart to `apply_event`'s passes: a
    /// caller that swaps the entire transcript has neither a call to scope to
    /// nor an event flag to gate on, so it owes the full pass.
    #[cfg(feature = "devtools")]
    pub(super) fn reconcile_all_embeds(
        &mut self,
        access: &mut WindowAccess<'_>,
        cx: &mut Context<Self>,
    ) {
        let scope = ReconcileScope::All;
        let theme = self.syntax_theme().to_owned();
        let is_light = daruda_ui::theme::agent_chat_syntax_is_light(cx);
        self.reconcile_diff_editors(&theme, is_light, &scope, access, cx);
        self.reconcile_tool_images(&scope, cx);
        self.reconcile_output_editors(&scope, access, cx);
        self.reconcile_mermaid(Self::host_is_dark(cx), cx);
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
    /// `diff_theme_fingerprint`), so this is a plain full pass — every embed
    /// whose inputs actually changed rebuilds, and nothing else does.
    ///
    /// Runs outside any window update: a global observer and the config-reload
    /// path both fire from `flush_effects`, after gpui has put the window back.
    /// Re-raster every mermaid diagram after the terminal palette changed:
    /// the cached images carry the old colours.
    pub fn rerender_mermaid_after_palette_change(&mut self, cx: &mut Context<Self>) {
        self.assets.clear_mermaid();
        self.reconcile_mermaid(Self::host_is_dark(cx), cx);
    }

    pub fn reconcile_embeds_after_theme_change(&mut self, cx: &mut Context<Self>) {
        let theme = self.syntax_theme().to_owned();
        let is_light = daruda_ui::theme::agent_chat_syntax_is_light(cx);
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
    pub(super) fn reconcile_diff_editors(
        &mut self,
        syntax_theme: &str,
        is_light: bool,
        scope: &ReconcileScope,
        access: &mut WindowAccess<'_>,
        cx: &mut Context<Self>,
    ) {
        let Some(colors) = cx.try_global::<daruda_ui::theme::DarudaTheme>().map(|t| {
            let surface = daruda_ui::theme::PaneSurfaceTokens::agent_chat(cx);
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
        self.apply_list_sync(ListSync::EveryRow, "diff-embed");
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
    pub(super) fn reconcile_output_editors(
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
        self.apply_list_sync(ListSync::EveryRow, "output-embed");
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
    pub(crate) fn reconcile_mermaid(&mut self, dark: bool, cx: &mut Context<Self>) {
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
        // touches `Hsla` / the `DarudaTheme` global — see `daruda_content::MermaidPalette`.
        let palette = mermaid_palette::agent_chat_palette(cx);

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
                    if let Some(image) = raster.and_then(CachedImage::from_raster) {
                        view.assets
                            .mermaid_images
                            .lock()
                            .unwrap()
                            .insert(key, image);
                        // The fence grew from a code block to a diagram, so its
                        // cached height is stale — remeasure before repainting or
                        // it clips. Index is unknown here, so this is a full
                        // remeasure, but it's one-shot per landed raster.
                        view.apply_list_sync(ListSync::EveryRow, "mermaid-raster");
                        cx.notify();
                    }
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
mod tests;
