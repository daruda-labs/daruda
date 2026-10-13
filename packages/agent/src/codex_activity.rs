//! Local aggregation of Codex CLI activity from its rollout session JSONL
//! files, mirroring [`crate::activity`]'s Claude aggregator but reading a
//! different on-disk layout: `<CODEX_HOME>/sessions/<YYYY>/<MM>/<DD>/*.jsonl`
//! instead of Claude's flat `<projects_root>/<project>/<session>.jsonl`.
//!
//! Counting semantics, matched against real Codex session files:
//! - `turns`: `event_msg` records whose `payload.type == "task_started"` —
//!   one per human prompt, mirroring Claude's non-tool-result `user` record.
//! - `tokens`: `event_msg` records whose `payload.type == "token_count"`,
//!   summing `payload.info.last_token_usage.total_tokens` — the *delta* for
//!   that one model call. (`total_token_usage` is the session's running
//!   cumulative total and must not be summed.)
//! - Day attribution: the record's UTC `timestamp` converted to the
//!   **local** date, same as Claude.
//!
//! No sqlite dependency: Codex's `state_*.sqlite` also carries token totals,
//! but the rollout JSONL already carries both turns and tokens, so the
//! incremental byte-offset scan used for Claude applies unchanged.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::activity::{ActivityError, ActivitySource, ActivityStats, session_prompt_preview};
use crate::activity_scan::{
    FileEntry, SessionLogFormat, epoch_millis, local_date, update_activity as update_jsonl_activity,
};

/// Bumped whenever the cache schema or counting semantics change.
///
/// v2: `FileEntry` gained `session_id`/`cwd`/`last_active_ms` for
/// [`ActivityStats::recent_sessions`] — a v1 cache has none of these, so it
/// must rebuild rather than silently report an empty recent-sessions list
/// forever.
///
/// v3: recent sessions gained `prompt_preview` and `git_branch`, and
/// `session_id` capture now matches real rollout files' `payload.session_id`
/// field before falling back to the legacy assumed `payload.id`.
const CODEX_CACHE_VERSION: u32 = 3;

pub struct CodexActivity;

impl ActivitySource for CodexActivity {
    fn fetch(&self, config_dir: Option<&Path>) -> Option<ActivityStats> {
        let (sessions_root, cache_path) = codex_activity_paths(config_dir)?;
        update_activity(&sessions_root, &cache_path).ok()
    }
}

/// Resolve Codex's rollout session root and profile-scoped cache path.
///
/// `None` reads the system default Codex home; `Some(config_dir)` reads a
/// managed account's isolated `CODEX_HOME`. The system-default home keeps the
/// unkeyed cache name, while managed accounts key the cache by their final
/// path component so accounts do not collide.
fn codex_activity_paths(config_dir: Option<&Path>) -> Option<(PathBuf, PathBuf)> {
    let (codex_home, cache_file_name) = match config_dir {
        Some(dir) => (
            dir.to_path_buf(),
            codex_cache_file_name(dir, CacheScope::Managed),
        ),
        None => (
            crate::accounts::codex::system_codex_home()?,
            codex_cache_file_name(Path::new(".codex"), CacheScope::SystemDefault),
        ),
    };
    let sessions_root = codex_home.join("sessions");
    let cache_path = daruda_store::persistence::default_data_dir()
        .join("cache")
        .join(cache_file_name);
    Some((sessions_root, cache_path))
}

#[derive(Clone, Copy)]
enum CacheScope {
    SystemDefault,
    Managed,
}

fn codex_cache_file_name(codex_home: &Path, scope: CacheScope) -> String {
    if matches!(scope, CacheScope::SystemDefault) {
        return "codex-activity.json".to_string();
    }
    codex_home
        .file_name()
        .and_then(|name| name.to_str())
        .map(|key| format!("codex-activity-{key}.json"))
        .unwrap_or_else(|| "codex-activity-managed.json".to_string())
}

/// Refresh the activity cache against the rollout session logs under
/// `sessions_root` (`<CODEX_HOME>/sessions`) and return the aggregated
/// stats. Same incremental-cache flow as [`crate::activity::update_activity`].
pub fn update_activity(
    sessions_root: &Path,
    cache_path: &Path,
) -> Result<ActivityStats, ActivityError> {
    update_jsonl_activity::<CodexLogFormat>(sessions_root, cache_path)
}

struct CodexLogFormat;

impl SessionLogFormat for CodexLogFormat {
    const CACHE_VERSION: u32 = CODEX_CACHE_VERSION;

