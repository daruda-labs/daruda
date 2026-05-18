use std::path::PathBuf;

use super::*;

// ---- Project ----

#[test]
fn project_from_path_extracts_name() {
    let p = Project::from_path("/Users/test/projects/daruda");
    assert_eq!(p.name, "daruda");
    assert_eq!(p.root, PathBuf::from("/Users/test/projects/daruda"));
}

#[test]
fn project_from_root_path_uses_untitled() {
    let p = Project::from_path("/");
    assert_eq!(p.name, "untitled");
}

// ---- path_hash ----

#[test]
fn path_hash_is_stable() {
    let h1 = path_hash(std::path::Path::new("/tmp/foo"));
    let h2 = path_hash(std::path::Path::new("/tmp/foo"));
    assert_eq!(h1, h2);
}

#[test]
fn path_hash_differs_for_different_paths() {
    let h1 = path_hash(std::path::Path::new("/tmp/foo"));
    let h2 = path_hash(std::path::Path::new("/tmp/bar"));
    assert_ne!(h1, h2);
}

#[test]
fn path_hash_is_16_hex_chars() {
    let h = path_hash(std::path::Path::new("/some/path"));
    assert_eq!(h.len(), 16);
    assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
}

// ---- RecentEntry ----

#[test]
fn recent_entry_now_sets_timestamp() {
    let e = RecentEntry::now("/tmp/test");
    assert!(e.last_opened > 0);
    assert_eq!(e.name, "test");
}

// ---- Serialization round-trip ----

#[test]
fn project_state_round_trip() {
    // Build via the new shape (tabs inside a worktree) to confirm
    // serialization survives a round trip without losing structure.
    let worktree = SerializedWorktree {
        id: 0,
        kind: WorktreeKind::Default,
        path: PathBuf::from("/tmp/myproject"),
        name: None,
        tab_order: 0,
        is_unread: false,
        last_activity: 0,
        tabs: vec![SerializedTab {
            layout: SerializedLayout::Leaf {
                pane_id: 1,
                cwd: Some(PathBuf::from("/tmp/myproject")),
                file: None,
            },
            last_focused_pane: 1,
            user_label: None,
        }],
        active_tab_index: 0,
        base_ref: None,
        description: None,
    };
    let state = ProjectState {
        root: PathBuf::from("/tmp/myproject"),
        worktrees: vec![worktree],
        active_worktree_id: 0,
        active_dock_view: LeftDockView::GitChanges,
        active_right_panel_view: RightDockView::default(),
        active_usage_window: UsageWindow::default(),
        tabs: Vec::new(),
        active_tab_index: 0,
        focused_pane_id: 1,
        docks: DockStates {
            left_open: true,
            left_size: 220.0,
            bottom_open: false,
            bottom_size: 200.0,
            right_open: true,
            right_size: 250.0,
        },
        window: WindowState {
            x: 100.0,
            y: 50.0,
            width: 1200.0,
            height: 800.0,
        },
        window_user_label: None,
        font_size: 14.0,
        vertical_spacing: 1.1,
        horizontal_spacing: 1.0,
    };

    let json = serde_json::to_string(&state).unwrap();
    let restored: ProjectState = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.root, state.root);
    assert_eq!(restored.worktrees.len(), 1);
    assert_eq!(restored.worktrees[0].tabs.len(), 1);
    assert!(restored.tabs.is_empty());
    assert!(restored.docks.left_open);
    assert_eq!(restored.window.width, 1200.0);
    assert_eq!(restored.font_size, 14.0);
}

#[test]
fn split_layout_round_trip() {
    let layout = SerializedLayout::Split {
        direction: SplitDirectionSerde::Horizontal,
        children: vec![
            SerializedLayout::Leaf {
                pane_id: 1,
                cwd: Some(PathBuf::from("/a")),
                file: None,
            },
            SerializedLayout::Leaf {
                pane_id: 2,
                cwd: Some(PathBuf::from("/b")),
                file: None,
            },
        ],
        ratios: vec![0.5, 0.5],
    };

    let json = serde_json::to_string(&layout).unwrap();
    let restored: SerializedLayout = serde_json::from_str(&json).unwrap();
    match restored {
        SerializedLayout::Split {
            children, ratios, ..
        } => {
            assert_eq!(children.len(), 2);
            assert_eq!(ratios.len(), 2);
        }
        _ => panic!("expected Split"),
    }
}

