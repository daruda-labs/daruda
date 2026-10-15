//! Tool-output image decoding: inline `Image` blocks and bounded, authorized
//! previews of local-image resource links.

use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Read as _;
use std::path::{Path, PathBuf};

use daruda_acp::{ChatItem, ToolOutputBlock};
use daruda_content::link_target;
use daruda_content::visual;
use daruda_ui::CachedImage;
use gpui::Context;

use super::super::agent_chat_helpers::{TurnBoundary, tool_image_key};
use super::super::output_editor::output_editor_key;
use super::super::view::AgentChatView;
use super::{ReconcileScope, output_content, stale_keys};

/// Local resource previews are a fallback for adapter-emitted file links, not
/// a general URI fetcher. Keep reads bounded before bytes reach an image
/// decoder; decoded allocations are bounded separately in
/// `visual::decode_image_bounded`.
pub(super) const MAX_RESOURCE_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
/// Bound both retained textures and work spawned by an expand-all/replay. The
/// latest expanded previews win; older resources remain usable as links.
pub(super) const MAX_RESOURCE_IMAGE_PREVIEWS: usize = 4;
const MAX_RESOURCE_IMAGE_INFLIGHT: usize = 2;
const MAX_RESOURCE_IMAGE_DIMENSION: u32 = 4096;
const MAX_RESOURCE_IMAGE_ALLOC_BYTES: u64 = 64 * 1024 * 1024;

/// Resolved by the same rules the resource link's click uses, so the preview
/// and the opener cannot disagree about which file — or whether an image.
pub(super) fn resource_image_path(uri: &str, mime: Option<&str>, cwd: &Path) -> Option<PathBuf> {
    let path = link_target::resource_path(uri, Some(cwd))?;
    link_target::is_image_resource(&path, mime).then_some(path)
}

fn resource_image_source(uri: &str, mime: Option<&str>) -> u64 {
    let mut hasher = DefaultHasher::new();
    uri.hash(&mut hasher);
    mime.hash(&mut hasher);
    hasher.finish()
}

/// Canonicalization closes symlink escapes. Restricting reads to the pane cwd
/// and OS temp roots keeps an untrusted resource link from turning the GUI into
/// an arbitrary local-file reader; Codex `view_image` outputs live in temp.
pub(super) fn authorized_resource_image_path(path: &Path, cwd: &Path) -> Option<PathBuf> {
    let path = daruda_core::path::canonicalize(path).ok()?;
    let cwd = daruda_core::path::canonicalize(cwd).ok()?;
    if path.starts_with(cwd) {
        return Some(path);
    }

    let temp_roots =
        std::iter::once(std::env::temp_dir()).chain(cfg!(unix).then(|| PathBuf::from("/tmp")));
    temp_roots
        .into_iter()
        .filter_map(|root| daruda_core::path::canonicalize(root).ok())
        .any(|root| path.starts_with(root))
        .then_some(path)
}

pub(super) fn load_resource_image(path: &Path, cwd: &Path) -> Option<visual::RasterImage> {
    let path = authorized_resource_image_path(path, cwd)?;
    let file = std::fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_RESOURCE_IMAGE_BYTES {
        return None;
    }

    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_RESOURCE_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_RESOURCE_IMAGE_BYTES {
        return None;
    }
    visual::decode_image_bounded(
        &bytes,
        MAX_RESOURCE_IMAGE_DIMENSION,
        MAX_RESOURCE_IMAGE_ALLOC_BYTES,
    )
    .ok()
}

