//! Workspace operation behind the right dock's collapsible sections.

use gpui::Context;

use super::section::DockSection;
use crate::workspace::Workspace;

impl Workspace {
    pub(in crate::workspace) fn toggle_right_dock_section(
        &mut self,
        section: DockSection,
        cx: &mut Context<Self>,
    ) {
        self.right_dock_sections.toggle(section);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::DockSection;
    use crate::workspace::right_dock::section::DockSections;

    #[test]
    fn toggling_twice_is_a_no_op() {
        let mut sections = DockSections::default();
        sections.toggle(DockSection::UsageTokens);
        sections.toggle(DockSection::UsageTokens);
        assert!(sections.is_open(DockSection::UsageTokens));
    }
}