#[test]
fn file_leaf_round_trip_preserves_viewer_state() {
    // File panes carry their viewer state through serialization so a
    // restart restores the exact tab the user had open.
    let leaf = SerializedLayout::Leaf {
        pane_id: 7,
        cwd: None,
        file: Some(SerializedFileContent {
            worktree_id: 1,
            path: PathBuf::from("src/main.rs"),
            staged: false,
            view_mode: SerializedFileViewMode::Raw,
        }),
    };

    let json = serde_json::to_string(&leaf).unwrap();
    let restored: SerializedLayout = serde_json::from_str(&json).unwrap();
    match restored {
        SerializedLayout::Leaf { file: Some(fc), .. } => {
            assert_eq!(fc.path, PathBuf::from("src/main.rs"));
            assert!(!fc.staged);
            assert_eq!(fc.view_mode, SerializedFileViewMode::Raw);
        }
        _ => panic!("expected Leaf with file content"),
    }
}

#[test]
fn legacy_leaf_without_file_field_loads_as_terminal() {
    // Forward-compat: state files written before File panes existed
    // omit the `file` key entirely — they must deserialize as
    // Terminal panes (file = None) without errors.
    let legacy_json = r#"{"type":"Leaf","pane_id":1,"cwd":"/some/dir"}"#;
    let restored: SerializedLayout = serde_json::from_str(legacy_json).unwrap();
    match restored {
        SerializedLayout::Leaf { cwd, file, .. } => {
            assert_eq!(cwd, Some(PathBuf::from("/some/dir")));
            assert!(file.is_none(), "missing `file` field defaults to None");
        }
        _ => panic!("expected Leaf"),
    }
}

#[test]
fn file_leaf_skips_serialization_when_terminal() {
    // Terminal leaves (`file: None`) emit no `file` key in JSON so
    // saved state stays small for the common case.
    let leaf = SerializedLayout::Leaf {
        pane_id: 1,
        cwd: Some(PathBuf::from("/tmp")),
        file: None,
    };
    let json = serde_json::to_string(&leaf).unwrap();
    assert!(
        !json.contains("\"file\""),
        "terminal leaves must not write a `file` key, got: {json}"
    );
}

#[test]
fn recent_list_round_trip() {
    let entries = vec![RecentEntry::now("/tmp/a"), RecentEntry::now("/tmp/b")];
    let json = serde_json::to_string(&entries).unwrap();
    let restored: Vec<RecentEntry> = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.len(), 2);
    assert_eq!(restored[0].name, "a");
}

#[test]
fn dock_states_default_is_all_closed() {
    let d = DockStates::default();
    assert!(!d.left_open);
    assert!(!d.bottom_open);
    assert!(!d.right_open);
}

#[test]
fn window_state_default_is_zero() {
    let w = WindowState::default();
    assert_eq!(w.x, 0.0);
    assert_eq!(w.width, 0.0);
}

// ---- Persistence (file I/O) ----

