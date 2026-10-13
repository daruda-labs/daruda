use super::*;

#[test]
fn patch_config_file_preserves_unmanaged_sections() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    // Write an initial file with a [keybindings] section that the Settings UI
    // does not touch.
    std::fs::write(
        &path,
        "[keybindings]\n\"cmd-t\" = \"new_tab\"\n\n[font]\nsize = 14.0\n",
    )
    .unwrap();

    let cfg = Config::load_from(&path);

    // Simulate Settings saving a font size change.
    let mut updated = cfg.clone();
    updated.font.terminal.size = 16.0;

    // Call a path-aware variant so we don't write to the real config location.
    patch_config_file_to(&updated, &path).unwrap();

    let reloaded = Config::load_from(&path);
    // The font size change must be applied.
    assert_eq!(reloaded.font.terminal.size, 16.0);
    // The keybinding must be preserved.
    assert_eq!(reloaded.keybindings.bindings["cmd-t"], "new_tab");
}

#[test]
fn settings_patch_migrates_legacy_font_fields_without_losing_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "# keep this comment\n[font]\nsize = 14.0\neditor_size = 17.0 # external value\n",
    )
    .unwrap();

    let written =
        crate::apply_settings_patch_to(&crate::SettingsPatch::TerminalFontSize(16.0), &path)
            .expect("field patch");

    assert_eq!(written.font.terminal.size, 16.0);
    assert_eq!(written.font.editor.size, 17.0);
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("# keep this comment"), "{text}");
    assert!(text.contains("[font.terminal]"), "{text}");
    assert!(text.contains("[font.editor]"), "{text}");
    assert!(text.contains("size = 17.0"), "{text}");
    assert!(!text.contains("editor_size"), "{text}");
}

#[test]
fn settings_patch_updates_an_inline_table_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "font = { size = 14.0, editor_size = 17.0 } # keep inline\n",
    )
    .unwrap();

    let written =
        crate::apply_settings_patch_to(&crate::SettingsPatch::TerminalFontSize(16.0), &path)
            .expect("inline field patch");

    assert_eq!(written.font.terminal.size, 16.0);
    assert_eq!(written.font.editor.size, 17.0);
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("font = {"), "{text}");
    assert!(text.contains("# keep inline"), "{text}");
}

#[test]
fn checked_settings_patch_rejects_a_same_field_disk_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[font]\nsize = 14.0\n").unwrap();
    let expected = Config::load_from(&path);
    std::fs::write(&path, "[font]\nsize = 20.0\n").unwrap();

    let error = crate::apply_settings_patch_to_if_unchanged(
        &crate::SettingsPatch::TerminalFontSize(16.0),
        &expected,
        &path,
    )
    .expect_err("same-field external edit must conflict");

    assert_eq!(
        error,
        crate::SettingsPatchApplyError::Conflict(crate::SettingsFieldId::TerminalFontSize)
    );
    assert_eq!(Config::load_from(&path).font.terminal.size, 20.0);
}

#[test]
fn checked_settings_patch_keeps_an_unrelated_disk_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[font]\nsize = 14.0\neditor_size = 13.0\n").unwrap();
    let expected = Config::load_from(&path);
    std::fs::write(&path, "[font]\nsize = 14.0\neditor_size = 17.0\n").unwrap();

    let written = crate::apply_settings_patch_to_if_unchanged(
        &crate::SettingsPatch::TerminalFontSize(16.0),
        &expected,
        &path,
    )
    .expect("unrelated external edit");

    assert_eq!(written.font.terminal.size, 16.0);
    assert_eq!(written.font.editor.size, 17.0);
}

#[cfg(unix)]
#[test]
fn settings_patch_preserves_a_config_symlink() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("managed-config.toml");
    let path = dir.path().join("config.toml");
    std::fs::write(&target, "[font]\nsize = 14.0\n").unwrap();
    symlink(&target, &path).unwrap();

    crate::apply_settings_patch_to(&crate::SettingsPatch::TerminalFontSize(16.0), &path)
        .expect("symlinked config patch");

    assert!(
        std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(Config::load_from(&target).font.terminal.size, 16.0);
}

#[test]
fn settings_patch_refuses_to_replace_a_corrupt_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let corrupt = "[font\nsize = 14";
    std::fs::write(&path, corrupt).unwrap();

    let error =
        crate::apply_settings_patch_to(&crate::SettingsPatch::TerminalFontSize(16.0), &path)
            .expect_err("corrupt config must block the write");

    assert!(error.contains("parse error"), "{error}");
    assert_eq!(std::fs::read_to_string(path).unwrap(), corrupt);
}

