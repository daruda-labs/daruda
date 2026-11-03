use std::fs;

use serde_json::{Value, json};
use tempfile::TempDir;

use super::*;

fn target(dir: &TempDir, harness: Harness) -> HookTarget {
    HookTarget::new(harness, dir.path().join("home"))
}

fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn seed(target: &HookTarget, value: &Value) {
    fs::create_dir_all(&target.home).unwrap();
    fs::write(
        target.hooks_file(),
        serde_json::to_vec_pretty(value).unwrap(),
    )
    .unwrap();
}

/// Paradev (Superset) entry shape found in a real `~/.claude/settings.json`.
fn foreign_command() -> &'static str {
    "[ -n \"$SUPERSET_HOME_DIR\" ] && SUPERSET_HOOK_HARNESS=claude \"$SUPERSET_HOME_DIR/hooks/notify.sh\" || true"
}

#[test]
fn the_command_is_a_guarded_no_op_outside_a_daruda_terminal() {
    let command = hook_command(Harness::Codex, "Stop");
    let bin = process_env::HOOK_BIN.name();
    assert_eq!(
        command,
        format!("[ -n \"${bin}\" ] && \"${bin}\" --hook Stop --harness codex || true")
    );
    assert!(is_daruda_command(&command));
}

/// Without the route variables the command must succeed and do nothing,
/// or an agent run outside daruda would see a failing hook.
#[cfg(unix)]
#[test]
fn the_command_succeeds_without_a_route() {
    let status = daruda_core::process::command(daruda_core::shell::posix_tool("bash"))
        .arg("-c")
        .arg(hook_command(Harness::Claude, "Stop"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
}

#[cfg(unix)]
#[test]
fn the_command_runs_the_routed_binary_with_its_harness() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::Builder::new()
        .prefix("hook ' $USER `echo expanded` ")
        .tempdir()
        .unwrap();
    let log = dir.path().join("called");
    let fake = dir.path().join("my daruda");
    fs::write(
        &fake,
        format!(
            "#!/bin/sh\necho \"$@\" > {}\n",
            daruda_core::shell::quote::quote_path(&log, daruda_core::shell::Shell::Posix)
        ),
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let status = daruda_core::process::command(daruda_core::shell::posix_tool("bash"))
        .arg("-c")
        .arg(hook_command(Harness::Codex, "Interrupt"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(process_env::HOOK_BIN.name(), &fake)
        .stdin(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        fs::read_to_string(&log).unwrap().trim(),
        "--hook Interrupt --harness codex"
    );
}

#[test]
fn install_registers_every_claude_event_once() {
    let dir = TempDir::new().unwrap();
    let target = target(&dir, Harness::Claude);
    assert_eq!(install(&target).unwrap(), Change::Written);
    let settings = read(&target.hooks_file());
    for event in CLAUDE_EVENTS {
        let groups = settings["hooks"][*event].as_array().unwrap();
        assert_eq!(groups.len(), 1, "{event}");
        let handler = &groups[0]["hooks"][0];
        assert_eq!(handler["command"], hook_command(Harness::Claude, event));
        assert_eq!(handler["timeout"], CLAUDE_TIMEOUT_SECS);
    }
    assert_eq!(
        settings["hooks"]["Notification"][0]["matcher"],
        "permission_prompt|elicitation_dialog"
    );
    assert!(is_installed(&target));
}

#[test]
fn a_second_install_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let target = target(&dir, Harness::Claude);
    install(&target).unwrap();
    let first = fs::read(target.hooks_file()).unwrap();
    assert_eq!(install(&target).unwrap(), Change::Unchanged);
    assert_eq!(fs::read(target.hooks_file()).unwrap(), first);
}

#[test]
fn install_keeps_other_tools_and_replaces_legacy_daruda_entries() {
    let dir = TempDir::new().unwrap();
    let target = target(&dir, Harness::Claude);
    seed(
        &target,
        &json!({
            "model": "sonnet",
            "hooks": {
                "Stop": [
                    { "hooks": [{ "type": "command", "command": foreign_command() }] },
                    { "matcher": "", "hooks": [{
                        "type": "command",
                        "command": "'/Users/me/.daruda/hooks/notify.sh' Stop",
                        "timeout": 10
                    }] }
                ]
            }
        }),
    );
    install(&target).unwrap();
    let settings = read(&target.hooks_file());
    assert_eq!(settings["model"], "sonnet");
    let stop = settings["hooks"]["Stop"].as_array().unwrap();
    assert_eq!(stop.len(), 2, "foreign entry plus one daruda entry");
    assert_eq!(stop[0]["hooks"][0]["command"], foreign_command());
    assert_eq!(
        stop[1]["hooks"][0]["command"],
        hook_command(Harness::Claude, "Stop")
    );
}

#[test]
fn a_group_shared_with_another_tool_keeps_that_tools_handler() {
    let dir = TempDir::new().unwrap();
    let target = target(&dir, Harness::Claude);
    seed(
        &target,
        &json!({ "hooks": { "Stop": [{ "hooks": [
            { "type": "command", "command": foreign_command() },
            { "type": "command", "command": hook_command(Harness::Claude, "Stop") }
        ] }] } }),
    );
    uninstall(&target).unwrap();
    let settings = read(&target.hooks_file());
    let handlers = settings["hooks"]["Stop"][0]["hooks"].as_array().unwrap();
    assert_eq!(handlers.len(), 1);
    assert_eq!(handlers[0]["command"], foreign_command());
}

#[test]
fn uninstall_prunes_events_left_empty_and_keeps_a_missing_file_missing() {
    let dir = TempDir::new().unwrap();
    let target = target(&dir, Harness::Claude);
    assert_eq!(uninstall(&target).unwrap(), Change::Unchanged);
    assert!(!target.hooks_file().exists());
    install(&target).unwrap();
    uninstall(&target).unwrap();
    assert!(!is_installed(&target));
    assert_eq!(read(&target.hooks_file())["hooks"], json!({}));
}

#[test]
fn malformed_or_mis_shaped_files_are_refused_untouched() {
    let dir = TempDir::new().unwrap();
    let target = target(&dir, Harness::Claude);
    fs::create_dir_all(&target.home).unwrap();
    fs::write(target.hooks_file(), b"{ malformed").unwrap();
    assert!(matches!(install(&target), Err(InstallerError::Json(_))));
    assert_eq!(fs::read(target.hooks_file()).unwrap(), b"{ malformed");

    seed(&target, &json!({ "hooks": { "PreToolUse": "broken" } }));
    match install(&target) {
        Err(InstallerError::BadSchema { event, .. }) => assert_eq!(event, "PreToolUse"),
        other => panic!("expected BadSchema, got {other:?}"),
    }
    assert_eq!(read(&target.hooks_file())["hooks"]["PreToolUse"], "broken");

    seed(&target, &json!([1, 2]));
    assert!(matches!(
        install(&target),
        Err(InstallerError::BadSchema { .. })
    ));
}

#[test]
fn codex_entries_carry_their_matchers_and_trust() {
    let dir = TempDir::new().unwrap();
    let target = target(&dir, Harness::Codex);
    install(&target).unwrap();
    let hooks = read(&target.hooks_file());
    assert_eq!(
        hooks["hooks"]["PreToolUse"][0]["matcher"],
        "^request_user_input$"
    );
    assert!(hooks["hooks"]["Stop"][0].get("matcher").is_none());
    assert!(
        hooks["hooks"]["Stop"][0]["hooks"][0]
            .get("timeout")
            .is_none()
    );

    let config: toml_edit::DocumentMut = fs::read_to_string(target.home.join("config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let state = config["hooks"]["state"].as_table().unwrap();
    assert_eq!(state.len(), CODEX_EVENTS.len());
    let prefix = canonical(&target.hooks_file())
        .to_string_lossy()
        .into_owned();
    let stop = &state[&format!("{prefix}:stop:0:0")]["trusted_hash"];
    assert!(stop.as_str().unwrap().starts_with("sha256:"));
}

#[test]
fn codex_trust_follows_the_entry_and_spares_other_settings() {
    let dir = TempDir::new().unwrap();
    let target = target(&dir, Harness::Codex);
    fs::create_dir_all(&target.home).unwrap();
    fs::write(
        target.home.join("config.toml"),
        "# mine\nmodel = \"x\"\n\n[hooks.state.\"/elsewhere:stop:0:0\"]\ntrusted_hash = \"keep\"\n",
    )
    .unwrap();
    seed(
        &target,
        &json!({ "hooks": { "Stop": [
            { "hooks": [{ "type": "command", "command": foreign_command() }] }
        ] } }),
    );
    install(&target).unwrap();
    let prefix = canonical(&target.hooks_file())
        .to_string_lossy()
        .into_owned();
    let text = fs::read_to_string(target.home.join("config.toml")).unwrap();
    assert!(text.starts_with("# mine\nmodel = \"x\""), "{text}");
    assert!(text.contains("trusted_hash = \"keep\""));
    // daruda's Stop entry sits after the foreign group.
    assert!(text.contains(&format!("{prefix}:stop:1:0")), "{text}");
    assert!(!text.contains(&format!("{prefix}:stop:0:0")), "{text}");

    uninstall(&target).unwrap();
    let text = fs::read_to_string(target.home.join("config.toml")).unwrap();
    assert!(!text.contains(&prefix), "{text}");
    assert!(text.contains("trusted_hash = \"keep\""));
}

#[test]
fn system_targets_name_each_agents_hook_file() {
    let claude = HookTarget::new(Harness::Claude, "/h/.claude");
    assert_eq!(claude.hooks_file(), Path::new("/h/.claude/settings.json"));
    let codex = HookTarget::new(Harness::Codex, "/h/.codex");
    assert_eq!(codex.hooks_file(), Path::new("/h/.codex/hooks.json"));
}

#[cfg(unix)]
#[test]
fn a_symlinked_configuration_is_edited_at_its_target() {
    let dir = TempDir::new().unwrap();
    let target = target(&dir, Harness::Claude);
    let dotfiles = dir.path().join("dotfiles").join("settings.json");
    fs::create_dir_all(dotfiles.parent().unwrap()).unwrap();
    fs::write(&dotfiles, b"{}").unwrap();
    fs::create_dir_all(&target.home).unwrap();
    daruda_core::path::symlink(&dotfiles, target.hooks_file()).unwrap();

    install(&target).unwrap();
    assert!(
        fs::symlink_metadata(target.hooks_file())
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(read(&dotfiles)["hooks"]["Stop"].is_array());
}