#[test]
fn save_and_load_state_round_trip() {
    let dir = std::env::temp_dir().join("daruda_test_project_state");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let data_dir = dir.join("data");
    let root = dir.join("myproject");
    std::fs::create_dir_all(&root).unwrap();

    let state = ProjectState {
        root: root.clone(),
        worktrees: Vec::new(),
        active_worktree_id: 0,
        active_dock_view: LeftDockView::default(),
        active_right_panel_view: RightDockView::default(),
        active_usage_window: UsageWindow::default(),
        tabs: vec![],
        active_tab_index: 0,
        focused_pane_id: 0,
        docks: DockStates::default(),
        window: WindowState::default(),
        window_user_label: None,
        font_size: 13.0,
        vertical_spacing: 1.0,
        horizontal_spacing: 1.0,
    };

    persistence::save_state_in(&data_dir, &state).unwrap();
    let loaded = persistence::load_state_in(&data_dir, &root).unwrap();
    assert_eq!(loaded.root, root);
    assert_eq!(loaded.font_size, 13.0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_state_missing_returns_none() {
    let result = persistence::load_state(std::path::Path::new("/nonexistent/path"));
    assert!(result.is_none());
}

#[test]
fn save_and_load_recent_round_trip() {
    let entries = vec![
        RecentEntry::now("/tmp/test_a"),
        RecentEntry::now("/tmp/test_b"),
    ];
    let json = serde_json::to_string_pretty(&entries).unwrap();
    let restored: Vec<RecentEntry> = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.len(), 2);
}

#[test]
fn recent_max_is_reasonable() {
    const _: () = assert!(RECENT_MAX >= 5);
    const _: () = assert!(RECENT_MAX <= 100);
}

#[test]
fn window_state_is_valid_checks_dimensions() {
    let zero = WindowState::default();
    assert!(!zero.is_valid());
    let valid = WindowState {
        x: 0.0,
        y: 0.0,
        width: 1200.0,
        height: 800.0,
    };
    assert!(valid.is_valid());
}

#[test]
fn split_direction_serde_round_trip() {
    let h = SplitDirectionSerde::Horizontal;
    let v = SplitDirectionSerde::Vertical;
    let h_json = serde_json::to_string(&h).unwrap();
    let v_json = serde_json::to_string(&v).unwrap();
    assert_eq!(h_json, "\"horizontal\"");
    assert_eq!(v_json, "\"vertical\"");
    let h2: SplitDirectionSerde = serde_json::from_str(&h_json).unwrap();
    assert_eq!(h2, SplitDirectionSerde::Horizontal);
}

#[test]
fn fnv_hash_is_16_chars() {
    let h = path_hash(std::path::Path::new("/Users/test/project"));
    assert_eq!(h.len(), 16);
    assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn fnv_hash_is_deterministic() {
    let a = path_hash(std::path::Path::new("/a/b/c"));
    let b = path_hash(std::path::Path::new("/a/b/c"));
    assert_eq!(a, b);
}

// ---- Worktree data model ----

#[test]
fn worktree_kind_default_round_trip() {
    let k = WorktreeKind::Default;
    let json = serde_json::to_string(&k).unwrap();
    let back: WorktreeKind = serde_json::from_str(&json).unwrap();
    assert_eq!(back, WorktreeKind::Default);
    assert!(!back.is_git());
}

#[test]
fn worktree_kind_git_round_trip() {
    let k = WorktreeKind::Git {
        branch: Some("main".into()),
        repo_root: PathBuf::from("/tmp/repo"),
        worktree_root: PathBuf::from("/tmp/repo"),
    };
    let json = serde_json::to_string(&k).unwrap();
    let back: WorktreeKind = serde_json::from_str(&json).unwrap();
    assert_eq!(back, k);
    assert!(back.is_git());
}

#[test]
fn worktree_kind_git_detached_head() {
    let k = WorktreeKind::Git {
        branch: None,
        repo_root: PathBuf::from("/tmp/repo"),
        worktree_root: PathBuf::from("/tmp/repo"),
    };
    let json = serde_json::to_string(&k).unwrap();
    let back: WorktreeKind = serde_json::from_str(&json).unwrap();
    assert_eq!(back, k);
}

#[test]
fn serialized_worktree_default_for_path() {
    let w = SerializedWorktree::default_for_path(0, PathBuf::from("/tmp/plain"));
    assert_eq!(w.id, 0);
    assert_eq!(w.kind, WorktreeKind::Default);
    assert!(w.tabs.is_empty());
    assert!(!w.is_unread);
}

#[test]
fn serialized_worktree_display_name_prefers_user_name() {
    let mut w = SerializedWorktree::default_for_path(0, PathBuf::from("/tmp/scratch"));
    w.name = Some("My Scratch".into());
    assert_eq!(w.display_name(), "My Scratch");
}

#[test]
fn serialized_worktree_display_name_uses_branch_for_git() {
    let w = SerializedWorktree {
        id: 1,
        kind: WorktreeKind::Git {
            branch: Some("feat/sidebar".into()),
            repo_root: PathBuf::from("/tmp/repo"),
            worktree_root: PathBuf::from("/tmp/repo"),
        },
        path: PathBuf::from("/tmp/repo"),
        name: None,
        tab_order: 0,
        is_unread: false,
        last_activity: 0,
        tabs: Vec::new(),
        active_tab_index: 0,
        base_ref: None,
        description: None,
    };
    assert_eq!(w.display_name(), "feat/sidebar");
}

#[test]
fn serialized_worktree_display_name_detached_head() {
    let w = SerializedWorktree {
        id: 1,
        kind: WorktreeKind::Git {
            branch: None,
            repo_root: PathBuf::from("/tmp/repo"),
            worktree_root: PathBuf::from("/tmp/repo"),
        },
        path: PathBuf::from("/tmp/repo"),
        name: None,
        tab_order: 0,
        is_unread: false,
        last_activity: 0,
        tabs: Vec::new(),
        active_tab_index: 0,
        base_ref: None,
        description: None,
    };
    assert_eq!(w.display_name(), "(detached)");
}

#[test]
fn serialized_worktree_display_name_uses_basename_for_default() {
    let w = SerializedWorktree::default_for_path(0, PathBuf::from("/Users/alice/scratch"));
    assert_eq!(w.display_name(), "scratch");
}

#[test]
fn serialized_worktree_loads_legacy_json_with_old_field_names() {
    // JSON saved before the label→name / task→description rename.
    // serde aliases must accept the old keys so existing state files
    // load without migration.
    let json = r#"{
        "id": 0,
        "kind": { "type": "default" },
        "path": "/tmp/legacy",
        "label": null,
        "tab_order": 0,
        "is_unread": false,
        "last_activity": 0,
        "tabs": [],
        "active_tab_index": 0
    }"#;
    let w: SerializedWorktree = serde_json::from_str(json).unwrap();
    assert!(w.base_ref.is_none());
    assert!(w.description.is_none());
}

#[test]
fn serialized_worktree_round_trips_base_ref_and_description() {
    let mut w = SerializedWorktree::default_for_path(0, PathBuf::from("/tmp/scratch"));
    w.base_ref = Some("origin/main".into());
    w.description = Some("PR #123 review".into());
    let json = serde_json::to_string(&w).unwrap();
    let back: SerializedWorktree = serde_json::from_str(&json).unwrap();
    assert_eq!(back.base_ref.as_deref(), Some("origin/main"));
    assert_eq!(back.description.as_deref(), Some("PR #123 review"));
}

#[test]
fn worktree_status_defaults_to_idle() {
    assert_eq!(WorktreeStatus::default(), WorktreeStatus::Idle);
}

#[test]
fn dock_view_default_is_worktrees() {
    assert_eq!(LeftDockView::default(), LeftDockView::Worktrees);
}

#[test]
fn dock_view_round_trips_as_snake_case() {
    for (v, expect) in [
        (LeftDockView::Worktrees, "\"worktrees\""),
        (LeftDockView::GitChanges, "\"git_changes\""),
        (LeftDockView::Files, "\"files\""),
    ] {
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, expect);
        let back: LeftDockView = serde_json::from_str(&json).unwrap();
        assert_eq!(back, v);
    }
}

