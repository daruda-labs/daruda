//! Fold state for the right dock's collapsible sections.

use std::collections::HashSet;

/// Every collapsible section the right dock renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::workspace) enum DockSection {
    UsageTurns,
    UsageTokens,
    UsageRecentSessions,
    SkillsProject,
    SkillsPersonal,
    SkillsPlugins,
    ToolsProject,
    ToolsLocal,
    ToolsUser,
}

impl DockSection {
    /// History and the plugin catalogue are secondary, so they start folded.
    fn opens_by_default(self) -> bool {
        !matches!(self, Self::UsageRecentSessions | Self::SkillsPlugins)
    }
}

/// Sections whose open state differs from their default; empty = defaults.
#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::workspace) struct DockSections {
    toggled: HashSet<DockSection>,
}

impl DockSections {
    pub fn is_open(&self, section: DockSection) -> bool {
        section.opens_by_default() != self.toggled.contains(&section)
    }

    pub fn toggle(&mut self, section: DockSection) {
        if !self.toggled.remove(&section) {
            self.toggled.insert(section);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_section() {
        let sections = DockSections::default();
        assert!(sections.is_open(DockSection::UsageTurns));
        assert!(!sections.is_open(DockSection::UsageRecentSessions));
    }

    #[test]
    fn toggle_flips_and_restores() {
        let mut sections = DockSections::default();
        sections.toggle(DockSection::UsageRecentSessions);
        assert!(sections.is_open(DockSection::UsageRecentSessions));
        sections.toggle(DockSection::UsageRecentSessions);
        assert_eq!(sections, DockSections::default());
    }
}
