//! Public API tests for the hooks installer.

use daruda_agent::hooks::installer::*;
use daruda_agent::hooks::route::Harness;
use serde_json::Value;
use std::fs;
use tempfile::TempDir;

fn read(target: &HookTarget) -> Value {
    serde_json::from_slice(&fs::read(target.hooks_file()).unwrap()).unwrap()
}

#[test]
fn install_then_uninstall_round_trips_for_every_harness() {
    for harness in Harness::ALL {
        let home = TempDir::new().unwrap();
        let target = HookTarget::new(harness, home.path());
        assert!(!is_installed(&target));
        assert_eq!(install(&target).unwrap(), Change::Written);
        assert!(is_installed(&target), "{harness:?}");
        assert_eq!(install(&target).unwrap(), Change::Unchanged);
        uninstall(&target).unwrap();
        assert!(!is_installed(&target), "{harness:?}");
    }
}

#[test]
fn every_entry_runs_the_guarded_hook_command() {
    let home = TempDir::new().unwrap();
    let target = HookTarget::new(Harness::Claude, home.path());
    install(&target).unwrap();
    let settings = read(&target);
    for event in CLAUDE_EVENTS {
        let command = settings["hooks"][*event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert_eq!(command, hook_command(Harness::Claude, event));
        assert!(command.ends_with("|| true"));
    }
}

#[test]
fn is_installed_is_false_for_a_missing_or_malformed_file() {
    let home = TempDir::new().unwrap();
    let target = HookTarget::new(Harness::Claude, home.path());
    assert!(!is_installed(&target));
    fs::write(target.hooks_file(), b"{ malformed").unwrap();
    assert!(!is_installed(&target));
}

#[test]
fn codex_trust_follows_foreign_groups_and_mixed_handlers() {
    use serde_json::json;
    for mixed in [false, true] {
        let home = TempDir::new().unwrap();
        let target = HookTarget::new(Harness::Codex, home.path());
        let own = json!({"type": "command", "command": hook_command(Harness::Codex, "Stop")});
        let foreign = json!({"type": "command", "command": "echo user-hook"});
        let (groups, group, handler) = if mixed {
            (json!([{"hooks": [own, foreign]}]), 0, 1)
        } else {
            (json!([{"hooks": [own]}, {"hooks": [foreign]}]), 1, 0)
        };
        fs::write(
            target.hooks_file(),
            serde_json::to_vec(&json!({"hooks": {"Stop": groups}})).unwrap(),
        )
        .unwrap();
        let prefix = daruda_core::path::canonicalize(target.hooks_file()).unwrap();
        let old_key = format!("{}:stop:{group}:{handler}", prefix.display());
        let new_key = format!("{}:stop:0:0", prefix.display());
        let text = format!(
            "[hooks.state.{}]\ntrusted_hash = \"foreign-trust\"\ncustom = \"preserved\"\n",
            serde_json::to_string(&old_key).unwrap()
        );
        fs::write(home.path().join("config.toml"), text).unwrap();
        for enabled in [true, true, false] {
            if enabled {
                install(&target).unwrap();
            } else {
                uninstall(&target).unwrap();
            }
            let doc: toml_edit::DocumentMut = fs::read_to_string(home.path().join("config.toml"))
                .unwrap()
                .parse()
                .unwrap();
            assert_eq!(
                doc["hooks"]["state"][&new_key]["trusted_hash"].as_str(),
                Some("foreign-trust")
            );
            assert_eq!(
                doc["hooks"]["state"][&new_key]["custom"].as_str(),
                Some("preserved")
            );
            assert_eq!(
                read(&target)["hooks"]["Stop"][0]["hooks"][0]["command"],
                "echo user-hook"
            );
        }
    }
}