#[test]
fn project_state_persists_active_dock_view() {
    let worktree = SerializedWorktree::default_for_path(0, PathBuf::from("/tmp"));
    let state = ProjectState {
        root: PathBuf::from("/tmp"),
        worktrees: vec![worktree],
        active_worktree_id: 0,
        active_dock_view: LeftDockView::Files,
        active_right_panel_view: RightDockView::default(),
        active_usage_window: UsageWindow::default(),
        tabs: Vec::new(),
        active_tab_index: 0,
        focused_pane_id: 0,
        docks: DockStates::default(),
        window: WindowState::default(),
        window_user_label: None,
        font_size: 13.0,
        vertical_spacing: 1.0,
        horizontal_spacing: 1.0,
    };
    let json = serde_json::to_string(&state).unwrap();
    let back: ProjectState = serde_json::from_str(&json).unwrap();
    assert_eq!(back.active_dock_view, LeftDockView::Files);
}

#[test]
fn legacy_json_without_active_dock_view_defaults_to_worktrees() {
    // Files written before the LeftDockView field existed must load
    // without error and default to Worktrees.
    let legacy_json = r#"{
        "root": "/tmp/legacy",
        "worktrees": [],
        "active_worktree_id": 0,
        "focused_pane_id": 0,
        "docks": {
            "left_open": false, "left_size": 0.0,
            "bottom_open": false, "bottom_size": 0.0,
            "right_open": false, "right_size": 0.0
        },
        "window": { "x": 0.0, "y": 0.0, "width": 0.0, "height": 0.0 },
        "font_size": 13.0,
        "vertical_spacing": 1.0,
        "horizontal_spacing": 1.0
    }"#;
    let state: ProjectState = serde_json::from_str(legacy_json).unwrap();
    assert_eq!(state.active_dock_view, LeftDockView::Worktrees);
}

