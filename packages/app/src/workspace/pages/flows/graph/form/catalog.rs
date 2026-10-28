//! Catalog-backed choices; cached advertisements are hints, not validation.

use crate::surface::strings as s;
use crate::ui::select::SelectOption;
use daruda_store::agent_vocabulary::{AgentVocabularyCache, VocabEntry};

#[derive(Clone, Default)]
pub(in crate::workspace) struct Catalog {
    pub agents: std::sync::Arc<[daruda_config::AgentDefinition]>,
    pub vocabulary: AgentVocabularyCache,
}

impl Catalog {
    pub(super) fn agents(&self, inherited: Option<&str>) -> Vec<SelectOption> {
        let mut items = vec![inherited_option(inherited)];
        items.extend(self.agents.iter().map(|agent| {
            SelectOption::new(
                agent.id.clone(),
                s::flow::form_agent_choice(&agent.name, &agent.id),
            )
        }));
        items
    }

    pub(super) fn axis(&self, id: &str, model: bool, inherited: Option<&str>) -> Vec<SelectOption> {
        let mut items = vec![inherited_option(inherited)];
        let Some(command) = self
            .agents
            .iter()
            .find(|agent| agent.id == id)
            .and_then(|agent| agent.launch.local_command())
        else {
            return items;
        };
        let seed = daruda_config::agent_vocabulary_seed(command);
        let cached = if model {
            self.vocabulary.known_models_for(id, command)
        } else {
            self.vocabulary.known_modes_for(id, command)
        };
        let seeded = seed
            .as_ref()
            .map(|seed| {
                if model {
                    seed.models.as_slice()
                } else {
                    seed.modes.as_slice()
                }
            })
            .unwrap_or_default();
        items.extend(
            known_axis(cached, seeded)
                .iter()
                .map(|entry| SelectOption::new(entry.id.clone(), entry.name.clone())),
        );
        items
    }
}

fn known_axis<'a>(cached: Option<&'a [VocabEntry]>, seeded: &'a [VocabEntry]) -> &'a [VocabEntry] {
    cached.unwrap_or(seeded)
}

pub(super) fn inherited_option(value: Option<&str>) -> SelectOption {
    SelectOption::new(
        "",
        value
            .map(s::flow::form_inherited)
            .unwrap_or_else(s::flow::form_follow_defaults),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_empty_does_not_resurrect_seeded_choices() {
        let seeded = [VocabEntry::new("model", "Model")];
        assert_eq!(known_axis(None, &seeded), seeded);
        assert!(known_axis(Some(&[]), &seeded).is_empty());
    }
}
