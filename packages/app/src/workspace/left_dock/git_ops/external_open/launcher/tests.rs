use super::*;

fn preset_named(name: &'static str) -> daruda_config::ExternalEditorPreset {
    *daruda_config::external_editor_preset(name).expect("test preset must exist in PRESETS")
}

#[test]
fn single_edition_preset_yields_one_open_dash_a_candidate() {
    let preset = preset_named("vscode");
    let candidates = preset_launch_candidates(std::path::Path::new("/tmp/f.rs"), &preset);
    if cfg!(target_os = "macos") {
        assert_eq!(candidates.len(), 1);
        let (cmd, args) = &candidates[0];
        assert_eq!(*cmd, "open");
        assert_eq!(
            args,
            &[
                std::ffi::OsString::from("-a"),
                std::ffi::OsString::from("Visual Studio Code"),
                std::ffi::OsString::from("/tmp/f.rs"),
            ]
        );
    } else {
        // Linux and Windows alike: the editor's own CLI.
        assert_eq!(
            candidates,
            vec![("code", vec![std::ffi::OsString::from("/tmp/f.rs")])]
        );
    }
}

#[test]
fn multi_edition_preset_yields_one_candidate_per_bundle_id_on_macos() {
    let preset = preset_named("intellij");
    let candidates = preset_launch_candidates(std::path::Path::new("/tmp/f.rs"), &preset);
    if cfg!(target_os = "macos") {
        assert_eq!(candidates.len(), 2);
        for (cmd, args) in &candidates {
            assert_eq!(*cmd, "open");
            assert_eq!(args[0], std::ffi::OsString::from("-b"));
            assert_eq!(args[2], std::ffi::OsString::from("/tmp/f.rs"));
        }
        assert_eq!(
            candidates[0].1[1],
            std::ffi::OsString::from("com.jetbrains.intellij")
        );
        assert_eq!(
            candidates[1].1[1],
            std::ffi::OsString::from("com.jetbrains.intellij.ce")
        );
    }
}

#[test]
fn macos_only_preset_has_no_cli_candidates() {
    let preset = preset_named("xcode");
    if !cfg!(target_os = "macos") {
        assert!(preset_launch_candidates(std::path::Path::new("/tmp/f.rs"), &preset).is_empty());
    }
}
