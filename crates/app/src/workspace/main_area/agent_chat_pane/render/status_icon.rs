//! The one status vocabulary the agent chat draws.
//!
//! A run's rollup, a plan step and a tool call all answer the same question —
//! how is this going, and how did it end — so they share one table rather than
//! each picking its own mark. They used to disagree: a running run was a text
//! `●`, a running plan step its own glyph, a finished tool call the word
//! "Done", so one turn described itself three ways.
//!
//! [`StatusIcon`] is that shared vocabulary, and the three domain enums convert
//! into it. None can pick a mark the others cannot; adding a state means adding
//! it here, once.

use daruda_acp::{PlanStatus, ToolStatusView};
use gpui::{AnyElement, App, Hsla, IntoElement, SharedString, div, prelude::*, px};

use super::pulse_opacity;
use crate::ui::theme;
use crate::ui::{Icon, Sizable as _};
use crate::workspace::main_area::agent_chat_pane::agent_chat_helpers::Rollup;

// Material Symbols, daruda's own icon set (see `assets.rs`). One family for the
// whole vocabulary, and the shape carries the state so colour is reinforcement
// rather than the only channel (`DESIGN.md`) — which is also why a stopped step
// (`block`, a ⊘) and a failed one (`error`, a `!`) are different marks and not
// one mark in two colours. The outcome marks take the family's filled cut so
// they hold their shape at `xsmall`; `radio-button-*` has no filled variant and
// needs none — an unreached step *should* read as an empty ring.
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

/// [`status_icon`] with the live state's age beside it. The number is the
/// liveness signal, so the mark stays solid: a counter says *how long* as well
/// as *still going*, which a blink cannot, and two moving signals on one row
/// read as noise. `age` is ignored unless the state is live — a settled mark
/// has nothing left to count.
pub(super) fn status_icon_with_age(
    icon: impl Into<StatusIcon>,
    age: Option<std::time::Duration>,
    t: &theme::DarudaTheme,
    dim: f32,
    cx: &App,
) -> AnyElement {
    let icon = icon.into();
    mark(icon, icon.is_live().then_some(age).flatten(), t, dim, cx)
}

fn mark(
    icon: StatusIcon,
    age: Option<std::time::Duration>,
    t: &theme::DarudaTheme,
    dim: f32,
    cx: &App,
) -> AnyElement {
    let color = icon.color(t, dim, cx);
    // A counter already carries the motion, so the mark beside it stays solid.
    let blink = icon.is_live() && age.is_none();
    div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GAP_SM))
        .when(blink, |el| el.opacity(pulse_opacity(cx)))
        .child(Icon::empty().path(icon.asset()).xsmall().text_color(color))
        .children(age.map(|age| {
            div()
                .flex_none()
                .text_color(color)
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(SharedString::from(format_age(age)))
        }))
        .into_any_element()
}

/// `"5s"` under a minute, `"1m05s"` at or over — the same shape the working
/// indicator's run timer uses, so the two read as one unit of measure.
fn format_age(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{:02}s", secs / 60, secs % 60)
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