#[test]
fn settings_patch_round_trips_structural_session_hosts() {
    use daruda_store::project::SessionHostId;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let host = SessionHostEntry {
        id: SessionHostId::new(),
        label: "Build box".to_string(),
        kind: SessionHostKind::Ssh {
            target: "vm-work".to_string(),
        },
    };
    let tombstone = SessionHostTombstone {
        old_id: SessionHostId::new(),
        kind: SessionHostKind::Docker {
            container: "old-dev".to_string(),
        },
        value: "old-dev".to_string(),
        removed_at: 1_700_000_000,
        redirected_to: Some(host.id),
    };

    let written = crate::apply_settings_patch_to(
        &crate::SettingsPatch::SessionHosts {
            entries: vec![host.clone()],
            tombstones: vec![tombstone.clone()],
        },
        &path,
    )
    .expect("catalog patch");

    assert_eq!(written.session_hosts, vec![host]);
    assert_eq!(written.session_host_tombstones, vec![tombstone]);
}

#[test]
fn settings_patch_writes_render_max_fps() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    crate::apply_settings_patch_to(&crate::SettingsPatch::RenderMaxFps(60), &path)
        .expect("render patch");

    assert_eq!(Config::load_from(&path).render.max_fps, 60);
}

/// The Agents & Chat page's composer, reading-width and flow-limit keys
/// through the real `toml_edit` writer, read back.
#[test]
fn settings_patch_round_trips_the_agent_and_flow_keys() {
    use crate::SettingsPatch as P;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    for patch in [
        P::AgentInputMaxRows(12),
        P::AgentReadingWidth(900.0),
        P::FlowTimeoutMinutes(0),
        P::FlowMaxNodeRuns(40),
        P::FlowMaxCost(2.5),
        P::FlowCostCurrency("EUR".to_string()),
    ] {
        crate::apply_settings_patch_to(&patch, &path).expect("agent patch");
    }
    let c = Config::load_from(&path);
    assert_eq!(c.agent.input_max_rows, 12);
    assert_eq!(c.agent.reading_width, 900.0);
    assert_eq!(c.flow.timeout_minutes, 0);
    assert_eq!(c.flow.max_node_runs, 40);
    assert_eq!(c.flow.max_cost, 2.5);
    assert_eq!(c.flow.cost_currency, "EUR");
}

/// The Advanced cards' keys and the update switch through the real
/// `toml_edit` writer, read back.
#[test]
fn settings_patch_round_trips_the_advanced_keys() {
    use crate::SettingsPatch as P;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    for patch in [
        P::UpdateAutoCheck(false),
        P::ClaudeStatusStaleSecs(600),
        P::ClaudeStatusFileTtlDays(14),
        P::UsageLimitsPollSecs(0),
        P::UsageStatusPollSecs(120),
        P::PortsPollSecs(9),
        P::LogsRetentionDays(7),
        P::LogsMaxFileSizeMb(20),
        P::PresenceGraceSecs(5),
        P::PresenceIdleSecs(45),
        P::PresenceIdleForegroundSecs(240),
    ] {
        crate::apply_settings_patch_to(&patch, &path).expect("advanced patch");
    }
    let c = Config::load_from(&path);
    assert!(!c.update.auto_check);
    assert_eq!(c.claude_status.stale_threshold_secs, 600);
    assert_eq!(c.claude_status.file_ttl_days, 14);
    assert_eq!(c.usage.poll.limits_secs, 0);
    assert_eq!(c.usage.poll.status_secs, 120);
    assert_eq!(c.ports.poll_secs, 9);
    assert_eq!(c.logs.retention_days, 7);
    assert_eq!(c.logs.max_file_size_mb, 20);
    assert_eq!(c.presence.away_grace_secs, 5);
    assert_eq!(c.presence.away_idle_secs, 45);
    assert_eq!(c.presence.away_idle_foreground_secs, 240);
}

/// Reset deletes the key rather than writing the default, and keeps the
/// table and its other keys.
#[test]
fn resetting_a_field_removes_only_its_key() {
    use crate::SettingsPatch as P;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    crate::apply_settings_patch_to(&P::TerminalFontSize(20.0), &path).unwrap();
    crate::apply_settings_patch_to(&P::TerminalLineHeight(1.5), &path).unwrap();
    let before = Config::load_from(&path);

    let default = P::TerminalFontSize(Config::default().font.terminal.size);
    let after = crate::reset_settings_field_to_if_unchanged(&default, &before, &path).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    let doc: toml_edit::DocumentMut = text.parse().unwrap();
    let terminal = doc["font"]["terminal"]
        .as_table()
        .expect("terminal table kept");
    assert!(!terminal.contains_key("size"), "{text}");
    assert!(terminal.contains_key("line_height"), "{text}");
    assert_eq!(
        after.font.terminal.size,
        Config::default().font.terminal.size
    );
    assert_eq!(Config::load_from(&path).font.terminal.line_height, 1.5);
}

