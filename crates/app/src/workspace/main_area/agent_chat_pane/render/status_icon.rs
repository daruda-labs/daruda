//! The one status vocabulary the agent chat draws.
//!
//! A run's rollup and a plan step answer the same question — how is this
//! going, and how did it end — so they share one table rather than each
//! picking its own mark. They used to disagree: a running run was a text `●`
//! while a running plan step was its own glyph, so the plan header and the
//! plan row under it described one step two ways.
//!
//! [`StatusIcon`] is that shared vocabulary, and the two domain enums convert
//! into it. Neither can pick a mark the other cannot; adding a state means
//! adding it here, once.

use daruda_acp::PlanStatus;
use gpui::{AnyElement, App, Hsla, IntoElement, div, prelude::*};

use super::pulse_opacity;
use crate::ui::theme;
use crate::ui::{Icon, Sizable as _};
use crate::workspace::main_area::agent_chat_pane::agent_chat_helpers::Rollup;

// Material Symbols, daruda's own icon set (see `assets.rs`). One family for the
// whole vocabulary, and the shape carries the state so colour is reinforcement
// rather than the only channel (`DESIGN.md`) — which is also why a stopped step
// (`cancel`, a ✕) and a failed one (`error`, a `!`) are different marks and not
// one mark in two colours.
const ICON_RUNNING: &str = "icons/ui/radio-button-checked.svg";
const ICON_OK: &str = "icons/ui/check-circle.svg";
const ICON_PENDING: &str = "icons/ui/radio-button-unchecked.svg";
const ICON_CANCELLED: &str = "icons/ui/cancel.svg";
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
    let icon = icon.into();
    div()
        .flex_none()
        .when(icon.is_live(), |el| el.opacity(pulse_opacity(cx)))
        .child(
            Icon::empty()
                .path(icon.asset())
                .xsmall()
                .text_color(icon.color(t, dim, cx)),
        )
        .into_any_element()
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
