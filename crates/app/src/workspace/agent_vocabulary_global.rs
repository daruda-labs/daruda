//! App-wide owner of persisted ACP mode/model vocabularies.
//!
//! Every Workspace and the Settings window mirrors this Global. Keeping the
//! mutable cache here prevents one window from replacing another window's
//! newer `agent_vocabulary.json` snapshot.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use daruda_store::agent_vocabulary::{AgentVocabularyCache, VocabEntry};
use gpui::{App, BorrowAppContext as _, Global};

pub(crate) struct AgentVocabularyGlobal {
    caches: BTreeMap<PathBuf, AgentVocabularyCache>,
}

impl Global for AgentVocabularyGlobal {}

/// Install `data_dir`'s cache if this process has not seen that store yet.
/// Production has one profile directory; path-keying also keeps test stores
/// isolated when one app context constructs multiple workspaces.
pub(crate) fn install_path(cx: &mut App, data_dir: &Path) {
    let data_dir = data_dir.to_path_buf();
    if !cx.has_global::<AgentVocabularyGlobal>() {
        cx.set_global(AgentVocabularyGlobal {
            caches: BTreeMap::from([(data_dir.clone(), AgentVocabularyCache::load_in(&data_dir))]),
        });
        return;
    }
    if !cx
        .global::<AgentVocabularyGlobal>()
        .caches
        .contains_key(&data_dir)
    {
        let loaded = AgentVocabularyCache::load_in(&data_dir);
        cx.update_global::<AgentVocabularyGlobal, _>(|global, _| {
            global.caches.insert(data_dir, loaded);
        });
    }
}

pub(crate) fn snapshot(cx: &App, data_dir: &Path) -> AgentVocabularyCache {
    cx.global::<AgentVocabularyGlobal>()
        .caches
        .get(data_dir)
        .cloned()
        .unwrap_or_default()
}

/// Apply one agent advertisement to the shared cache and persist the exact
/// shared snapshot. `None` means the event carried no replacement for that
/// axis; `Some([])` is a known-empty advertisement and is recorded.
pub(crate) fn record(
    cx: &mut App,
    data_dir: &Path,
    agent_id: &str,
    source: &str,
    modes: Option<Vec<VocabEntry>>,
    models: Option<Vec<VocabEntry>>,
) -> std::io::Result<bool> {
    install_path(cx, data_dir);
    let data_dir = data_dir.to_path_buf();
    let mut next = snapshot(cx, &data_dir);
    let mut changed = false;
    if let Some(modes) = modes {
        changed |= next.record_modes(agent_id, source, modes);
    }
    if let Some(models) = models {
        changed |= next.record_models(agent_id, source, models);
    }
    if !changed {
        return Ok(false);
    }

    next.save_in(&data_dir)?;
    cx.update_global::<AgentVocabularyGlobal, _>(|global, _| {
        global.caches.insert(data_dir, next);
    });
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn updates_from_two_writers_share_one_persisted_snapshot(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            install_path(cx, dir.path());
            record(
                cx,
                dir.path(),
                "claude",
                "claude-acp",
                None,
                Some(vec![VocabEntry::new("opus", "Opus")]),
            )
            .unwrap();
            record(
                cx,
                dir.path(),
                "codex",
                "codex-acp",
                Some(vec![VocabEntry::new("agent", "Agent")]),
                None,
            )
            .unwrap();
        });

        let loaded = AgentVocabularyCache::load_in(dir.path());
        assert_eq!(
            loaded.known_models_for("claude", "claude-acp"),
            Some([VocabEntry::new("opus", "Opus")].as_slice())
        );
        assert_eq!(
            loaded.known_modes_for("codex", "codex-acp"),
            Some([VocabEntry::new("agent", "Agent")].as_slice())
        );
    }

    #[gpui::test]
    fn an_old_connection_cannot_replace_a_new_sources_vocabulary(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            record(
                cx,
                dir.path(),
                "assistant",
                "old-acp",
                Some(vec![VocabEntry::new("plan", "Plan")]),
                None,
            )
            .unwrap();
            record(
                cx,
                dir.path(),
                "assistant",
                "new-acp",
                None,
                Some(vec![VocabEntry::new("new-model", "New Model")]),
            )
            .unwrap();

            // A late event from the old connection used to reset the whole
            // agent entry back to old-acp and hide new-acp from Settings.
            record(
                cx,
                dir.path(),
                "assistant",
                "old-acp",
                Some(vec![VocabEntry::new("review", "Review")]),
                None,
            )
            .unwrap();
        });

        let loaded = AgentVocabularyCache::load_in(dir.path());
        assert_eq!(
            loaded.known_models_for("assistant", "new-acp"),
            Some([VocabEntry::new("new-model", "New Model")].as_slice())
        );
        assert_eq!(
            loaded.known_modes_for("assistant", "old-acp"),
            Some([VocabEntry::new("review", "Review")].as_slice())
        );
    }

    #[gpui::test]
    fn a_failed_save_does_not_commit_and_the_same_advertisement_retries(cx: &mut TestAppContext) {
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path().join("data");
        std::fs::write(&data_dir, "not a directory").unwrap();

        cx.update(|cx| {
            let result = record(
                cx,
                &data_dir,
                "claude",
                "claude-acp",
                None,
                Some(vec![VocabEntry::new("opus", "Opus")]),
            );
            assert!(result.is_err());
            assert_eq!(
                snapshot(cx, &data_dir).known_models_for("claude", "claude-acp"),
                None,
                "an unsaved vocabulary must not become the committed snapshot"
            );
        });

        std::fs::remove_file(&data_dir).unwrap();
        cx.update(|cx| {
            assert!(
                record(
                    cx,
                    &data_dir,
                    "claude",
                    "claude-acp",
                    None,
                    Some(vec![VocabEntry::new("opus", "Opus")]),
                )
                .unwrap(),
                "the identical advertisement must retry after the failed save"
            );
        });

        assert_eq!(
            AgentVocabularyCache::load_in(&data_dir).known_models_for("claude", "claude-acp"),
            Some([VocabEntry::new("opus", "Opus")].as_slice())
        );
    }
}