impl AgentChatView {
    /// Decode every inline `Image` block and eligible local-image
    /// `ResourceLink` that does not yet have a cached bitmap. Mirrors
    /// [`Self::reconcile_mermaid`]'s collect-then-spawn shape, minus the `dark`
    /// theming (tool images are rendered as-is, not re-themed). Local resource
    /// files are read only after the call settles, on the background executor.
    ///
    /// A decode failure caches `None` under the key rather than leaving it
    /// absent, so malformed content is not retried forever. Inline images show
    /// their media descriptor; resource images retain their original link.
    pub(crate) fn reconcile_tool_images(&mut self, scope: &ReconcileScope, cx: &mut Context<Self>) {
        // Collect the not-yet-cached, not-in-flight images first; the spawn
        // re-enters the view, which can't happen while the `items` borrow is
        // live.
        let mut pending: Vec<(u64, String)> = Vec::new();
        for item in &self.items {
            let Some((id, output)) = output_content(item) else {
                continue;
            };
            if !scope.covers(&id) {
                continue;
            }
            for block in output {
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

        let local_cwd = self
            .cwd
            .as_ref()
            .and_then(daruda_store::project::PaneCwd::as_local)
            .map(Path::to_path_buf);
        let mut resource_live = HashSet::new();
        let mut resource_candidates: Vec<(String, u64, PathBuf, PathBuf)> = Vec::new();
        if let Some(cwd) = local_cwd.as_ref() {
            let boundary = TurnBoundary::of(&self.items);
            for (item_ix, item) in self.items.iter().enumerate() {
                let Some((id, output)) = output_content(item) else {
                    continue;
                };
                if !scope.covers(&id)
                    || matches!(item, ChatItem::ToolCall(tc) if tc.status.is_live())
                    || !self.tool_body_on_screen(item_ix, item, boundary)
                {
                    continue;
                }
                for (ix, block) in output.iter().enumerate() {
                    let ToolOutputBlock::ResourceLink { uri, mime, .. } = block else {
                        continue;
                    };
                    let Some(path) = resource_image_path(uri, mime.as_deref(), cwd) else {
                        continue;
                    };
                    let key = output_editor_key(&id, ix);
                    let source = resource_image_source(uri, mime.as_deref());
                    resource_candidates.push((key, source, path, cwd.clone()));
                }
            }
        }

        // Preserve the most recent expanded resources deterministically. In a
        // full pass this also keeps the LRU walk below from briefly evicting and
        // re-adding every retained key when the transcript has more candidates
        // than the cache can hold.
        if resource_candidates.len() > MAX_RESOURCE_IMAGE_PREVIEWS {
            resource_candidates.drain(..resource_candidates.len() - MAX_RESOURCE_IMAGE_PREVIEWS);
        }
        resource_live.extend(resource_candidates.iter().map(|(key, ..)| key.clone()));

        let stale = stale_keys(
            self.assets.resource_image_sources.keys(),
            &resource_live,
            scope,
        );
        for key in stale {
            self.assets.resource_image_sources.remove(&key);
            self.assets
                .resource_image_order
                .retain(|cached| cached != &key);
            self.assets.resource_images.lock().unwrap().remove(&key);
        }

        let mut resource_pending: Vec<(String, u64, PathBuf, PathBuf)> = Vec::new();
        for (key, source, path, cwd) in resource_candidates {
            let source_changed = self.assets.resource_image_sources.get(&key) != Some(&source);
            if source_changed {
                self.assets
                    .resource_image_sources
                    .insert(key.clone(), source);
                self.assets.resource_images.lock().unwrap().remove(&key);
            }

            self.assets
                .resource_image_order
                .retain(|cached| cached != &key);
            self.assets.resource_image_order.push_back(key.clone());
            while self.assets.resource_image_order.len() > MAX_RESOURCE_IMAGE_PREVIEWS {
                let Some(evicted) = self.assets.resource_image_order.pop_front() else {
                    break;
                };
                self.assets.resource_image_sources.remove(&evicted);
                self.assets.resource_images.lock().unwrap().remove(&evicted);
            }

            let cached = self
                .assets
                .resource_images
                .lock()
                .unwrap()
                .contains_key(&key);
            let in_flight = self.assets.resource_image_inflight.get(&key);
            if !cached && in_flight.is_none() {
                resource_pending.push((key, source, path, cwd));
            }
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
                    let cached = raster.and_then(CachedImage::from_raster);
                    view.assets.tool_images.lock().unwrap().insert(key, cached);
                    // The block just resolved from a pending placeholder to a
                    // decoded image (or a failure label), so its cached height
                    // is stale — remeasure before repainting or it clips.
                    // Index is unknown here, so this is a full remeasure, but
                    // it's one-shot per landed decode.
                    view.resync_all_row_heights("tool-image");
                    cx.notify();
                });
            })
            .detach();
        }

        let available =
            MAX_RESOURCE_IMAGE_INFLIGHT.saturating_sub(self.assets.resource_image_inflight.len());
        let to_spawn: Vec<_> = resource_pending
            .into_iter()
            .filter(|(key, source, ..)| self.assets.resource_image_sources.get(key) == Some(source))
            .take(available)
            .collect();
        for (key, source, path, cwd) in to_spawn {
            self.assets
                .resource_image_inflight
                .insert(key.clone(), source);

            cx.spawn(async move |this, cx| {
                let raster = cx
                    .background_executor()
                    .spawn(async move {
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            load_resource_image(&path, &cwd)
                        }))
                        .ok()
                        .flatten()
                    })
                    .await;
                // SILENT-OK: view/window dropped before the read resolved.
                let _ = this.update(cx, |view, cx| {
                    // Only the task that owns this slot may clear it. A newer
                    // source at the same block index waits until this read
                    // finishes, keeping the global concurrency bound honest.
                    if view.assets.resource_image_inflight.get(&key) != Some(&source) {
                        return;
                    }
                    view.assets.resource_image_inflight.remove(&key);
                    if view.assets.resource_image_sources.get(&key) == Some(&source) {
                        let cached = raster.and_then(CachedImage::from_raster);
                        view.assets
                            .resource_images
                            .lock()
                            .unwrap()
                            .insert(key, cached);
                        view.resync_all_row_heights("resource-image");
                        cx.notify();
                    }
                    // Fill the next queued preview now that one of the bounded
                    // decoder slots is free. The pass is idempotent for every
                    // cached/in-flight entry.
                    view.reconcile_tool_images(&ReconcileScope::All, cx);
                });
            })
            .detach();
        }
    }
}
