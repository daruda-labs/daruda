use super::*;
#[test]
fn only_installed_user_plugins_are_global_theme_providers() {
    use daruda_agent::skills::plugins::PluginInstall;
    let plugin = |scope: &str| PluginInstall {
        id: "sample@market".into(),
        scope: scope.into(),
        install_path: PathBuf::from("plugin"),
        version: "1".into(),
    };
    let found = sources(
        Path::new("user"),
        &[plugin("project"), plugin("local"), plugin("user")],
    );
    assert_eq!(found.len(), 2);
    assert_eq!(found[1].0, "plugin/sample@market");
    assert_eq!(
        found[1].1,
        PathBuf::from("plugin/daruda/status-icon-themes")
    );
}
