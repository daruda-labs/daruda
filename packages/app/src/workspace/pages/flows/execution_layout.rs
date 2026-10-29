//! Execution columns are budgeted from the space remaining between the docks.

use crate::{ui::theme::palette, workspace::Workspace};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum ExecutionLayout {
    Overview,
    Inspector,
    Focused,
}

impl ExecutionLayout {
    pub fn for_width(available: f32) -> Self {
        let pair = palette::FLOW_RUN_HISTORY_W + palette::FLOW_RUN_INSPECTOR_W;
        if available >= pair + palette::FLOW_RUN_OVERVIEW_MIN_W {
            Self::Overview
        } else if available >= pair {
            Self::Inspector
        } else {
            Self::Focused
        }
    }

    pub fn overview(self) -> bool {
        self == Self::Overview
    }
    pub fn history(self) -> bool {
        self != Self::Focused
    }
}

impl Workspace {
    pub(in crate::workspace) fn execution_layout(
        &self,
        window: &gpui::Window,
        cx: &gpui::App,
    ) -> ExecutionLayout {
        let width = f32::from(window.viewport_size().width);
        let left = self.docks.left.read(cx).display_size();
        let right = self.docks.right.read(cx);
        let right = if right.is_open { right.size } else { 0.0 };
        ExecutionLayout::for_width((width - left - right).max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_space_determines_the_column_layout() {
        assert_eq!(
            ExecutionLayout::for_width(1440.0 - 220.0),
            ExecutionLayout::Overview
        );
        assert_eq!(
            ExecutionLayout::for_width(1100.0 - 300.0 - 300.0),
            ExecutionLayout::Focused
        );
        assert_eq!(
            ExecutionLayout::for_width(1000.0 - 220.0),
            ExecutionLayout::Inspector
        );
    }
}
