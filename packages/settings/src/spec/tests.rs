use super::*;

/// Every row's placeholder has to reach a real locale key. `t!` answers a
/// miss with the key path itself, which renders as `settings.foo` in the
/// field — visible only while the input is empty, so nothing else catches
/// it. The number is asserted too: a typo'd `%{value}` drops it silently.
#[test]
fn every_text_row_placeholder_resolves() {
    for row in TEXT_SETTINGS {
        let text = (row.placeholder)();
        assert!(
            !text.starts_with("settings."),
            "{:?} placeholder fell through to its key: {text}",
            row.setting
        );
        // A numeric field shows a sample number; a free-text one (a shell
        // path) has no sample value to lose.
        let numeric = (row.show)(&Config::default()).parse::<f64>().is_ok();
        assert!(
            !numeric || text.chars().any(|c| c.is_ascii_digit()),
            "{:?} placeholder lost its sample value: {text}",
            row.setting
        );
    }
}

/// The `expect` in each lookup is only safe while every variant has exactly
/// one row. `ALL` is the enum's own list, so adding a variant without a row
/// fails here rather than at the first click on the new widget.
#[test]
fn every_setting_has_exactly_one_row() {
    for setting in TextSetting::ALL {
        let n = TEXT_SETTINGS
            .iter()
            .filter(|s| s.setting == setting)
            .count();
        assert_eq!(n, 1, "{setting:?} has {n} rows in TEXT_SETTINGS");
    }
    assert_eq!(TEXT_SETTINGS.len(), TextSetting::ALL.len());

    for setting in SelectSetting::ALL {
        let n = SELECT_SETTINGS
            .iter()
            .filter(|s| s.setting == setting)
            .count();
        assert_eq!(n, 1, "{setting:?} has {n} rows in SELECT_SETTINGS");
    }
    assert_eq!(SELECT_SETTINGS.len(), SelectSetting::ALL.len());

    for setting in BoolSetting::ALL {
        let n = BOOL_SETTINGS
            .iter()
            .filter(|s| s.setting == setting)
            .count();
        assert_eq!(n, 1, "{setting:?} has {n} rows in BOOL_SETTINGS");
    }
    assert_eq!(BOOL_SETTINGS.len(), BoolSetting::ALL.len());
}

/// `show` and `read` are the two directions of one mapping, and a select's
/// value round-trips through them. A row where they disagree would silently
/// reset the user's pick the next time the config changed underneath.
///
/// Compared through `Debug` because `SettingsPatch` carries no `PartialEq`;
/// that is enough to catch a row naming the wrong variant or reading a
/// different config field than it writes.
#[test]
fn a_select_value_round_trips_through_show_and_read() {
    let config = Config::default();
    for spec in SELECT_SETTINGS {
        let shown = (spec.show)(&config);
        let Some(read_back) = (spec.read)(&shown) else {
            panic!("{:?}: show() produced a value read() rejects", spec.setting);
        };
        assert_eq!(
            format!("{read_back:?}"),
            format!("{:?}", (spec.current)(&config)),
            "{:?}: show() -> read() does not land on current()",
            spec.setting
        );
    }
}

/// How `load_settings_patch` reaches a given `SettingsPatch` variant.
enum Coverage {
    /// A `spec` row owns it.
    Row,
    /// `load_settings_patch` handles it by hand (a whole sub-form).
    ByHand,
    /// This window does not show it, so it has nothing to catch up with.
    NotShown,
}