#[test]
fn right_panel_view_default_is_usage() {
    assert_eq!(RightDockView::default(), RightDockView::Usage);
}

#[test]
fn right_panel_view_round_trips_as_snake_case() {
    for (v, expect) in [
        (RightDockView::Usage, "\"usage\""),
        (RightDockView::Skills, "\"skills\""),
        (RightDockView::Tools, "\"tools\""),
        (RightDockView::Tasks, "\"tasks\""),
    ] {
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, expect);
        let back: RightDockView = serde_json::from_str(&json).unwrap();
        assert_eq!(back, v);
    }
}

#[test]
fn project_state_persists_active_right_panel_view() {
    let worktree = SerializedWorktree::default_for_path(0, PathBuf::from("/tmp"));
    let state = ProjectState {
        root: PathBuf::from("/tmp"),
        worktrees: vec![worktree],
        active_worktree_id: 0,
        active_dock_view: LeftDockView::default(),
        active_right_panel_view: RightDockView::Tools,
        active_usage_window: UsageWindow::default(),
        tabs: Vec::new(),
        active_tab_index: 0,
        focused_pane_id: 0,
        docks: DockStates::default(),
        window: WindowState::default(),
        window_user_label: None,
        font_size: 13.0,
        vertical_spacing: 1.0,
        horizontal_spacing: 1.0,
    };
    let json = serde_json::to_string(&state).unwrap();
    let back: ProjectState = serde_json::from_str(&json).unwrap();
    assert_eq!(back.active_right_panel_view, RightDockView::Tools);
}

#[test]
fn usage_window_default_is_last_7d() {
    assert_eq!(UsageWindow::default(), UsageWindow::Last7d);
}

#[test]
fn usage_window_round_trips_as_snake_case() {
    for (v, expect) in [
        (UsageWindow::All, "\"all\""),
        (UsageWindow::Last5h, "\"last_5h\""),
        (UsageWindow::Last24h, "\"last_24h\""),
        (UsageWindow::Last7d, "\"last_7d\""),
    ] {
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, expect);
        let back: UsageWindow = serde_json::from_str(&json).unwrap();
        assert_eq!(back, v);
    }
}

#[test]
fn usage_window_slug_matches_serde_value() {
    for variant in UsageWindow::ALL {
        let slug = variant.slug();
        let from_slug = UsageWindow::from_slug(slug).unwrap();
        assert_eq!(from_slug, *variant);
        let json = serde_json::to_string(variant).unwrap();
        assert_eq!(json, format!("\"{slug}\""));
    }
}

#[test]
fn usage_window_from_slug_returns_none_for_unknown() {
    assert!(UsageWindow::from_slug("definitely-not-a-window").is_none());
}

#[test]
fn usage_window_duration_is_some_for_bounded_variants() {
    use std::time::Duration;
    assert_eq!(UsageWindow::All.duration(), None);
    assert_eq!(
        UsageWindow::Last5h.duration(),
        Some(Duration::from_secs(5 * 3600))
    );
    assert_eq!(
        UsageWindow::Last7d.duration(),
        Some(Duration::from_secs(7 * 24 * 3600))
    );
}

#[test]
fn usage_window_cutoff_subtracts_duration_from_now() {
    use std::time::{Duration, SystemTime};
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    assert_eq!(UsageWindow::All.cutoff(now), None);
    assert_eq!(
        UsageWindow::Last5h.cutoff(now),
        Some(now - Duration::from_secs(5 * 3600))
    );
}

#[test]
fn project_state_persists_active_usage_window() {
    let worktree = SerializedWorktree::default_for_path(0, PathBuf::from("/tmp"));
    let state = ProjectState {
        root: PathBuf::from("/tmp"),
        worktrees: vec![worktree],
        active_worktree_id: 0,
        active_dock_view: LeftDockView::default(),
        active_right_panel_view: RightDockView::default(),
        active_usage_window: UsageWindow::Last24h,
        tabs: Vec::new(),
        active_tab_index: 0,
        focused_pane_id: 0,
        docks: DockStates::default(),
        window: WindowState::default(),
        window_user_label: None,
        font_size: 13.0,
        vertical_spacing: 1.0,
        horizontal_spacing: 1.0,
    };
    let json = serde_json::to_string(&state).unwrap();
    let back: ProjectState = serde_json::from_str(&json).unwrap();
    assert_eq!(back.active_usage_window, UsageWindow::Last24h);
}

