use super::*;
use serde_json::json;
use tempfile::TempDir;

/// Noon UTC — far enough from both midnights that the local date
/// equals what chrono computes for every real-world timezone the
/// test host could run in.
const D1: &str = "2026-06-01T12:00:00Z";

fn user_line(ts: &str, sid: &str) -> String {
    json!({"type": "user", "timestamp": ts, "sessionId": sid,
           "message": {"role": "user", "content": "hi"}})
    .to_string()
        + "\n"
}

/// `projects_root/<project>/<name>.jsonl` ← `content`.
fn write_log(root: &Path, project: &str, name: &str, content: &str) -> PathBuf {
    let dir = root.join(project);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.jsonl"));
    fs::write(&path, content).unwrap();
    path
}

fn fixture() -> (TempDir, PathBuf, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let projects = tmp.path().join("projects");
    let cache = tmp.path().join("cache").join("activity.json");
    (tmp, projects, cache)
}

#[test]
fn activity_paths_for_scopes_projects_root_and_cache_to_config_dir() {
    let account_id = "alice-1234";
    let config_dir = Path::new("/data/claude-accounts").join(account_id);

    let (projects_root, cache_path) = claude_activity_paths(Some(&config_dir))
        .expect("claude_activity_paths should resolve for an explicit config_dir");

    assert_eq!(projects_root, config_dir.join("projects"));

    let cache_file_name = cache_path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("cache path should have a file name");
    assert!(
        cache_file_name.contains(account_id),
        "cache file name {cache_file_name:?} should embed the account key {account_id:?}"
    );
    assert_ne!(
        cache_file_name, "activity.json",
        "account-scoped cache must not collide with the system-default cache file name"
    );
}

#[test]
fn cache_version_mismatch_triggers_full_rebuild() {
    let (_tmp, projects, cache) = fixture();
    write_log(&projects, "p", "a", &user_line(D1, "s1"));
    update_activity(&projects, &cache).unwrap();

    // Rewrite the cache as a future version with bogus contents —
    // it must be discarded, not trusted.
    let raw = fs::read_to_string(&cache).unwrap();
    let mut v: Value = serde_json::from_str(&raw).unwrap();
    v["version"] = json!(CLAUDE_CACHE_VERSION + 1);
    v["files"] = json!({});
    fs::write(&cache, v.to_string()).unwrap();

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(stats.daily[0].turns, 1);
}
