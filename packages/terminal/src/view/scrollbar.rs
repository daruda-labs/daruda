//! Shared thumb geometry for painting and pointer-drag inversion.

pub(super) struct ScrollbarMetrics {
    pub(super) thumb_height: f32,
    travel: f32,
    scrollable: u32,
}

impl ScrollbarMetrics {
    pub(super) fn new(track_height: f32, total_rows: u32, viewport_rows: u32) -> Self {
        let thumb_ratio = viewport_rows as f32 / total_rows as f32;
        let thumb_height =
            (track_height * thumb_ratio).max(crate::ux::theme::TERMINAL_SCROLLBAR_THUMB_MIN_H);
        Self {
            thumb_height,
            travel: track_height - thumb_height,
            scrollable: total_rows.saturating_sub(viewport_rows),
        }
    }

    pub(super) fn thumb_top(&self, viewport_offset: u32) -> f32 {
        if self.scrollable > 0 {
            let fraction = viewport_offset as f32 / self.scrollable as f32;
            fraction * self.travel
        } else {
            self.travel
        }
    }

    pub(super) fn offset_for_drag(&self, cursor_y: f32, click_offset: f32) -> i32 {
        let thumb_top = (cursor_y - click_offset).clamp(0.0, self.travel);
        let fraction = if self.travel > 0.0 {
            (thumb_top / self.travel).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (fraction * self.scrollable as f32).round() as i32
    }
}

#[cfg(test)]
mod tests;
