//! The one status vocabulary the agent chat draws.
//!
//! A run's rollup, a plan step and a tool call all answer the same question —
//! how is this going, how did it end — so [`StatusIcon`] is the single table
//! all three convert into. None can pick a mark the others cannot, and adding
//! a state means adding it here, once.

use daruda_acp::{PlanStatus, ToolStatusView};
use gpui::{AnyElement, App, Hsla, IntoElement, SharedString, div, prelude::*, px};

use super::{format_elapsed, pulse_opacity};
use crate::ui::theme;
use crate::ui::{Icon, Sizable as _};
use crate::workspace::main_area::agent_chat_pane::agent_chat_helpers::Rollup;

// Material Symbols, daruda's own set (see `assets.rs`). Shape carries the state
// so colour only reinforces it (`DESIGN.md`): a stopped step (⊘) and a failed
// one (!) are different marks, not one mark in two colours.
// The outcome marks take the filled cut, which is what holds their shape at
// `small`; `radio-button-*` has none and needs none, since an unreached step
// should read as an empty ring.
const ICON_RUNNING: &str = "icons/ui/radio-button-checked.svg";
const ICON_OK: &str = "icons/ui/check-circle.svg";
const ICON_PENDING: &str = "icons/ui/radio-button-unchecked.svg";
const ICON_CANCELLED: &str = "icons/ui/block.svg";
const ICON_PARTIAL: &str = "icons/ui/warning.svg";
const ICON_FAILED: &str = "icons/ui/error.svg";

/// How a run or one of its steps stands. The union of what the two domains
/// distinguish: a rollup never reports `Pending` / `Cancelled` (a run either
/// started or does not exist), and a plan step never reports `Partial` (it is
/// one step, so there is nothing to be partial about).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum StatusIcon {
    Running,
    Ok,
    Partial,
    Failed,
    Pending,
    Cancelled,
}

impl From<Rollup> for StatusIcon {
    fn from(rollup: Rollup) -> Self {
        match rollup {
            Rollup::Running => Self::Running,
            Rollup::Ok => Self::Ok,
            Rollup::Partial => Self::Partial,
            Rollup::Failed => Self::Failed,
        }
    }
}

impl From<PlanStatus> for StatusIcon {
    fn from(status: PlanStatus) -> Self {
        match status {
            PlanStatus::InProgress => Self::Running,
            PlanStatus::Completed => Self::Ok,
            PlanStatus::Pending => Self::Pending,
            PlanStatus::Cancelled => Self::Cancelled,
        }
    }
}

impl From<ToolStatusView> for StatusIcon {
    fn from(status: ToolStatusView) -> Self {
        match status {
            // `Pending` reads as running for the same reason the badge did: the
            // adapter marks every call `Pending` until a progress ping many
            // tools never get (see `ToolStatusView::is_live`).
            ToolStatusView::Pending | ToolStatusView::InProgress => Self::Running,
            ToolStatusView::Completed => Self::Ok,
            ToolStatusView::Failed => Self::Failed,
            ToolStatusView::Cancelled => Self::Cancelled,
        }
    }
}

impl StatusIcon {
    fn asset(self) -> &'static str {
        match self {
            Self::Running => ICON_RUNNING,
            Self::Ok => ICON_OK,
            Self::Partial => ICON_PARTIAL,
            Self::Failed => ICON_FAILED,
            Self::Pending => ICON_PENDING,
            Self::Cancelled => ICON_CANCELLED,
        }
    }

    /// `t` is already dimmed for an inactive pane (`theme::current(cx).dimmed`),
    /// so only the muted tier — which reads a live global rather than `t` — has
    /// to dim itself.
    fn color(self, t: &theme::DarudaTheme, dim: f32, cx: &App) -> Hsla {
        match self {
            // Amber "executing tool" accent so live work reads stronger than a
            // settled mark.
            Self::Running => t.status_executing_tool_dark,
            // file_diff_stat_add == SUCCESS (green); no dedicated done token.
            Self::Ok => t.file_diff_stat_add,
            // Partial = some failed, some succeeded → warning, not a hard failure.
            Self::Partial => t.banner_warning_text,
            Self::Failed => t.banner_error_text,
            // Neither reached nor resolved: muted, so only outcomes take colour.
            Self::Pending | Self::Cancelled => {
                theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim)
            }
        }
    }

    /// Whether this state is still happening — the single condition the blink
    /// keys off, so the rollup and the plan cannot pulse on different rules.
    pub(super) fn is_live(self) -> bool {
        matches!(self, Self::Running)
    }
}

