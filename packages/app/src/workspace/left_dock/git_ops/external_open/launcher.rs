//! Editor command selection and native execution, independent of GPUI.

/// One command and its arguments for a preset editor.
type LaunchCandidate = (&'static str, Vec<std::ffi::OsString>);

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
#[cfg(not(test))]
pub(super) fn open_with_preset(
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
mod tests;
