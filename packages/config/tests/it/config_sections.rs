//! Public API tests for config sections.

use daruda_config::*;

// ---- Project layer (resolve / round-trip) ----

#[test]
fn resolve_with_empty_project_returns_user_unchanged() {
    let user = Config::default();
    let project = ProjectConfig::default();
    let effective = user.clone().resolve(&project);
    assert_eq!(effective.shell.program, user.shell.program);
    assert_eq!(
        effective.shell.close_pane_on_exit,
        user.shell.close_pane_on_exit
    );
    assert_eq!(effective.font.terminal.size, user.font.terminal.size);
}

#[test]
fn resolve_overrides_shell_section_only() {
    let mut user = Config::default();
    user.font.terminal.size = 17.0;
    user.shell.program = Some("/bin/bash".into());

    let project_shell = ShellConfig {
        program: Some("/usr/local/bin/zsh".into()),
        close_pane_on_exit: false,
        natural_text_editing: true,
        restore_output: false,
    };
    let project = ProjectConfig {
        shell: Some(project_shell),
    };

    let effective = user.clone().resolve(&project);
    // Shell entirely replaced (section-level override).
    assert_eq!(
        effective.shell.program.as_deref(),
        Some("/usr/local/bin/zsh")
    );
    assert!(!effective.shell.close_pane_on_exit);
    // Other sections unchanged.
    assert_eq!(effective.font.terminal.size, 17.0);
}

#[test]
fn resolve_section_replace_drops_user_subkeys() {
    // `resolve` is wholesale section replacement — when project sets
    // `[shell]`, every field in the user shell section is dropped,
    // including ones the project file omits. The user opts in to this
    // by adding `[shell]` to the project config at all.
    let mut user = Config::default();
    user.shell.program = Some("/bin/bash".into());
    user.shell.close_pane_on_exit = false;

    let project = ProjectConfig {
        shell: Some(ShellConfig::default()),
    };
    let effective = user.resolve(&project);
    assert!(effective.shell.program.is_none());
    assert!(effective.shell.close_pane_on_exit);
}

// ---- telegram round-trip ----

#[test]
fn patch_config_file_round_trips_telegram_enabled_and_chat_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let mut cfg = Config::default();
    cfg.telegram.enabled = true;
    cfg.telegram.authorized_chat_id = Some(999888777);
    cfg.telegram.only_when_away = false;
    cfg.presence.away_grace_secs = 30;
    cfg.presence.away_idle_secs = 5;
    daruda_config::patch_config_file_to(&cfg, &path).unwrap();

    let reloaded = Config::load_from(&path);
    assert!(reloaded.telegram.enabled);
    assert_eq!(reloaded.telegram.authorized_chat_id, Some(999888777));
    assert!(!reloaded.telegram.only_when_away);
    assert_eq!(reloaded.presence.away_grace_secs, 30);
    assert_eq!(reloaded.presence.away_idle_secs, 5);

    cfg.presence.away_grace_secs = 0;
    daruda_config::patch_config_file_to(&cfg, &path).unwrap();
    assert_eq!(Config::load_from(&path).presence.away_grace_secs, 0);
}

/// The absence thresholds moved out of `[telegram]` into `[presence]`. A
/// config still carrying the old keys must not keep them beside the new ones,
/// where a stale value would read as live configuration.
#[test]
fn patch_config_file_drops_the_superseded_telegram_timing_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[telegram]\nenabled = true\ndefer_while_active = true\nactive_idle_secs = 60\naway_grace_secs = 15\n",
    )
    .unwrap();

    let mut cfg = Config::load_from(&path);
    cfg.presence.away_idle_secs = 90;
    daruda_config::patch_config_file_to(&cfg, &path).unwrap();

    // Assert against the parsed `[telegram]` table, not the raw text: the new
    // `[presence]` section contains `away_grace_secs` as a substring, so a
    // text scan cannot tell the retired key from its successor.
    let doc: toml_edit::DocumentMut = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    let telegram = doc["telegram"].as_table().expect("[telegram] table");
    for legacy in ["defer_while_active", "active_idle_secs", "away_grace_secs"] {
        assert!(
            !telegram.contains_key(legacy),
            "[telegram].{legacy} should be gone:\n{doc}"
        );
    }
    assert!(telegram.contains_key("only_when_away"));
    assert_eq!(Config::load_from(&path).presence.away_idle_secs, 90);
}

