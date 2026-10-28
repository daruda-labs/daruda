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
    /// History starts folded; configured tools and skills are visible at rest.
    fn opens_by_default(self) -> bool {
        !matches!(self, Self::UsageRecentSessions)
    }
}

/// What a fold click toggles: a section, or one source block inside it when
/// a section renders a block per configuration file.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::workspace) struct SectionKey {
    pub section: DockSection,
    pub source: Option<gpui::SharedString>,
}

impl From<DockSection> for SectionKey {
    fn from(section: DockSection) -> Self {
        Self {
            section,
            source: None,
        }
    }
}

/// Sections whose open state differs from their default; empty = defaults.
#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::workspace) struct DockSections {
    toggled: HashSet<SectionKey>,
}

impl DockSections {
    pub fn is_open(&self, key: impl Into<SectionKey>) -> bool {
        let key = key.into();
        key.section.opens_by_default() != self.toggled.contains(&key)
    }

    pub fn toggle(&mut self, key: impl Into<SectionKey>) {
        let key = key.into();
        if !self.toggled.remove(&key) {
            self.toggled.insert(key);
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
        assert!(sections.is_open(DockSection::SkillsPlugins));
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

    #[test]
    fn sources_fold_independently() {
        let mut sections = DockSections::default();
        let key = |source: &str| SectionKey {
            section: DockSection::ToolsProject,
            source: Some(source.to_string().into()),
        };
        sections.toggle(key("a"));
        assert!(!sections.is_open(key("a")));
        assert!(sections.is_open(key("b")));
        assert!(sections.is_open(DockSection::ToolsProject));
    }
}
