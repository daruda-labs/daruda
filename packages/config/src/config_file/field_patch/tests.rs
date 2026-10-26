use super::*;

#[test]
fn nested_poll_patch_keeps_sibling_comments_and_unknown_keys() {
    let mut doc: toml_edit::DocumentMut = "# custom polling\n[usage.poll]\nlimits_secs = 60\nstatus_secs = 120 # keep status\ncustom = 7\n".parse().unwrap();
    let mut config = Config::default();
    config.usage.poll.limits_secs = 90;
    patch_settings_document(&mut doc, &config, &SettingsPatch::UsageLimitsPollSecs(90));
    assert_eq!(doc["usage"]["poll"]["limits_secs"].as_integer(), Some(90));
    assert_eq!(doc["usage"]["poll"]["custom"].as_integer(), Some(7));
    assert!(doc.to_string().contains("status_secs = 120 # keep status"));
    assert!(doc.to_string().contains("# custom polling"));
}

#[test]
fn font_patch_keeps_an_inline_parent_and_an_unwritten_sibling() {
    let mut doc: toml_edit::DocumentMut =
        "font = { terminal = { size = 14.0 }, custom = 7 } # keep inline\n"
            .parse()
            .unwrap();
    let mut config = Config::default();
    config.font.editor.size = 17.0;
    patch_settings_document(&mut doc, &config, &SettingsPatch::EditorFontSize(17.0));
    assert!(doc["font"].is_inline_table());
    assert_eq!(doc["font"]["custom"].as_integer(), Some(7));
    assert!(doc["font"]["terminal"].get("line_height").is_none());
    assert_eq!(doc["font"]["editor"]["size"].as_float(), Some(17.0));
    assert!(doc.to_string().contains("# keep inline"));
}