#[test]
fn legacy_json_without_active_usage_window_defaults_to_last_7d() {
    let legacy_json = r#"{
        "root": "/tmp/legacy",
        "worktrees": [],
        "active_worktree_id": 0,
        "focused_pane_id": 0,
        "docks": {
            "left_open": false, "left_size": 0.0,
            "bottom_open": false, "bottom_size": 0.0,
            "right_open": false, "right_size": 0.0
        },
        "window": { "x": 0.0, "y": 0.0, "width": 0.0, "height": 0.0 },
        "font_size": 13.0,
        "vertical_spacing": 1.0,
        "horizontal_spacing": 1.0
    }"#;
    let state: ProjectState = serde_json::from_str(legacy_json).unwrap();
    assert_eq!(state.active_usage_window, UsageWindow::Last7d);
}

#[test]
fn legacy_json_without_active_right_panel_view_defaults_to_usage() {
    // Files written before the RightDockView field existed must load
    // without error and default to Usage.
    let legacy_json = r#"{
        "root": "/tmp/legacy",
        "worktrees": [],
        "active_worktree_id": 0,
        "focused_pane_id": 0,
        "docks": {
            "left_open": false, "left_size": 0.0,
            "bottom_open": false, "bottom_size": 0.0,
            "right_open": false, "right_size": 0.0
        },
        "window": { "x": 0.0, "y": 0.0, "width": 0.0, "height": 0.0 },
        "font_size": 13.0,
        "vertical_spacing": 1.0,
        "horizontal_spacing": 1.0
    }"#;
    let state: ProjectState = serde_json::from_str(legacy_json).unwrap();
    assert_eq!(state.active_right_panel_view, RightDockView::Usage);
}

// ---- ProjectState migration ----

fn sample_tab(pane_id: u64) -> SerializedTab {
    SerializedTab {
        layout: SerializedLayout::Leaf {
            pane_id,
            cwd: Some(PathBuf::from("/tmp/legacy")),
            file: None,
        },
        last_focused_pane: pane_id,
        user_label: None,
    }
}

fn legacy_state_with_tabs(tabs: Vec<SerializedTab>) -> ProjectState {
    ProjectState {
        root: PathBuf::from("/tmp/legacy"),
        worktrees: Vec::new(),
        active_worktree_id: 0,
        active_dock_view: LeftDockView::default(),
        active_right_panel_view: RightDockView::default(),
        active_usage_window: UsageWindow::default(),
        tabs,
        active_tab_index: 0,
        focused_pane_id: 1,
        docks: DockStates::default(),
        window: WindowState::default(),
        window_user_label: None,
        font_size: 13.0,
        vertical_spacing: 1.0,
        horizontal_spacing: 1.0,
    }
}

#[test]
fn serialized_tab_user_label_round_trip() {
    let tab = SerializedTab {
        layout: SerializedLayout::Leaf {
            pane_id: 7,
            cwd: None,
            file: None,
        },
        last_focused_pane: 7,
        user_label: Some("PR #123 review".into()),
    };
    let json = serde_json::to_string(&tab).unwrap();
    assert!(json.contains("PR #123 review"));
    let decoded: SerializedTab = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.user_label.as_deref(), Some("PR #123 review"));
}

#[test]
fn serialized_tab_legacy_without_user_label_loads_as_none() {
    // Old state files predate the user_label field. They must decode
    // cleanly with `user_label = None` thanks to `#[serde(default)]`.
    let legacy = r#"{
        "layout": { "type": "Leaf", "pane_id": 1, "cwd": null },
        "last_focused_pane": 1
    }"#;
    let decoded: SerializedTab = serde_json::from_str(legacy).unwrap();
    assert!(decoded.user_label.is_none());
}

#[test]
fn serialized_tab_user_label_none_is_skipped_in_json() {
    // skip_serializing_if keeps the field out when it's None so old
    // readers (and the test fixture above) round-trip without churn.
    let tab = SerializedTab {
        layout: SerializedLayout::Leaf {
            pane_id: 1,
            cwd: None,
            file: None,
        },
        last_focused_pane: 1,
        user_label: None,
    };
    let json = serde_json::to_string(&tab).unwrap();
    assert!(!json.contains("user_label"));
}

