//! Display glyphs that read the same in every locale, so they bypass i18n.

/// Agent integration banner's leading icon.
pub const AGENT_BANNER_ICON: &str = "ⓘ";
/// Per-badge tooltip — appended after the session_id prefix to mark
/// the truncation.
pub const AGENT_BADGE_TOOLTIP_ELLIPSIS: &str = "…";
/// Files-view chevron while a directory's children are still loading.
pub const FILES_CHEVRON_PENDING: &str = "…";
/// Glyph appended to the status pill label as the dropdown chevron.
/// Leading space provides the visual gap between the label and the
/// triangle.
pub const TASK_PILL_CHEVRON: &str = " ▾";
/// Tab-title dirty indicator. Painted before the title with a
/// trailing space so titles align across dirty / clean tabs.
pub const TAB_TITLE_DIRTY_DOT: &str = "● ";
/// Severity glyphs in the toast leading icon slot.
pub const TOAST_ICON_INFO: &str = "ℹ";
pub const TOAST_ICON_WARNING: &str = "⚠";
pub const TOAST_ICON_ERROR: &str = "✕";
/// `×N` repeat counter prefix.
pub const TOAST_REPEAT_PREFIX: &str = "×";

#[cfg(test)]
mod tests {
    use super::*;

    /// A glyph is one visible mark; an empty one paints nothing in its slot.
    #[test]
    fn glyphs_are_non_empty() {
        for g in [
            AGENT_BANNER_ICON,
            AGENT_BADGE_TOOLTIP_ELLIPSIS,
            FILES_CHEVRON_PENDING,
            TASK_PILL_CHEVRON,
            TAB_TITLE_DIRTY_DOT,
            TOAST_ICON_INFO,
            TOAST_ICON_WARNING,
            TOAST_ICON_ERROR,
            TOAST_REPEAT_PREFIX,
        ] {
            assert!(!g.trim().is_empty());
        }
    }
}