/// The retired keys must be *migrated*, not merely stripped: they stop having
/// any effect at load (`#[serde(default)]` ignores unknown fields), so a
/// write-time removal alone would silently reset a customized install.
#[test]
fn loading_migrates_the_retired_telegram_timing_keys_into_their_new_homes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[telegram]\nenabled = true\ndefer_while_active = false\nactive_idle_secs = 90\naway_grace_secs = 30\n",
    )
    .unwrap();

    let cfg = Config::load_from(&path);
    assert!(
        !cfg.telegram.only_when_away,
        "defer_while_active = false meant send-always"
    );
    assert_eq!(cfg.presence.away_grace_secs, 30);
    assert_eq!(cfg.presence.away_idle_secs, 90);

    // Round-tripping keeps the migrated values and drops the old spellings.
    daruda_config::patch_config_file_to(&cfg, &path).unwrap();
    let reloaded = Config::load_from(&path);
    assert!(!reloaded.telegram.only_when_away);
    assert_eq!(reloaded.presence.away_grace_secs, 30);
    assert_eq!(reloaded.presence.away_idle_secs, 90);
}

/// A config that states both spellings keeps the new one — the retired key
/// must not override a setting the user wrote in its current form.
#[test]
fn a_new_presence_key_outranks_the_retired_telegram_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[telegram]\nonly_when_away = true\naway_grace_secs = 30\n[presence]\naway_grace_secs = 45\n",
    )
    .unwrap();

    let cfg = Config::load_from(&path);
    assert_eq!(cfg.presence.away_grace_secs, 45);
    assert!(cfg.telegram.only_when_away);
}

#[test]
fn explicit_presence_defaults_outrank_legacy_telegram_keys_across_save_paths() {
    for source in [
        "[telegram]\nenabled = true\nonly_when_away = true\ndefer_while_active = false\nactive_idle_secs = 90\naway_grace_secs = 30\n[presence]\naway_grace_secs = 10\naway_idle_secs = 30\n",
        "telegram = { enabled = true, only_when_away = true, defer_while_active = false, active_idle_secs = 90, away_grace_secs = 30 }\npresence = { away_grace_secs = 10, away_idle_secs = 30 }\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, source).unwrap();
        let assert_defaults = |cfg: &Config| {
            assert!(cfg.telegram.only_when_away);
            assert_eq!(cfg.presence.away_grace_secs, 10);
            assert_eq!(cfg.presence.away_idle_secs, 30);
        };

        assert_defaults(&Config::load_from(&path));
        let patched = apply_settings_patch_to(&SettingsPatch::TelegramEnabled(false), &path)
            .expect("incremental settings save");
        assert!(!patched.telegram.enabled);
        assert_defaults(&patched);
        assert_defaults(&Config::load_from(&path));

        patch_config_file_to(&patched, &path).expect("full settings save");
        assert_defaults(&Config::load_from(&path));
    }
}

#[test]
fn partial_presence_migration_fills_only_missing_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[telegram]\ndefer_while_active = false\nactive_idle_secs = 90\naway_grace_secs = 30\n[presence]\naway_idle_secs = 60\n",
    )
    .unwrap();

    let cfg = Config::load_from(&path);
    assert!(!cfg.telegram.only_when_away);
    assert_eq!(cfg.presence.away_grace_secs, 30);
    assert_eq!(cfg.presence.away_idle_secs, 60);
    assert_eq!(cfg.presence.away_idle_foreground_secs, 180);
}