#[test]
fn project_state_window_user_label_round_trip() {
    let mut state = legacy_state_with_tabs(vec![sample_tab(1)]);
    state.window_user_label = Some("daruda — review".into());
    let json = serde_json::to_string(&state).unwrap();
    assert!(json.contains("daruda — review"));
    let decoded: ProjectState = serde_json::from_str(&json).unwrap();
    assert_eq!(
        decoded.window_user_label.as_deref(),
        Some("daruda — review")
    );
}

#[test]
fn project_state_legacy_without_window_user_label_loads_as_none() {
    // Old state files predate window_user_label. Must decode cleanly
    // with `window_user_label = None` thanks to `#[serde(default)]`.
    let legacy = r#"{
        "root": "/tmp/legacy",
        "focused_pane_id": 0,
        "docks": {
            "left_open": false,
            "left_size": 0.0,
            "bottom_open": false,
            "bottom_size": 0.0,
            "right_open": false,
            "right_size": 0.0
        },
        "window": { "x": 0.0, "y": 0.0, "width": 0.0, "height": 0.0 },
        "font_size": 14.0,
        "vertical_spacing": 1.0,
        "horizontal_spacing": 1.0
    }"#;
    let decoded: ProjectState = serde_json::from_str(legacy).unwrap();
    assert!(decoded.window_user_label.is_none());
}

#[test]
fn project_state_window_user_label_none_is_skipped_in_json() {
    let state = legacy_state_with_tabs(vec![sample_tab(1)]);
    assert!(state.window_user_label.is_none());
    let json = serde_json::to_string(&state).unwrap();
    assert!(!json.contains("window_user_label"));
}

#[test]
fn migrate_legacy_wraps_tabs_in_default_worktree() {
    let mut state = legacy_state_with_tabs(vec![sample_tab(1), sample_tab(2)]);
    state.active_tab_index = 1;
    state.migrate_legacy();

    assert_eq!(state.worktrees.len(), 1);
    assert!(state.tabs.is_empty());
    assert_eq!(state.active_tab_index, 0); // moved into worktree

    let wt = &state.worktrees[0];
    assert_eq!(wt.id, 0);
    assert_eq!(wt.kind, WorktreeKind::Default);
    assert_eq!(wt.path, state.root);
    assert_eq!(wt.tabs.len(), 2);
    assert_eq!(wt.active_tab_index, 1);
    assert_eq!(state.active_worktree_id, wt.id);
}

#[test]
fn migrate_legacy_is_idempotent() {
    let mut state = legacy_state_with_tabs(vec![sample_tab(1)]);
    state.migrate_legacy();
    let snapshot = serde_json::to_string(&state).unwrap();
    state.migrate_legacy();
    let after = serde_json::to_string(&state).unwrap();
    assert_eq!(snapshot, after);
    assert_eq!(state.worktrees.len(), 1);
    assert_eq!(state.worktrees[0].tabs.len(), 1);
}

#[test]
fn migrate_legacy_noop_when_worktrees_already_present() {
    let pre_existing = SerializedWorktree {
        id: 7,
        kind: WorktreeKind::Git {
            branch: Some("main".into()),
            repo_root: PathBuf::from("/tmp/repo"),
            worktree_root: PathBuf::from("/tmp/repo"),
        },
        path: PathBuf::from("/tmp/repo"),
        name: None,
        tab_order: 0,
        is_unread: false,
        last_activity: 0,
        tabs: vec![sample_tab(9)],
        active_tab_index: 0,
        base_ref: None,
        description: None,
    };
    let mut state = ProjectState {
        root: PathBuf::from("/tmp/repo"),
        worktrees: vec![pre_existing],
        active_worktree_id: 7,
        active_dock_view: LeftDockView::default(),
        active_right_panel_view: RightDockView::default(),
        active_usage_window: UsageWindow::default(),
        tabs: vec![sample_tab(1)], // legacy noise — should not migrate
        active_tab_index: 0,
        focused_pane_id: 0,
        docks: DockStates::default(),
        window: WindowState::default(),
        window_user_label: None,
        font_size: 13.0,
        vertical_spacing: 1.0,
        horizontal_spacing: 1.0,
    };
    state.migrate_legacy();
    assert_eq!(state.worktrees.len(), 1);
    assert_eq!(state.worktrees[0].id, 7);
    // Legacy tabs are left untouched — caller (save) drops them via
    // skip_serializing_if, so no data is written on next save.
    assert_eq!(state.tabs.len(), 1);
}