/// Reset lands on the default even when the file still spells the field
/// the way an older release did — the old spelling is carried away first.
#[test]
fn resetting_reaches_the_default_through_legacy_spellings() {
    use crate::SettingsPatch as P;
    let d = Config::default();
    let cases: [(&str, P); 5] = [
        (
            "[theme]\npreset = \"dracula\"\n",
            P::TerminalPreset(d.theme.terminal_preset.clone()),
        ),
        (
            "[font]\nsize = 18.0\n",
            P::TerminalFontSize(d.font.terminal.size),
        ),
        (
            "[font]\nsize = 18.0\n\n[font.terminal]\nsize = 20.0\n",
            P::TerminalFontSize(d.font.terminal.size),
        ),
        (
            "[telegram]\ndefer_while_active = false\n",
            P::TelegramOnlyWhenAway(d.telegram.only_when_away),
        ),
        (
            "[telegram]\naway_grace_secs = 99\n",
            P::PresenceGraceSecs(d.presence.away_grace_secs),
        ),
    ];
    for (text, default) in cases {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        let before = Config::load_from(&path);
        crate::reset_settings_field_to_if_unchanged(&default, &before, &path).unwrap();
        let after = Config::load_from(&path);
        assert!(
            !default.field_changed_between(&after, &Config::default()),
            "{text:?} did not reset: {}",
            std::fs::read_to_string(&path).unwrap()
        );
    }
}

/// Resetting the terminal font through the old shared `family` key keeps
/// the editor on the family the user had chosen.
#[test]
fn resetting_a_shared_flat_font_key_keeps_the_other_domain() {
    use crate::SettingsPatch as P;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[font]\nfamily = \"Iosevka\"\n").unwrap();
    let before = Config::load_from(&path);
    let default = P::TerminalFontFamily(Config::default().font.terminal.family.clone());
    crate::reset_settings_field_to_if_unchanged(&default, &before, &path).unwrap();
    let after = Config::load_from(&path);
    assert_eq!(
        after.font.terminal.family,
        Config::default().font.terminal.family
    );
    assert_eq!(after.font.editor.family, "Iosevka");
}

/// Editing one font field writes that key alone, so a field reset earlier
/// stays unwritten and keeps following the default.
#[test]
fn a_font_edit_does_not_pin_a_reset_font_key() {
    use crate::SettingsPatch as P;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    crate::apply_settings_patch_to(&P::TerminalFontSize(20.0), &path).unwrap();
    let before = Config::load_from(&path);
    let default = P::TerminalFontSize(Config::default().font.terminal.size);
    crate::reset_settings_field_to_if_unchanged(&default, &before, &path).unwrap();
    crate::apply_settings_patch_to(&P::EditorFontSize(15.0), &path).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    let doc: toml_edit::DocumentMut = text.parse().unwrap();
    let terminal = doc["font"].get("terminal").and_then(|t| t.as_table_like());
    assert!(terminal.is_none_or(|t| !t.contains_key("size")), "{text}");
    assert_eq!(Config::load_from(&path).font.editor.size, 15.0);
}

/// A field that moved on disk since the window read it is not reset.
#[test]
fn resetting_a_field_changed_on_disk_is_a_conflict() {
    use crate::SettingsPatch as P;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let stale = Config::default();
    crate::apply_settings_patch_to(&P::TerminalFontSize(20.0), &path).unwrap();

    let default = P::TerminalFontSize(Config::default().font.terminal.size);
    assert!(matches!(
        crate::reset_settings_field_to_if_unchanged(&default, &stale, &path),
        Err(crate::SettingsPatchApplyError::Conflict(_))
    ));
}

/// The Workspace page's keys through the real `toml_edit` writer, read back.
#[test]
fn settings_patch_round_trips_the_workspace_keys() {
    use crate::SettingsPatch as P;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    for patch in [
        P::LeftDefaultWidth(300.0),
        P::LeftCollapsedByDefault(true),
        P::FileIconColorMode(crate::IconColorMode::Color),
        P::PreviewTab(false),
        P::StatusBarHiddenItems(vec![crate::StatusBarItem::Ports]),
    ] {
        crate::apply_settings_patch_to(&patch, &path).expect("workspace patch");
    }

    let c = Config::load_from(&path);
    assert_eq!(c.left_dock.left_default_width, 300.0);
    assert!(c.left_dock.left_collapsed_by_default);
    assert_eq!(
        c.left_dock.file_icon_color_mode,
        crate::IconColorMode::Color
    );
    assert!(!c.file_viewer.preview_tab);
    assert_eq!(c.status_bar.hidden_items, vec![crate::StatusBarItem::Ports]);
}

