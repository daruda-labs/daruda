use super::*;

#[test]
fn workspace_retains_versions_features_and_target_dependencies() {
    let upstream: Value = toml::from_str(
        r#"
        [workspace.package]
        edition = "2024"
        [workspace.lints.rust]
        unsafe_op_in_unsafe_fn = "deny"
        [workspace.dependencies]
        local = { path = "crates/local", package = "actual", features = ["base"] }
        registry = { version = "1", default-features = false }
        unused = "2"
    "#,
    )
    .unwrap();
    let gpui: Value = toml::from_str(
        r#"
        [dependencies]
        local = { workspace = true, features = ["extra"] }
        [target.'cfg(windows)'.dependencies]
        registry.workspace = true
    "#,
    )
    .unwrap();
    let result = workspace(&upstream, &gpui, "https://example.test/zed", "pinned").unwrap();
    let dependencies = &result["workspace"]["dependencies"];
    assert_eq!(
        dependencies["local"]["git"].as_str(),
        Some("https://example.test/zed")
    );
    assert_eq!(dependencies["local"]["rev"].as_str(), Some("pinned"));
    assert_eq!(dependencies["local"]["package"].as_str(), Some("actual"));
    assert_eq!(dependencies["local"]["features"][0].as_str(), Some("base"));
    assert!(dependencies["local"].get("path").is_none());
    assert_eq!(
        dependencies["registry"],
        upstream["workspace"]["dependencies"]["registry"]
    );
    assert!(dependencies.get("unused").is_none());
}

#[test]
fn missing_workspace_dependency_is_an_error() {
    let upstream: Value = toml::from_str("[workspace.dependencies]").unwrap();
    let gpui: Value = toml::from_str("[dependencies]\nmissing.workspace = true").unwrap();
    assert!(workspace(&upstream, &gpui, "url", "rev").is_err());
}

#[test]
fn all_zed_consumers_must_share_the_pin_and_override() {
    let mut root: Value = toml::from_str(
        r#"
        [workspace.dependencies]
        gpui = { git = "zed", rev = "pinned" }
        gpui_platform = { git = "zed", rev = "pinned" }
        [patch.zed]
        gpui = { path = "vendor/zed/crates/gpui" }
    "#,
    )
    .unwrap();
    assert!(pinned_source(&root).is_ok());
    root["workspace"]["dependencies"]["gpui_platform"]["rev"] = "different".into();
    assert!(pinned_source(&root).is_err());
    root["workspace"]["dependencies"]["gpui_platform"]["rev"] = "pinned".into();
    root["patch"]["zed"]["gpui"]["path"] = "unpatched".into();
    assert!(pinned_source(&root).is_err());
}
