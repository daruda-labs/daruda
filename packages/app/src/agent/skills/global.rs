//! GPUI Global registration for [`SkillsState`]. The `impl Global` and
//! bootstrap helper live here so `agent/skills/mod.rs` stays GPUI-free
//! (CLAUDE.md G2 / G7).

use gpui::{App, BorrowAppContext, Global};

use super::SkillsState;

impl Global for SkillsState {}

/// Register state without filesystem work, including in workspace fixtures.
pub fn init(cx: &mut App) {
    if !cx.has_global::<SkillsState>() {
        cx.set_global(SkillsState::default());
    }
}

struct PluginCatalog {
    _task: gpui::Task<()>,
}
impl Global for PluginCatalog {}

/// The default Settings catalog is independent of pane-specific libraries.
/// Start once at app bootstrap; tests can register state without real watches.
pub(crate) fn start_catalog(cx: &mut App) {
    init(cx);
    if !cx.has_global::<PluginCatalog>() {
        let task = cx.spawn(async move |cx| {
            loop {
                let (watch, plugins) = cx
                    .background_executor()
                    .spawn(async {
                        let root = super::plugins::plugins_root();
                        let personal = super::scan::skills_personal_dir();
                        snapshot(&root, &personal)
                    })
                    .await;
                cx.update(|cx| {
                    cx.update_global::<SkillsState, _>(|state, _| {
                        state.plugin = plugins;
                        state.last_scanned = Some(std::time::SystemTime::now());
                    });
                });
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(200))
                        .await;
                    if watch.changed() {
                        break;
                    }
                }
            }
        });
        cx.set_global(PluginCatalog { _task: task });
    }
}

fn snapshot(
    root: &std::path::Path,
    personal: &std::path::Path,
) -> (crate::hooks::source_watch::SourceWatch, Vec<super::Skill>) {
    let mut paths = vec![root.to_path_buf(), personal.to_path_buf()];
    paths.extend(
        super::plugins::read_installed_plugins(&root.join("installed_plugins.json"))
            .into_iter()
            .map(|p| p.install_path),
    );
    paths.extend(
        super::plugins::read_known_marketplaces(&root.join("known_marketplaces.json"))
            .into_iter()
            .map(|p| p.install_location),
    );
    let watch = crate::hooks::source_watch::SourceWatch::new(paths);
    (watch, super::scan::scan_plugins_at(root, personal))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_catalog_is_loaded_and_watches_external_installations() {
        let _serial = crate::hooks::tests_common::fsevent_serial();
        let home = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let skill = external.path().join("skills/sample/SKILL.md");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::write(&skill, "---\nname: sample\n---\nOriginal").unwrap();
        let manifest = serde_json::json!({"version": 2, "plugins": {"example@market": [{"scope": "user", "installPath": external.path(), "version": "1"}]}});
        std::fs::write(
            home.path().join("installed_plugins.json"),
            manifest.to_string(),
        )
        .unwrap();
        let (watch, plugins) = snapshot(home.path(), &home.path().join("personal"));
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "example:sample");
        std::fs::write(&skill, "---\nname: updated\n---\nChanged").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !watch.changed() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let (_, plugins) = snapshot(home.path(), &home.path().join("personal"));
        assert_eq!(plugins[0].name, "example:updated");
    }
}
