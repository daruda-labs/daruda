//! GPUI Global wrapper for [`SkillsState`].
//!
//! `SkillsState` is GPUI-free, so the orphan rule blocks a bare
//! `impl Global` from here. The newtype carries the marker plus
//! `Deref`/`DerefMut` so callers use it without `.0` boilerplate.

use std::ops::{Deref, DerefMut};

use gpui::{App, BorrowAppContext, Global};

use super::SkillsState;

/// Newtype over `SkillsState` so the `impl Global` marker lives in the app
/// crate.
#[derive(Default)]
pub struct GlobalSkills(pub(crate) SkillsState);

impl Global for GlobalSkills {}

impl Deref for GlobalSkills {
    type Target = SkillsState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for GlobalSkills {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Register state without filesystem work, including in workspace fixtures.
pub fn init(cx: &mut App) {
    crate::status_icon_themes::init(cx);
    if !cx.has_global::<GlobalSkills>() {
        cx.set_global(GlobalSkills::default());
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
                let (watch, plugins, themes) = cx
                    .background_executor()
                    .spawn(async {
                        let root = super::plugins::plugins_root();
                        let personal = super::scan::skills_personal_dir();
                        snapshot(
                            &root,
                            &personal,
                            &daruda_store::persistence::status_icon_themes_dir(),
                        )
                    })
                    .await;
                cx.update(|cx| {
                    cx.update_global::<crate::status_icon_themes::StatusIconThemes, _>(
                        |state, _| {
                            state.0 = themes;
                        },
                    );
                    cx.update_global::<GlobalSkills, _>(|state, _| {
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
    themes_dir: &std::path::Path,
) -> (
    crate::hooks::source_watch::SourceWatch,
    Vec<super::Skill>,
    crate::status_icon_themes::Catalog,
) {
    let installs = super::plugins::read_installed_plugins(&root.join("installed_plugins.json"));
    let sources = crate::status_icon_themes::sources(themes_dir, &installs);
    let mut paths = vec![root.to_path_buf(), personal.to_path_buf()];
    paths.extend(installs.iter().map(|p| p.install_path.clone()));
    paths.extend(
        super::plugins::read_known_marketplaces(&root.join("known_marketplaces.json"))
            .into_iter()
            .map(|p| p.install_location),
    );
    paths.extend(sources.iter().map(|(_, path)| path.clone()));
    let watch = crate::hooks::source_watch::SourceWatch::new(paths);
    (
        watch,
        super::scan::scan_plugins_at(root, personal),
        crate::status_icon_themes::load(&sources),
    )
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
        let (watch, plugins, _) = snapshot(
            home.path(),
            &home.path().join("personal"),
            &home.path().join("themes"),
        );
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "example:sample");
        std::fs::write(&skill, "---\nname: updated\n---\nChanged").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !watch.changed() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let (_, plugins, _) = snapshot(
            home.path(),
            &home.path().join("personal"),
            &home.path().join("themes"),
        );
        assert_eq!(plugins[0].name, "example:updated");
    }
    #[test]
    fn status_packages_are_watched_and_disappear_after_uninstall() {
        let _serial = crate::hooks::tests_common::fsevent_serial();
        let root = tempfile::tempdir().unwrap();
        let plugin = tempfile::tempdir().unwrap();
        let user = root.path().join("themes");
        let package = plugin.path().join("daruda/status-icon-themes/example");
        std::fs::create_dir_all(&package).unwrap();
        let theme = package.join("theme.json");
        let spec = serde_json::json!({"version":1,"id":"example","name":"Example","states":{"working":{"colors":{"dark":"#abcdef","light":"#123456"}}}});
        std::fs::write(&theme, spec.to_string()).unwrap();
        let manifest_path = root.path().join("installed_plugins.json");
        let manifest = serde_json::json!({"plugins":{"example@market":[{"scope":"user","installPath":plugin.path()}]}});
        std::fs::write(&manifest_path, manifest.to_string()).unwrap();
        let (watch, _, catalog) = snapshot(root.path(), &root.path().join("personal"), &user);
        assert!(catalog.themes.contains_key("plugin/example@market/example"));
        let mut changed = spec;
        changed["name"] = "Changed".into();
        std::fs::write(&theme, changed.to_string()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !watch.changed() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let (_, _, changed) = snapshot(root.path(), &root.path().join("personal"), &user);
        assert_eq!(
            changed.themes["plugin/example@market/example"].name,
            "Changed"
        );
        std::fs::write(&manifest_path, r#"{"plugins":{}}"#).unwrap();
        let (_, _, removed) = snapshot(root.path(), &root.path().join("personal"), &user);
        assert!(removed.themes.is_empty());
        let (watch, _, _) = snapshot(root.path(), &root.path().join("personal"), &user);
        std::fs::create_dir_all(user.join("personal")).unwrap();
        std::fs::write(user.join("personal/theme.json"), r##"{"version":1,"id":"personal","name":"Personal","states":{"idle":{"colors":{"dark":"#abcdef","light":"#123456"}}}}"##).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !watch.changed() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let (_, _, added) = snapshot(root.path(), &root.path().join("personal"), &user);
        assert!(added.themes.contains_key("user/personal"));
    }
}