#[test]
fn migrate_legacy_noop_when_both_empty() {
    let mut state = legacy_state_with_tabs(Vec::new());
    state.migrate_legacy();
    assert!(state.worktrees.is_empty());
    assert!(state.tabs.is_empty());
}

#[test]
fn legacy_json_without_worktrees_field_deserializes() {
    // Simulate a file written by an older build (no `worktrees` field).
    let legacy_json = r#"{
        "root": "/tmp/legacy",
        "tabs": [
            {
                "layout": { "type": "Leaf", "pane_id": 1, "cwd": "/tmp/legacy" },
                "last_focused_pane": 1
            }
        ],
        "active_tab_index": 0,
        "focused_pane_id": 1,
        "docks": {
            "left_open": false, "left_size": 0.0,
            "bottom_open": false, "bottom_size": 0.0,
            "right_open": false, "right_size": 0.0
        },
        "window": { "x": 0.0, "y": 0.0, "width": 0.0, "height": 0.0 },
        "font_size": 13.0,
        "vertical_spacing": 1.0,
        "horizontal_spacing": 1.0
    }"#;
    let mut state: ProjectState = serde_json::from_str(legacy_json).unwrap();
    assert!(state.worktrees.is_empty());
    assert_eq!(state.tabs.len(), 1);
    state.migrate_legacy();
    assert_eq!(state.worktrees.len(), 1);
    assert_eq!(state.worktrees[0].tabs.len(), 1);
    assert!(state.tabs.is_empty());
}

#[test]
fn new_state_roundtrip_omits_legacy_tabs_field() {
    // After migration (or when built directly with worktrees), saving
    // must not write the empty top-level `tabs` array — otherwise
    // old builds might read junk and newer builds accumulate noise.
    let mut state = legacy_state_with_tabs(vec![sample_tab(1)]);
    state.migrate_legacy();
    let json = serde_json::to_string(&state).unwrap();
    assert!(
        !json.contains("\"tabs\":[]"),
        "expected empty top-level tabs to be omitted, got: {json}"
    );
    assert!(json.contains("\"worktrees\""));
}

#[test]
fn save_then_load_applies_migration_for_legacy_file() {
    let dir = std::env::temp_dir().join("daruda_test_migrate_legacy");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let data_dir = dir.join("data");
    let root = dir.join("legacy_project");
    std::fs::create_dir_all(&root).unwrap();

    // Hand-write a legacy-format JSON into the location load_state_in will read from.
    let projects_dir = data_dir.join("projects");
    std::fs::create_dir_all(&projects_dir).unwrap();
    let hash = path_hash(&root);
    let legacy_path = projects_dir.join(format!("{hash}.json"));
    let legacy_json = format!(
        r#"{{
            "root": "{}",
            "tabs": [
                {{
                    "layout": {{ "type": "Leaf", "pane_id": 1, "cwd": "{}" }},
                    "last_focused_pane": 1
                }}
            ],
            "active_tab_index": 0,
            "focused_pane_id": 1,
            "docks": {{
                "left_open": false, "left_size": 0.0,
                "bottom_open": false, "bottom_size": 0.0,
                "right_open": false, "right_size": 0.0
            }},
            "window": {{ "x": 0.0, "y": 0.0, "width": 0.0, "height": 0.0 }},
            "font_size": 13.0,
            "vertical_spacing": 1.0,
            "horizontal_spacing": 1.0
        }}"#,
        root.display(),
        root.display(),
    );
    std::fs::write(&legacy_path, legacy_json).unwrap();

    let loaded = persistence::load_state_in(&data_dir, &root).expect("load should succeed");
    assert_eq!(loaded.worktrees.len(), 1);
    assert_eq!(loaded.worktrees[0].tabs.len(), 1);
    assert!(loaded.tabs.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn touch_and_load_recent_integration() {
    let dir = std::env::temp_dir().join("daruda_test_touch_recent");
    let _ = std::fs::remove_dir_all(&dir);
    let data_dir = dir.join("data");
    let project = dir.join("project");
    std::fs::create_dir_all(&project).unwrap();

    persistence::touch_recent_in(&data_dir, &project).unwrap();
    let recent = persistence::load_recent_in(&data_dir);
    assert!(recent.iter().any(|e| e.root == project));

    let _ = std::fs::remove_dir_all(&dir);
}