#[test]
fn patch_config_file_clears_telegram_chat_id_when_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let mut cfg = Config::default();
    cfg.telegram.enabled = true;
    cfg.telegram.authorized_chat_id = Some(111);
    daruda_config::patch_config_file_to(&cfg, &path).unwrap();

    // Unpair: authorized_chat_id goes back to None.
    cfg.telegram.authorized_chat_id = None;
    daruda_config::patch_config_file_to(&cfg, &path).unwrap();

    let reloaded = Config::load_from(&path);
    assert_eq!(reloaded.telegram.authorized_chat_id, None);
}

// ---- general.language round-trip ----

#[test]
fn patch_config_file_round_trips_language() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let mut cfg = Config::default();
    cfg.general.language = "ko".to_owned();
    daruda_config::patch_config_file_to(&cfg, &path).unwrap();

    let reloaded = Config::load_from(&path);
    assert_eq!(reloaded.general.language, "ko");
}

#[test]
fn patch_config_file_round_trips_preferred_editor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let mut cfg = Config::default();
    cfg.editor.preferred = "vscode".to_owned();
    daruda_config::patch_config_file_to(&cfg, &path).unwrap();

    let reloaded = Config::load_from(&path);
    assert_eq!(reloaded.editor.preferred, "vscode");
}

#[test]
fn config_default_language_is_auto() {
    assert_eq!(Config::default().general.language, "auto");
}

#[test]
fn config_load_missing_general_defaults_to_auto() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    // Write a config that has no [general] section.
    std::fs::write(&path, "[font]\nsize = 14.0\n").unwrap();
    let cfg = Config::load_from(&path);
    assert_eq!(cfg.general.language, "auto");
}

// ---- session host registry ----

/// Unlike `agents`, an absent/empty catalog is a valid steady state — no
/// non-empty seed. Covers both a totally empty document and a config that
/// sets unrelated sections but never mentions `session_hosts`.
#[test]
fn missing_session_hosts_defaults_to_an_empty_catalog() {
    let cfg: Config = toml::from_str("").unwrap();
    assert!(cfg.session_hosts.is_empty());
    assert!(cfg.session_host_tombstones.is_empty());

    let cfg: Config = toml::from_str("[font]\nsize = 14.0\n").unwrap();
    assert!(cfg.session_hosts.is_empty());
    assert!(cfg.session_host_tombstones.is_empty());

    assert!(Config::default().session_hosts.is_empty());
    assert!(Config::default().session_host_tombstones.is_empty());
}

#[test]
fn session_hosts_and_tombstones_round_trip_through_config_toml() {
    use daruda_store::project::SessionHostId;

    let cfg = Config {
        session_hosts: vec![
            SessionHostEntry {
                id: SessionHostId::new(),
                label: "Build box".to_string(),
                kind: SessionHostKind::Ssh {
                    target: "vm-work".to_string(),
                },
            },
            SessionHostEntry {
                id: SessionHostId::new(),
                label: "Dev container".to_string(),
                kind: SessionHostKind::Docker {
                    container: "dev-1".to_string(),
                },
            },
        ],
        session_host_tombstones: vec![SessionHostTombstone {
            old_id: SessionHostId::new(),
            kind: SessionHostKind::Ssh {
                target: "old-box".to_string(),
            },
            value: "old-box".to_string(),
            removed_at: 1_700_000_000,
            redirected_to: None,
        }],
        ..Config::default()
    };
    let toml_str = toml::to_string(&cfg).expect("serialize");
    let back: Config = toml::from_str(&toml_str).expect("deserialize");
    assert_eq!(back.session_hosts, cfg.session_hosts, "{toml_str}");
    assert_eq!(
        back.session_host_tombstones, cfg.session_host_tombstones,
        "{toml_str}"
    );
}
