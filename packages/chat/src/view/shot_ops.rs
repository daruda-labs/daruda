//! What a capture scenario may ask of a seeded pane: which row of the
//! projection that just ran it should open, and which popover to pin.
//! The answers come from the seed, never from constants in the host.

use gpui::Context;

use super::super::rows::RowKind;
use super::{ActivityOptionsTab, AgentChatView, OverageNotice};

impl AgentChatView {
    /// The first conclusion's item index.
    pub fn first_conclusion_for_shot(&self) -> Option<usize> {
        self.rows.iter().find_map(|r| match r.kind {
            RowKind::ConclusionItem(ix) => Some(ix),
            _ => None,
        })
    }

    /// The run start the step window's boundary row is keyed by.
    pub fn tail_boundary_for_shot(&self) -> Option<usize> {
        self.rows.iter().find_map(|r| match r.kind {
            RowKind::TailMore { run_start, .. } => Some(run_start),
            _ => None,
        })
    }

    /// The last tool group with a call behind its own window's boundary.
    pub fn last_windowed_group_for_shot(&self) -> Option<String> {
        self.rows.iter().rev().find_map(|r| match &r.kind {
            RowKind::ToolGroupTailMore {
                gid, hidden_calls, ..
            } if *hidden_calls > 0 => Some(gid.clone()),
            _ => None,
        })
    }

    /// Pin the compact bar's combined options popover open on `tab`.
    pub fn pin_options_for_shot(&mut self, tab: ActivityOptionsTab, cx: &mut Context<Self>) {
        self.set_activity_options_tab(tab, cx);
        self.screenshot_options_open = true;
    }

    pub fn pin_fold_editor_for_shot(&mut self, cx: &mut Context<Self>) {
        self.set_activity_options_tab(ActivityOptionsTab::Fold, cx);
        self.screenshot_fold_open = true;
    }

    /// Raise the banner an overage stop leaves, as if the 5-hour window had
    /// just run out, and relabel the seeded stop marker to match. The reset
    /// moment is fixed so captures stay stable.
    pub fn seed_overage_stop_for_shot(&mut self, cx: &mut Context<Self>) {
        const SHOT_RESETS_AT: i64 = 1_791_340_200;
        self.overage_notice = Some(OverageNotice {
            window: Some(daruda_acp::RateLimitWindow::FiveHour),
            resets_at: Some(SHOT_RESETS_AT),
        });
        if let Some(ix) = self
            .items
            .iter()
            .rposition(|item| matches!(item, daruda_acp::ChatItem::Interrupted))
        {
            self.overage_stop_markers.insert(ix);
        }
        cx.notify();
    }

    /// Leave the filter on its tab but its popover shut, so the chip on
    /// the response bar is what shows.
    pub fn select_filter_tab_for_shot(&mut self, cx: &mut Context<Self>) {
        self.set_activity_options_tab(ActivityOptionsTab::Filter, cx);
        self.screenshot_filter_open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::make_test_view;

    #[gpui::test]
    fn an_empty_projection_offers_nothing_to_open(cx: &mut gpui::TestAppContext) {
        let view = make_test_view(cx);
        view.update(cx, |v, _, cx| {
            assert_eq!(v.first_conclusion_for_shot(), None);
            assert_eq!(v.tail_boundary_for_shot(), None);
            assert_eq!(v.last_windowed_group_for_shot(), None);
            v.pin_fold_editor_for_shot(cx);
            assert!(v.screenshot_fold_open);
        })
        .unwrap();
    }
}