/// `load_settings_patch` used to be an exhaustive `match`, so the compiler
/// refused a new `SettingsPatch` variant with no handler. Dispatching by row
/// gives that up. This match restores it: it is exhaustive, so a new variant
/// does not compile until someone classifies it here — and the assertions
/// below then check the classification is true of the actual tables.
fn coverage_of(patch: &SettingsPatch) -> Coverage {
    match patch {
        SettingsPatch::GeneralLanguage(_)
        | SettingsPatch::TerminalPreset(_)
        | SettingsPatch::UiPreset(_)
        | SettingsPatch::TerminalFontFamily(_)
        | SettingsPatch::TerminalFontSize(_)
        | SettingsPatch::TerminalLineHeight(_)
        | SettingsPatch::TerminalCellWidth(_)
        | SettingsPatch::EditorFontFamily(_)
        | SettingsPatch::EditorFontSize(_)
        | SettingsPatch::EditorLineHeight(_)
        | SettingsPatch::AgentChatFontFamily(_)
        | SettingsPatch::AgentChatFontSize(_)
        | SettingsPatch::AgentChatLineHeight(_)
        | SettingsPatch::CursorStyle(_)
        | SettingsPatch::AgentUseModifierToSend(_)
        | SettingsPatch::AgentUseReadingWidth(_)
        | SettingsPatch::AgentToolSummaryLabels(_)
        | SettingsPatch::RenderMaxFps(_)
        | SettingsPatch::ShellClosePaneOnExit(_)
        | SettingsPatch::WindowOpacity(_)
        | SettingsPatch::WindowBlur(_)
        | SettingsPatch::ScrollbackMaxRows(_)
        | SettingsPatch::TerminalInsetX(_)
        | SettingsPatch::TerminalInsetY(_)
        | SettingsPatch::FilesShowHidden(_)
        | SettingsPatch::FilesUseGitignore(_)
        | SettingsPatch::UpdateAutoCheck(_)
        | SettingsPatch::ClaudeStatusStaleSecs(_)
        | SettingsPatch::ClaudeStatusFileTtlDays(_)
        | SettingsPatch::UsageLimitsPollSecs(_)
        | SettingsPatch::UsageStatusPollSecs(_)
        | SettingsPatch::PortsPollSecs(_)
        | SettingsPatch::LogsRetentionDays(_)
        | SettingsPatch::LogsMaxFileSizeMb(_)
        | SettingsPatch::PresenceGraceSecs(_)
        | SettingsPatch::PresenceIdleSecs(_)
        | SettingsPatch::PresenceIdleForegroundSecs(_)
        | SettingsPatch::AgentInputMaxRows(_)
        | SettingsPatch::AgentReadingWidth(_)
        | SettingsPatch::FlowTimeoutMinutes(_)
        | SettingsPatch::FlowMaxNodeRuns(_)
        | SettingsPatch::FlowMaxCost(_)
        | SettingsPatch::FlowCostCurrency(_)
        | SettingsPatch::LeftCollapsedByDefault(_)
        | SettingsPatch::PreviewTab(_)
        | SettingsPatch::LeftDefaultWidth(_)
        | SettingsPatch::ShellNaturalTextEditing(_)
        | SettingsPatch::ShellProgram(_)
        | SettingsPatch::NotifyOsc9(_)
        | SettingsPatch::NotifyOsc777(_)
        | SettingsPatch::NotifyAttention(_)
        | SettingsPatch::NotifyLongRunning(_)
        | SettingsPatch::NotifySkipFocusedPane(_)
        | SettingsPatch::ClipboardCopyOnSelect(_)
        | SettingsPatch::GitConfirmCommit(_)
        | SettingsPatch::GitConfirmPush(_)
        | SettingsPatch::GitDefaultCommitMessage(_)
        | SettingsPatch::NotifyHook(_)
        | SettingsPatch::NotifyAgentCompletion(_)
        | SettingsPatch::NotifyAgentWaiting(_)
        | SettingsPatch::TelegramOnlyWhenAway(_)
        | SettingsPatch::NotifyLongRunningThresholdSecs(_)
        | SettingsPatch::SyntaxTheme(_)
        | SettingsPatch::ClipboardStreamingMaxBytes(_)
        | SettingsPatch::PreferredEditor(_)
        | SettingsPatch::FileIconColorMode(_)
        | SettingsPatch::PanelsGridColumns(_)
        | SettingsPatch::ClaudeStatusEnabled(_)
        | SettingsPatch::TelegramEnabled(_)
        | SettingsPatch::OrchestratorEnabled(_)
        | SettingsPatch::OrchestratorAgentId(_)
        | SettingsPatch::OrchestratorAccountId(_)
        | SettingsPatch::KeepAwake(_)
        | SettingsPatch::KeepAwakeApprovalWaitMinutes(_) => Coverage::Row,
        SettingsPatch::AgentCatalog(_)
        | SettingsPatch::SessionHosts { .. }
        | SettingsPatch::RemoteChannels(_)
        | SettingsPatch::StatusBarHiddenItems(_) => Coverage::ByHand,
        // Neither is a *field* of this window. The toggle is the status
        // bar's own menu gesture; Settings writes the same list through
        // `StatusBarHiddenItems`. The Telegram chat id is owned by pairing:
        // Remote Control writes it (Unpair) and renders it from
        // `telegram_authorized_chat_id`, which `adopt_external_settings`
        // mirrors — not from this reload table.
        SettingsPatch::ToggleStatusBarItem(_) | SettingsPatch::TelegramAuthorizedChatId(_) => {
            Coverage::NotShown
        }
    }
}

/// Every variant classified `Row` above must actually have one, and no
/// other classification may. Without this the match is just a second list
/// that can drift from the tables it describes.
#[test]
fn every_reloadable_field_is_classified_the_way_the_tables_are_built() {
    let config = Config::default();
    let by_row: Vec<daruda_config::SettingsFieldId> = TEXT_SETTINGS
        .iter()
        .map(text_field_id)
        .chain(SELECT_SETTINGS.iter().map(select_field_id))
        .chain(BOOL_SETTINGS.iter().map(bool_field_id))
        .collect();

    for patch in SettingsView::settings_ui_patches(&config) {
        let field = patch.field();
        match coverage_of(&patch) {
            Coverage::Row => assert!(
                by_row.contains(&field),
                "{field:?} is classified Row but no spec table owns it",
            ),
            Coverage::ByHand => assert!(
                !by_row.contains(&field),
                "{field:?} is classified ByHand but a spec row owns it too",
            ),
            Coverage::NotShown => {
                panic!("{field:?} is declared owned by settings_ui_patches but classified NotShown")
            }
        }
    }
}

/// The same round trip for checkboxes: a mirrored field written back as a
/// patch has to address the config slot `show` reads.
#[test]
fn a_bool_value_round_trips_through_show_and_patch() {
    let mut config = Config::default();
    for spec in BOOL_SETTINGS {
        let flipped = !(spec.show)(&config);
        (spec.patch)(flipped).apply_to(&mut config);
        assert_eq!(
            (spec.show)(&config),
            flipped,
            "{:?}: patch() and show() address different config fields",
            spec.setting
        );
    }
}

/// Each mode survives the select's value and patch directions, so a pick
/// writes the variant it names and reads back as the same option.
#[test]
fn every_keep_awake_mode_round_trips_through_its_select_row() {
    use daruda_config::KeepAwakeMode;
    let spec = select_spec(SelectSetting::KeepAwakeMode);
    let mut config = Config::default();
    for (value, mode) in [
        ("off", KeepAwakeMode::Off),
        ("agent", KeepAwakeMode::Agent),
        ("on", KeepAwakeMode::On),
    ] {
        let patch = (spec.read)(value).expect("a known option");
        assert_eq!(format!("{patch:?}"), format!("KeepAwake({mode:?})"));
        patch.apply_to(&mut config);
        assert_eq!(config.desktop.keep_awake, mode);
        assert_eq!((spec.show)(&config).as_ref(), value);
    }
    assert!((spec.read)("sometimes").is_none());
}
