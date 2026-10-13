//! Tests for the public `daruda_agent::skills::plugin_ops` API.

use daruda_agent::skills::plugin_ops::*;

#[test]
fn action_verbs_match_cli() {
    assert_eq!(PluginAction::Install.verb(), "install");
    assert_eq!(PluginAction::Uninstall.verb(), "uninstall");
}

#[test]
fn scope_flags_match_cli() {
    assert_eq!(PluginScope::User.flag(), "user");
    assert_eq!(PluginScope::Project.flag(), "project");
    assert_eq!(PluginScope::Local.flag(), "local");
}

#[test]
fn scope_default_is_user() {
    assert_eq!(PluginScope::default(), PluginScope::User);
}

#[test]
fn error_display_formats_exit_code() {
    let err = PluginOpError::Exit {
        code: Some(1),
        stderr: "boom\n".into(),
    };
    assert!(format!("{err}").contains("claude exited 1: boom"));
}
