//! Where a chat pane's "save as agent default" lands: the `[[agents]]` entry
//! in the settings store, whose observers reseed every pane following it.

use std::rc::Rc;

use daruda_config::SettingsPatch;
use gpui::BorrowAppContext as _;

use crate::settings_store::SettingsStore;
use daruda_chat::view::AgentDefaultWriter;

pub(in crate::workspace) fn agent_default_writer() -> AgentDefaultWriter {
    Rc::new(|agent_id, value, cx| {
        let agents = daruda_config::with_transcript_default(
            &SettingsStore::global(cx).user().agents,
            agent_id,
            value,
        )
        .ok_or_else(|| format!("no enabled agent entry has id `{agent_id}`"))?;
        cx.update_global::<SettingsStore, _>(|store, _| {
            store.apply_patch(SettingsPatch::AgentCatalog(agents))
        })
    })
}

#[cfg(test)]
mod tests {
    use daruda_config::TranscriptDefault;

    use super::*;

    fn claude(cx: &gpui::App) -> daruda_config::AgentDefinition {
        SettingsStore::global(cx)
            .user()
            .resolved_agents()
            .into_iter()
            .find(|agent| agent.id == "claude")
            .expect("the built-in catalog launches claude")
    }

    /// The value lands on the entry the agent launches from.
    #[gpui::test]
    fn a_default_lands_on_the_agents_entry(cx: &mut gpui::TestAppContext) {
        cx.update(SettingsStore::init);
        cx.update(|cx| {
            let tokens = daruda_transcript::fold_mode::FoldPreset::Expanded
                .mode()
                .tokens();
            let write = agent_default_writer();
            write(
                "claude",
                TranscriptDefault::FoldMode(Some(tokens.clone())),
                cx,
            )
            .expect("claude is in the catalog");
            assert_eq!(claude(cx).fold_mode, Some(tokens));
        });
    }

    /// An agent no entry launches has nowhere to keep a default.
    #[gpui::test]
    fn an_agent_without_an_entry_is_refused(cx: &mut gpui::TestAppContext) {
        cx.update(SettingsStore::init);
        cx.update(|cx| {
            let write = agent_default_writer();
            let result = write("gone", TranscriptDefault::FoldMode(None), cx);
            assert!(result.unwrap_err().contains("gone"));
        });
    }
}
