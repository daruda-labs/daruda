//! The report owns both its contents and the request that may replace them.

use gpui::SharedString;

#[derive(Default)]
pub(in crate::workspace) enum RunReport {
    #[default]
    Loading,
    Loaded(SharedString),
    /// Not there, or not readable.
    Missing,
}

#[derive(Default)]
pub(super) struct ReportState {
    content: RunReport,
    epoch: u64,
}

impl ReportState {
    pub fn content(&self) -> &RunReport {
        &self.content
    }
    pub fn begin_read(&mut self) -> u64 {
        self.epoch += 1;
        self.content = RunReport::Loading;
        self.epoch
    }
    pub fn accept(&mut self, epoch: u64, text: Option<String>) -> bool {
        if epoch != self.epoch {
            return false;
        }
        self.content = text
            .map(|text| RunReport::Loaded(text.into()))
            .unwrap_or(RunReport::Missing);
        true
    }
    #[cfg(test)]
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_latest_read_can_replace_the_report() {
        let mut report = ReportState::default();
        let old = report.begin_read();
        let current = report.begin_read();
        assert!(!report.accept(old, None));
        assert!(report.accept(current, Some("latest".into())));
        assert!(!report.accept(old, Some("old".into())));
        assert!(matches!(report.content(), RunReport::Loaded(text) if text.as_ref() == "latest"));
    }
}
