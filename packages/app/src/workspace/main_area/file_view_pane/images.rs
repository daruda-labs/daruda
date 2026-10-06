//! The file pane's GPU image table for Markdown preview.
//!
//! [`MdImages`] is built once when a load lands — the conversion happens here
//! and never in `render()` — and is indexed by the `slot` the resolve pass
//! stamped into the IR. [`FileContent::install_content`] and
//! [`FileContent::release_images`] are the pane's only two entry points, so
//! every table has exactly one birth and one death: `RenderImage` has no
//! `Drop` and the Metal sprite atlas never evicts on its own, so an image that
//! is not released leaks a texture for the process's lifetime.

use gpui::{Context, Window};

use super::PaneFileContent;
use super::render::CachedImage;
use super::visual::RasterImage;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::{FileContent, Pane};
use crate::workspace::main_area::pane_tree::PaneId;

/// GPU-ready Markdown images for one file pane, one entry per resolve slot.
/// `None` means the load or the conversion failed and the renderer falls back
/// to the alt text (or, for a diagram, the raw mermaid source).
#[derive(Default)]
pub(in crate::workspace) struct MdImages(Vec<Option<CachedImage>>);

impl MdImages {
    /// Convert a resolve pass's rasters into GPU images, keeping slot order.
    pub(in crate::workspace) fn from_rasters(rasters: Vec<Option<RasterImage>>) -> Self {
        Self(
            rasters
                .into_iter()
                .map(|raster| raster.and_then(CachedImage::from_raster))
                .collect(),
        )
    }

    /// The image behind `slot`, or `None` when it never loaded.
    ///
    /// An empty table is the released, never-loaded or no-images state — every
    /// teardown takes the table while the content whose slots index it is
    /// still painted, so a frame in between legitimately finds nothing. A slot
    /// past the end of a non-empty table cannot happen through the load funnel
    /// (`resolve_all` numbers into the table it returns, and `LoadOutcome`
    /// carries the pair), so it is a wiring bug the assertion catches.
    pub(in crate::workspace) fn get(&self, slot: u32) -> Option<&CachedImage> {
        if self.0.is_empty() {
            return None;
        }
        debug_assert!(
            (slot as usize) < self.0.len(),
            "markdown image slot {slot} is outside a table of {}",
            self.0.len()
        );
        self.0.get(slot as usize).and_then(Option::as_ref)
    }
}

impl FileContent {
    /// Install a finished load: the previous frame's GPU images go first, then
    /// the content and the table its slots index into.
    ///
    /// `window` — see [`Self::release_images`].
    pub(in crate::workspace) fn install_content(
        &mut self,
        content: PaneFileContent,
        rasters: Vec<Option<RasterImage>>,
        window: Option<&mut Window>,
        cx: &mut Context<Workspace>,
    ) {
        self.release_images(window, cx);
        self.images = MdImages::from_rasters(rasters);
        self.view.set_content(content);
        cx.notify();
    }

    /// Seed and read the table from a test outside `main_area`, which the
    /// field's visibility otherwise forbids. `install_content` remains the
    /// only production writer.
    #[cfg(test)]
    pub(in crate::workspace) fn images_for_test(&mut self) -> &mut MdImages {
        &mut self.images
    }

    /// Switch view mode, releasing the images when the switch discards the
    /// content they belong to. `None` when the mode is unchanged; `Some(true)`
    /// when the caller must reload. A markdown Preview↔Raw toggle keeps both
    /// content and table, so it releases nothing.
    pub(in crate::workspace) fn begin_mode_change(
        &mut self,
        mode: super::FileViewMode,
        window: Option<&mut Window>,
        cx: &mut Context<Workspace>,
    ) -> Option<bool> {
        let needs_reload = self.view.begin_mode_change(mode)?;
        if needs_reload {
            self.release_images(window, cx);
        }
        Some(needs_reload)
    }

    /// Drop this pane's GPU images and free the sprite-atlas textures behind
    /// them. Callers on a drop path must reach this before the pane goes:
    /// `FileContent` is a plain struct and `Drop` cannot reach `&mut App`.
    ///
    /// `window`: pass `Some` whenever a window update is in flight.
    /// `App::drop_image(_, None)` frees nothing for the window being updated —
    /// `update_window_id` takes it out of `App.windows` for the duration — so
    /// every teardown path here passes `Some`, being reached from an action
    /// handler or a `cx.listener`. `None` is correct for exactly one caller
    /// class: the async load landing that reaches `install_content` from
    /// `this.update` inside `cx.spawn`, with no window update in flight.
    pub(in crate::workspace) fn release_images(
        &mut self,
        mut window: Option<&mut Window>,
        cx: &mut Context<Workspace>,
    ) {
        let released = std::mem::take(&mut self.images);
        if released.0.is_empty() {
            return;
        }
        for image in released.0.into_iter().flatten() {
            cx.drop_image(image.render_image(), window.as_deref_mut());
        }
        // A clean `.cached()` view replays last frame's sprites, which still
        // point at the atlas tile just freed — the Metal draw path unwraps that
        // slot and panics. Repainting is what keeps the release safe.
        cx.notify();
    }
}

/// Release the GPU images of every file pane in `panes` whose id is in `ids`.
/// The pane drop paths (closing a pane or a tab, emptying a lane, removing a
/// lane, closing a project) call this before the panes are discarded, and all
/// hold a window — see [`FileContent::release_images`] for why that matters.
pub(in crate::workspace) fn release_pane_images(
    panes: &mut [Pane],
    ids: &[PaneId],
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    for pane in panes.iter_mut().filter(|p| ids.contains(&p.id)) {
        if let Some(fc) = pane.file_content_mut() {
            fc.release_images(Some(&mut *window), cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `width * height * 4` bytes, so the conversion succeeds.
    fn decodable() -> RasterImage {
        RasterImage {
            width: 1,
            height: 1,
            bgra: vec![0, 0, 0, 255],
            scale: 1.0,
        }
    }

    /// A short buffer for its dimensions — `from_raster` rejects it.
    fn undecodable() -> RasterImage {
        RasterImage {
            width: 4,
            height: 4,
            bgra: vec![0, 0, 0, 255],
            scale: 1.0,
        }
    }

    #[test]
    fn slot_order_survives_the_conversion() {
        let images = MdImages::from_rasters(vec![
            Some(decodable()),
            None,
            Some(undecodable()),
            Some(decodable()),
        ]);
        assert!(images.get(0).is_some());
        // A raster that never loaded and one that failed to convert are the
        // same thing to the renderer: fall back to the alt text.
        assert!(images.get(1).is_none());
        assert!(images.get(2).is_none());
        assert!(images.get(3).is_some());
    }

    /// An emptied table is the post-release state, not a numbering bug: it
    /// must answer `None` for any slot without tripping the assertion below.
    #[test]
    fn a_released_table_has_no_image_for_any_slot() {
        let released = MdImages::default();
        assert!(released.get(0).is_none());
        assert!(released.get(7).is_none());
    }

    /// A slot past the end means the resolve pass and the table came from
    /// different loads. Debug builds catch the caller; the assertion is all
    /// there is to check, since it fires before the `None` release returns.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "markdown image slot 1 is outside a table of 1")]
    fn an_out_of_range_slot_is_a_bug() {
        let _ = MdImages::from_rasters(vec![Some(decodable())]).get(1);
    }
}
