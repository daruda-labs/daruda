//! Visible row primitives for the lanes view:
//! `section_header`, `worktree_row`, the small `LaneId → GitBadge`
//! derivation, and the non-git placeholder hint.

use crate::ui::theme;
use gpui::{IntoElement, SharedString, div, prelude::*, px};

use daruda_store::project::{LaneId, ProjectId};

use crate::lane::availability::LaneAvailability;
use crate::surface::strings as surface_strings;
use crate::ui::{Icon, IconName, Sizable as _};
use crate::workspace::layout::LeftDockSnapshot;

// Sibling modules the child row files reach as `super::<module>`.
use super::{agent_badges, context_menu, create_modal, drag, group_menu, project_menu, tree};

mod headers;
mod worktree;
pub(super) use headers::{ProjectHeaderArgs, group_header_row, project_header_row, section_header};
pub(super) use worktree::worktree_row;

/// Compact summary of a lane's git status for the row badge — rolled
/// up into a single change count plus ahead/behind divergence so the
/// dock row can render a GitHub-Desktop-style chip
/// (`↑N ↓N [total]`). `None` means "show nothing" (clean tree + no
/// divergence, or no status cached yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GitBadgeData {
    /// Modified + staged + untracked, rolled up into a single count so the
    /// GH-Desktop pill stays narrow.
    pub total: u32,
    /// Commits ahead of the configured upstream (zero when no upstream
    /// is set or HEAD is detached).
    pub ahead: u32,
    /// Commits behind the configured upstream.
    pub behind: u32,
}

/// Build the GH-Desktop-style change/divergence summary for lane
/// `id` from the cached git status. Returns `None` when no status is
/// cached yet or the lane is fully clean (no changes + no
/// divergence) so the row stays uncluttered.
pub(super) fn git_badge_for(
    snap: &LeftDockSnapshot,
    project_id: ProjectId,
    id: LaneId,
) -> Option<GitBadgeData> {
    let target = daruda_store::project::LaneRef {
        project: project_id,
        lane: id,
    };
    // The badge spans both axes: the counts come from the working tree,
    // the arrows from the refs. Either side alone can make a badge.
    let status = snap.git_worktree_cache.get(&target);
    let tracking = snap.git_tracking_cache.get(&target);
    // Unstaged entries are identified by the y column, not x.
    let modified = status.map_or(0, |s| {
        s.staged.len()
            + s.unstaged
                .iter()
                .filter(|e| e.y != ' ' && e.y != '?')
                .count()
    });
    let untracked = status.map_or(0, |s| s.unstaged.iter().filter(|e| e.x == '?').count());
    let total = (modified + untracked) as u32;
    let (ahead, behind) = tracking.map_or((0, 0), |t| (t.ahead, t.behind));
    if total == 0 && ahead == 0 && behind == 0 {
        return None;
    }
    Some(GitBadgeData {
        total,
        ahead,
        behind,
    })
}

/// Icon + short label for an inaccessible lane / project state. Only
/// the non-`Present` variants are meaningful here; `Present` returns
/// `None` so callers render the normal row. The icon doubles as the
/// color cue — both states paint it in `theme::WARNING`.
fn availability_badge(availability: LaneAvailability) -> Option<(IconName, String)> {
    match availability {
        LaneAvailability::Present => None,
        // No `Lock` glyph in the icon set — `TriangleAlert` reads as
        // "something is wrong with this directory" for the missing
        // case; `EyeOff` reads as "present but unreadable" for the
        // permission-denied case.
        LaneAvailability::Missing => Some((
            IconName::TriangleAlert,
            surface_strings::projects::directory_missing(),
        )),
        LaneAvailability::AccessDenied => Some((
            IconName::EyeOff,
            surface_strings::projects::permission_denied(),
        )),
    }
}

/// Inline `⚠ <label>` chip rendered in place of the normal sublabel /
/// git badge for an inaccessible lane or project. Icon carries the
/// warning color; the label is muted.
fn availability_chip(icon: IconName, label: String, label_color: gpui::Hsla) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::LANE_LABEL_GAP))
        .text_size(px(theme::LANE_SUB_FONT_SIZE))
        .text_color(label_color)
        .child(
            Icon::new(icon)
                .with_size(px(theme::LANE_SUB_FONT_SIZE))
                .text_color(theme::WARNING),
        )
        .child(SharedString::from(label))
}

/// Right-aligned chip on the project header showing the default branch name
/// (e.g. `⎇ main`). Uses `GalleryVerticalEnd` as the branch glyph — the
/// closest available icon to the standard branch symbol; no dedicated
/// `GitBranch` variant exists in the current icon set.
fn branch_chip(branch: SharedString, label_color: gpui::Hsla) -> impl IntoElement {
    div()
        .flex()
        .flex_none() // must not stretch: sits right-aligned after the flex_1 name and keeps its intrinsic width
        .flex_row()
        .items_center()
        .gap(px(theme::LANE_LABEL_GAP))
        .px(px(theme::LANE_BRANCH_CHIP_PAD_X))
        .text_size(px(theme::LANE_SUB_FONT_SIZE))
        .text_color(label_color)
        .child(
            Icon::new(IconName::GalleryVerticalEnd)
                .with_size(px(theme::LANE_SUB_FONT_SIZE))
                .text_color(label_color),
        )
        .child(branch)
}

pub(super) fn non_git_placeholder(cx: &gpui::App) -> impl IntoElement {
    let t = theme::current(cx);
    let hint_color = t.text_subtle;
    let init_color = t.text_muted;
    div()
        .flex()
        .flex_col()
        .gap(px(theme::LANE_PLACEHOLDER_LINE_GAP))
        .p(px(theme::LANE_PLACEHOLDER_PAD))
        .text_size(px(theme::LANE_SUB_FONT_SIZE))
        .text_color(hint_color)
        .child(surface_strings::terminal::lane_non_git_hint())
        .child(
            div()
                .mt(px(theme::LANE_PLACEHOLDER_GIT_INIT_MT))
                .text_color(init_color)
                .child(surface_strings::terminal::lane_git_init_label()),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_badge_present_is_none() {
        assert!(availability_badge(LaneAvailability::Present).is_none());
    }

    #[test]
    fn availability_badge_missing_uses_triangle_alert() {
        let (icon, label) =
            availability_badge(LaneAvailability::Missing).expect("missing yields a badge");
        assert!(matches!(icon, IconName::TriangleAlert));
        assert_eq!(label, surface_strings::projects::directory_missing());
    }

    #[test]
    fn availability_badge_denied_uses_eye_off() {
        let (icon, label) =
            availability_badge(LaneAvailability::AccessDenied).expect("denied yields a badge");
        assert!(matches!(icon, IconName::EyeOff));
        assert_eq!(label, surface_strings::projects::permission_denied());
    }
}
