//! Workspace operation behind the right dock's collapsible sections.

use gpui::Context;

use super::section::SectionKey;
use crate::workspace::Workspace;

impl Workspace {
    pub(in crate::workspace) fn toggle_right_dock_section(
        &mut self,
        section: impl Into<SectionKey>,
        cx: &mut Context<Self>,
    ) {
        self.right_views.sections.toggle(section);
        cx.notify();
    }

    /// Drop fold state for Tools sources the current library no longer lists.
    pub(in crate::workspace) fn prune_tools_folds(&mut self) {
        use super::section::DockSection;
        let groups = &self.right_views.library.view.inventory.groups;
        let live = groups
            .iter()
            .map(|group| super::tools::render::fold_key(groups, &group.target))
            .collect();
        self.right_views.sections.retain_sources(
            &[
                DockSection::ToolsProject,
                DockSection::ToolsLocal,
                DockSection::ToolsUser,
            ],
            &live,
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::workspace::right_dock::section::DockSection;
    use crate::workspace::right_dock::section::DockSections;

    #[test]
    fn toggling_twice_is_a_no_op() {
        let mut sections = DockSections::default();
        sections.toggle(DockSection::UsageTokens);
        sections.toggle(DockSection::UsageTokens);
        assert!(sections.is_open(DockSection::UsageTokens));
    }
}