/// A shell program is written as a key, and clearing it removes the key
/// rather than writing an empty program that would fail to spawn.
#[test]
fn settings_patch_writes_and_clears_the_shell_program() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    crate::apply_settings_patch_to(
        &crate::SettingsPatch::ShellProgram(Some("/bin/fish".to_string())),
        &path,
    )
    .expect("set program");
    assert_eq!(
        Config::load_from(&path).shell.program.as_deref(),
        Some("/bin/fish")
    );

    crate::apply_settings_patch_to(&crate::SettingsPatch::ShellProgram(None), &path)
        .expect("clear program");
    assert_eq!(Config::load_from(&path).shell.program, None);
    assert!(!std::fs::read_to_string(&path).unwrap().contains("program"));
}

/// Every `[notifications]` key and `telegram.only_when_away` through the real
/// `toml_edit` writer: flipped off from their `true` defaults, the threshold
/// written as an integer, all read back from disk.
#[test]
fn settings_patch_round_trips_every_notification_key() {
    use crate::SettingsPatch as P;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    for patch in [
        P::NotifyOsc9(false),
        P::NotifyOsc777(false),
        P::NotifyAttention(false),
        P::NotifyLongRunning(false),
        P::NotifySkipFocusedPane(false),
        P::NotifyHook(false),
        P::NotifyAgentCompletion(false),
        P::NotifyAgentWaiting(false),
        P::NotifyLongRunningThresholdSecs(45),
        P::TelegramOnlyWhenAway(false),
    ] {
        crate::apply_settings_patch_to(&patch, &path).expect("notification patch");
    }

    let n = Config::load_from(&path).notifications;
    assert!(!n.osc9_enabled);
    assert!(!n.osc777_enabled);
    assert!(!n.attention_enabled);
    assert!(!n.long_running_enabled);
    assert!(!n.skip_focused_pane);
    assert!(!n.hook_notification_enabled);
    assert!(!n.agent_completion_enabled);
    assert!(!n.agent_waiting_enabled);
    assert_eq!(n.long_running_threshold_secs, 45);
    assert!(!Config::load_from(&path).telegram.only_when_away);
}

/// Each of the three orchestrator keys through the real `toml_edit` writer.
/// The two optional ones matter most: a `None` has to *remove* its key, since
/// an empty string is a real agent id that resolves to nothing.
#[test]
fn settings_patch_round_trips_every_orchestrator_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let account = daruda_store::accounts::AccountId::new();

    let written =
        crate::apply_settings_patch_to(&crate::SettingsPatch::OrchestratorEnabled(true), &path)
            .expect("enabled patch");
    assert!(written.orchestrator.enabled);

    let written = crate::apply_settings_patch_to(
        &crate::SettingsPatch::OrchestratorAgentId(Some("codex-acp".to_string())),
        &path,
    )
    .expect("agent patch");
    assert_eq!(written.orchestrator.agent_id.as_deref(), Some("codex-acp"));

    let written = crate::apply_settings_patch_to(
        &crate::SettingsPatch::OrchestratorAccountId(Some(account)),
        &path,
    )
    .expect("account patch");
    assert_eq!(written.orchestrator.account_id, Some(account));

    // Clearing both optional keys leaves `enabled` alone and removes them
    // rather than persisting a value that would resolve to nothing.
    let written =
        crate::apply_settings_patch_to(&crate::SettingsPatch::OrchestratorAgentId(None), &path)
            .expect("agent clear");
    assert_eq!(written.orchestrator.agent_id, None);
    let written =
        crate::apply_settings_patch_to(&crate::SettingsPatch::OrchestratorAccountId(None), &path)
            .expect("account clear");
    assert_eq!(written.orchestrator.account_id, None);
    assert!(written.orchestrator.enabled);

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(!on_disk.contains("agent_id"), "{on_disk}");
    assert!(!on_disk.contains("account_id"), "{on_disk}");
}

#[test]
fn patch_config_file_creates_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let mut cfg = Config::default();
    cfg.font.terminal.size = 18.0;
    patch_config_file_to(&cfg, &path).unwrap();

    assert!(path.exists());
    let reloaded = Config::load_from(&path);
    assert_eq!(reloaded.font.terminal.size, 18.0);
}

#[test]
fn patch_config_file_clamps_out_of_range_values() {
    // The Settings UI may construct a Config with values outside the
    // valid range (e.g., a slider bug). patch_config_file_to clamps
    // before writing so the on-disk file never holds invalid state.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let mut cfg = Config::default();
    cfg.font.terminal.size = 999.0; // way over the 72.0 ceiling
    cfg.window.opacity = 5.0; // over 1.0
    patch_config_file_to(&cfg, &path).unwrap();

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(
        !on_disk.contains("999"),
        "expected clamped font size on disk; got: {on_disk}"
    );

    let reloaded = Config::load_from(&path);
    assert!(reloaded.font.terminal.size <= 72.0);
    assert!(reloaded.window.opacity <= 1.0);
}
