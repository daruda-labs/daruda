//! Handing a file to an app outside daruda — the preferred external
//! editor, else the OS default handler.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use daruda_store::project::{LaneId, LaneRef};
use gpui::Context;

use crate::workspace::Workspace;

/// One command + its arguments, a candidate way to launch a preset editor.
type LaunchCandidate = (&'static str, Vec<std::ffi::OsString>);

impl Workspace {
    /// Open `path` from lane `lane_id` using the system default
    /// application. Runs the `open` command on a background thread so the
    /// UI thread is never blocked.
    ///
    /// `path` may be either lane-relative (Files left-dock convention) or
    /// absolute (Git Changes left-dock uses repo-root-relative paths and joins
    /// against repo_root before calling). `Path::join` returns the absolute
    /// argument unchanged, so the same code handles both cases.
    ///
    /// Launches `self.mirrors.preferred_editor` (`daruda_config::editor` preset name,
    /// Settings → External Editor) when set and recognized; an empty or
    /// unrecognized preference falls back to the OS default handler, same as
    /// before that setting existed.
    pub(in crate::workspace) fn open_file_externally(
        &mut self,
        lane: LaneRef,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(wt) = self.lane_for(lane) else {
            return;
        };
        let full_path = wt.path.join(&path);
        let preset = daruda_config::external_editor_preset(&self.mirrors.preferred_editor);
        self.spawn_external_open(full_path, preset, cx);
    }

    /// Open `path` in the OS default handler, ignoring the external-editor
    /// preference: for an image, a PDF or a directory the editor is the wrong
    /// tool, and the OS already knows the right one.
    pub(in crate::workspace) fn open_path_with_system_default(
        &mut self,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.spawn_external_open(path, None, cx);
    }

    /// The file viewer's way out of its binary placeholder: the file it is
    /// showing, handed to the OS default handler. `path` follows the viewer's
    /// own convention (absolute, or lane-relative for legacy state), same as
    /// [`Self::open_file_externally`].
    pub(in crate::workspace) fn open_lane_file_with_system_default(
        &mut self,
        lane_id: LaneId,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        let lane = LaneRef {
            project: self.active.project,
            lane: lane_id,
        };
        let Some(wt) = self.lane_for(lane) else {
            return;
        };
        let full_path = wt.path.join(&path);
        self.spawn_external_open(full_path, None, cx);
    }

    /// `open::that_detached` (the no-preset path) launches the default
    /// handler without blocking on it; `open_with_preset` waits on its own
    /// short-lived launcher commands so a failed candidate is detected.
    fn spawn_external_open(
        &mut self,
        full_path: std::path::PathBuf,
        preset: Option<&'static daruda_config::ExternalEditorPreset>,
        cx: &mut Context<Self>,
    ) {
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || {
                let result = open_with_preset(&full_path, preset);
                (full_path, result)
            },
            |ws, (full_path, result), cx| {
                if let Err(e) = result {
                    let report = ErrorReport::new(
                        crate::surface::strings::error::open_file_external_failed(),
                    )
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .at(file!(), line!())
                    .with_context("path", redact_home(&full_path))
                    .dedup("files.open_external")
                    .build();
                    ws.report_error(report, cx);
                }
            },
        )
        .detach();
    }
}

/// Command candidates to try, in order, to open `path` in `preset`'s
/// application on the current OS — pure decision logic, no process spawning
/// (so it's unit-testable without launching anything). Empty when `preset`
/// has no launcher for this OS (e.g. a macOS-only preset on Linux); the
/// caller falls back to the OS default handler in that case.
///
/// macOS: a multi-edition preset (non-empty `macos_bundle_ids`, e.g. IntelliJ
/// CE vs Ultimate) yields one `open -b <id>` candidate per id, since the
/// bundle id is stable across editions while the `.app` display name isn't;
/// otherwise `macos_app_name` yields a single `open -a "<name>"` candidate.
/// Linux and Windows: `cli_candidates` each yield a direct CLI-command candidate.
fn preset_launch_candidates(
    path: &std::path::Path,
    preset: &daruda_config::ExternalEditorPreset,
) -> Vec<LaunchCandidate> {
    use std::ffi::OsString;

    let path_arg = || path.as_os_str().to_owned();

    if cfg!(target_os = "macos") {
        if !preset.macos_bundle_ids.is_empty() {
            return preset
                .macos_bundle_ids
                .iter()
                .map(|id| {
                    (
                        "open",
                        vec![OsString::from("-b"), OsString::from(*id), path_arg()],
                    )
                })
                .collect();
        }
        if let Some(app_name) = preset.macos_app_name {
            return vec![(
                "open",
                vec![OsString::from("-a"), OsString::from(app_name), path_arg()],
            )];
        }
        Vec::new()
    } else {
        preset
            .cli_candidates
            .iter()
            .map(|cmd| (*cmd, vec![path_arg()]))
            .collect()
    }
}

/// Open `path` in `preset`'s application, or the OS default handler when
/// `preset` is `None` or has no launcher for this OS. Tries
/// [`preset_launch_candidates`] in order; if at least one candidate exists but
/// all fail, returns the last error rather than silently falling back to the
/// OS default — the user explicitly chose this editor, so launching something
/// else instead would be more surprising than an error.
///
/// Waits for each candidate's exit status (`.status()`, not `.spawn()`) —
/// required to actually detect a failed candidate. Every candidate here is a
/// short-lived *launcher* (macOS `open`, or an editor's own CLI entry point
/// like `code`/`idea`), not the editor itself: it forks the real GUI app and
/// returns in well under a second either way, so waiting for it doesn't wait
/// for the editor window to close. This distinction matters for the
/// macOS multi-edition candidates specifically — `open -b <bundle-id>` still
/// spawns successfully even when no app has that bundle id (the failure only
/// surfaces in `open`'s own exit code), so `.spawn()`'s `Ok` would have
/// wrongly looked like success on the very first candidate and never fallen
/// through to the next edition's bundle id.
///
/// Runs on a background thread (called from `spawn_bg_work_and_mutate`'s
/// worker closure), so blocking here doesn't block the UI. In the
/// unanticipated case of an editor CLI that doesn't detach, this would hold
/// one `background_executor` worker until the user closes that editor —
/// accepted because every built-in preset's launcher is a well-established
/// detach-and-return CLI (`code`, `subl`, `idea`, macOS `open`, …), not a
/// theoretical risk for the shipped catalog.
fn open_with_preset(
    path: &std::path::Path,
    preset: Option<&daruda_config::ExternalEditorPreset>,
) -> std::io::Result<()> {
    use std::io::Error;
    use std::process::Stdio;

    let candidates = preset
        .map(|p| preset_launch_candidates(path, p))
        .unwrap_or_default();
    let mut last_err = None;
    for (command, args) in &candidates {
        match daruda_core::process::command_on_path(command, None)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => last_err = Some(Error::other(format!("{command} exited with {status}"))),
            Err(e) => last_err = Some(e),
        }
    }
    match last_err {
        Some(e) => Err(e),
        None => open::that_detached(path),
    }
}

#[cfg(test)]
mod open_with_preset_tests {
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
            assert!(
                preset_launch_candidates(std::path::Path::new("/tmp/f.rs"), &preset).is_empty()
            );
        }
    }
}