    /// Recursively collect every `*.jsonl` under `sessions_root`. Codex nests
    /// rollout files three levels deep (`YYYY/MM/DD/`), but this walks any
    /// depth so a layout change upstream doesn't silently stop scanning.
    fn list_logs(sessions_root: &Path) -> Result<Vec<PathBuf>, ActivityError> {
        let mut logs = Vec::new();
        match fs::read_dir(sessions_root) {
            Ok(_) => walk_dir(sessions_root, &mut logs),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(logs),
            Err(e) => {
                return Err(ActivityError::Scan {
                    path: sessions_root.to_path_buf(),
                    source: e,
                });
            }
        }
        Ok(logs)
    }

    /// Classify one rollout JSONL object. `event_msg.task_started` and
    /// `event_msg.token_count` contribute counts; `session_meta` contributes
    /// cwd/session id/branch; `event_msg.user_message` contributes the display
    /// fallback prompt preview.
    fn count_record(record: &Value, entry: &mut FileEntry) {
        let Some(kind) = record.get("type").and_then(Value::as_str) else {
            return;
        };

        if kind == "session_meta" {
            if entry.session_id.is_none()
                && let Some(id) = record
                    .pointer("/payload/session_id")
                    .or_else(|| record.pointer("/payload/id"))
                    .and_then(Value::as_str)
            {
                entry.session_id = Some(id.to_string());
            }
            if entry.cwd.is_none()
                && let Some(cwd) = record.pointer("/payload/cwd").and_then(Value::as_str)
            {
                entry.cwd = Some(PathBuf::from(cwd));
            }
            if let Some(branch) = record
                .pointer("/payload/git/branch")
                .and_then(Value::as_str)
                .and_then(non_blank_owned)
            {
                entry.git_branch = Some(branch);
            }
            return;
        }

        if kind != "event_msg" {
            return;
        }
        let Some(timestamp) = record.get("timestamp").and_then(Value::as_str) else {
            return;
        };
        let Some(date) = local_date(timestamp) else {
            return;
        };
        if let Some(ms) = epoch_millis(timestamp) {
            entry.last_active_ms = Some(entry.last_active_ms.map_or(ms, |prev| prev.max(ms)));
        }
        let Some(payload_type) = record.pointer("/payload/type").and_then(Value::as_str) else {
            return;
        };

        match payload_type {
            "task_started" => {
                entry.days.entry(date).or_default().turns += 1;
            }
            "user_message" => {
                if let Some(preview) = record
                    .pointer("/payload/message")
                    .and_then(Value::as_str)
                    .and_then(session_prompt_preview)
                {
                    entry.prompt_preview = Some(preview);
                }
            }
            "token_count" => {
                let delta = record
                    .pointer("/payload/info/last_token_usage/total_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                if delta > 0 {
                    entry.days.entry(date).or_default().tokens += delta;
                }
            }
            _ => {}
        }
    }
}

/// Best-effort recursive walk. An unreadable subdirectory is skipped rather
/// than failing the whole scan — one broken day folder must not blank the
/// stats.
fn walk_dir(dir: &Path, logs: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_dir(&path, logs);
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            logs.push(path);
        }
    }
}

fn non_blank_owned(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_paths_for_scopes_sessions_root_and_cache_to_config_dir() {
        let account_id = "alice-1234";
        let config_dir = Path::new("/data/codex-accounts").join(account_id);

        let (sessions_root, cache_path) = codex_activity_paths(Some(&config_dir))
            .expect("codex_activity_paths should resolve for an explicit config_dir");

        assert_eq!(sessions_root, config_dir.join("sessions"));
        let cache_file_name = cache_path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("cache path should have a file name");
        assert!(cache_file_name.contains(account_id));
        assert_ne!(cache_file_name, "codex-activity.json");
    }

    #[test]
    fn explicit_dot_codex_home_still_uses_a_keyed_cache_name() {
        let system_home = Path::new("/Users/alice/.codex");
        let (sessions_root, cache_path) = codex_activity_paths(Some(system_home))
            .expect("codex_activity_paths should resolve for an explicit config_dir");

        assert_eq!(sessions_root, system_home.join("sessions"));
        assert_eq!(
            cache_path.file_name().and_then(|n| n.to_str()),
            Some("codex-activity-.codex.json")
        );
    }

    #[test]
    fn system_scope_uses_the_unkeyed_cache_name() {
        assert_eq!(
            codex_cache_file_name(Path::new("/Users/alice/.codex"), CacheScope::SystemDefault),
            "codex-activity.json"
        );
    }
}
