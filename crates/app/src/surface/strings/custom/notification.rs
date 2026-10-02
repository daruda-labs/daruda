/// Format a `Duration` as a compact, human-friendly span for the
/// "command finished" notification body. Examples: `42s`, `1m 03s`,
/// `2h 15m`. Sub-second resolution is dropped; the user threshold
/// is in whole seconds and any "completed in <1s" command is below
/// the long-running cut-off anyway.
pub(crate) fn format_duration_compact(d: std::time::Duration) -> String {
    let total_secs = d.as_secs();
    if total_secs < 60 {
        return format!("{total_secs}s");
    }
    let hours = total_secs / 3_600;
    let minutes = (total_secs % 3_600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{minutes}m {seconds:02}s")
    }
}