/// Render one status mark at text size. Live states blink on the shared
/// 2-tick pulse; settled ones stay solid.
pub(super) fn status_icon(
    icon: impl Into<StatusIcon>,
    t: &theme::DarudaTheme,
    dim: f32,
    cx: &App,
) -> AnyElement {
    mark(icon.into(), None, t, dim, cx)
}

/// [`status_icon`] for a surface that knows how long a live state has been
/// going. `age` is ignored unless the state is live — a settled mark has
/// nothing left to count.
pub(super) fn status_icon_with_age(
    icon: impl Into<StatusIcon>,
    age: Option<std::time::Duration>,
    t: &theme::DarudaTheme,
    dim: f32,
    cx: &App,
) -> AnyElement {
    mark(icon.into(), age, t, dim, cx)
}

/// What the status slot draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    /// The mark. `blink` while the state is still happening and nothing else
    /// on the row is carrying that.
    Mark { blink: bool },
    /// The age alone, for a live state whose start is known. It *replaces* the
    /// mark rather than joining it: a ticking number already says "running",
    /// and says how long as well, so drawing both is one signal too many.
    Age(std::time::Duration),
}

fn slot(icon: StatusIcon, age: Option<std::time::Duration>) -> Slot {
    match age.filter(|_| icon.is_live()) {
        Some(age) => Slot::Age(age),
        None => Slot::Mark {
            blink: icon.is_live(),
        },
    }
}

fn mark(
    icon: StatusIcon,
    age: Option<std::time::Duration>,
    t: &theme::DarudaTheme,
    dim: f32,
    cx: &App,
) -> AnyElement {
    let color = icon.color(t, dim, cx);
    let base = div().flex_none().text_color(color);
    match slot(icon, age) {
        Slot::Age(age) => base
            .text_size(px(theme::agent_chat_font_size(cx)))
            .child(SharedString::from(format_elapsed(age)))
            .into_any_element(),
        Slot::Mark { blink } => base
            .when(blink, |el| el.opacity(pulse_opacity(cx)))
            .child(Icon::empty().path(icon.asset()).small().text_color(color))
            .into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two domains meet on the states they share: a running run and a
    /// running step are the same mark, and so are a finished one of each.
    #[test]
    fn the_shared_states_map_to_one_mark() {
        assert_eq!(
            StatusIcon::from(Rollup::Running),
            StatusIcon::from(PlanStatus::InProgress)
        );
        assert_eq!(
            StatusIcon::from(Rollup::Ok),
            StatusIcon::from(PlanStatus::Completed)
        );
    }

    /// Every state owns a distinct asset — a mark that is only a colour apart
    /// from another is the failure this table exists to prevent.
    #[test]
    fn every_state_has_its_own_asset() {
        let all = [
            StatusIcon::Running,
            StatusIcon::Ok,
            StatusIcon::Partial,
            StatusIcon::Failed,
            StatusIcon::Pending,
            StatusIcon::Cancelled,
        ];
        let assets: std::collections::HashSet<_> = all.iter().map(|s| s.asset()).collect();
        assert_eq!(assets.len(), all.len(), "two states share one asset");
    }

    /// A known age replaces the mark; without one the mark stands, blinking
    /// only while the state is still happening.
    #[test]
    fn an_age_replaces_the_mark_it_would_otherwise_blink() {
        let age = std::time::Duration::from_secs(12);
        assert_eq!(slot(StatusIcon::Running, Some(age)), Slot::Age(age));
        assert_eq!(
            slot(StatusIcon::Running, None),
            Slot::Mark { blink: true },
            "live with no age: the mark carries the motion"
        );
        assert_eq!(
            slot(StatusIcon::Ok, Some(age)),
            Slot::Mark { blink: false },
            "a settled state has nothing to count"
        );
        assert_eq!(slot(StatusIcon::Ok, None), Slot::Mark { blink: false });
    }

    #[test]
    fn only_a_running_state_blinks() {
        assert!(StatusIcon::Running.is_live());
        for settled in [
            StatusIcon::Ok,
            StatusIcon::Partial,
            StatusIcon::Failed,
            StatusIcon::Pending,
            StatusIcon::Cancelled,
        ] {
            assert!(!settled.is_live(), "{settled:?} must not blink");
        }
    }
}
