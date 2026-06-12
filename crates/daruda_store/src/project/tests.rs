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

// ---- Serialization round-trip (low-level layout types) ----

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
            lane_id: 1,
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

// ---- Lane data model ----

#[test]
fn worktree_kind_default_round_trip() {
    let k = LaneKind::Default;
    let json = serde_json::to_string(&k).unwrap();
    let back: LaneKind = serde_json::from_str(&json).unwrap();
    assert_eq!(back, LaneKind::Default);
    assert!(!back.is_git());
}

#[test]
fn worktree_kind_git_round_trip() {
    let k = LaneKind::Git {
        branch: Some("main".into()),
        repo_root: PathBuf::from("/tmp/repo"),
        worktree_root: PathBuf::from("/tmp/repo"),
    };
    let json = serde_json::to_string(&k).unwrap();
    let back: LaneKind = serde_json::from_str(&json).unwrap();
    assert_eq!(back, k);
    assert!(back.is_git());
}

#[test]
fn worktree_kind_git_detached_head() {
    let k = LaneKind::Git {
        branch: None,
        repo_root: PathBuf::from("/tmp/repo"),
        worktree_root: PathBuf::from("/tmp/repo"),
    };
    let json = serde_json::to_string(&k).unwrap();
    let back: LaneKind = serde_json::from_str(&json).unwrap();
    assert_eq!(back, k);
}

#[test]
fn serialized_worktree_default_for_path() {
    let w = SerializedLane::default_for_path(0, PathBuf::from("/tmp/plain"));
    assert_eq!(w.id, 0);
    assert_eq!(w.kind, LaneKind::Default);
    assert!(w.tabs.is_empty());
    assert!(!w.is_unread);
}

#[test]
fn serialized_worktree_display_name_prefers_user_name() {
    let mut w = SerializedLane::default_for_path(0, PathBuf::from("/tmp/scratch"));
    w.name = Some("My Scratch".into());
    assert_eq!(w.display_name(), "My Scratch");
}

#[test]
fn serialized_worktree_display_name_uses_branch_for_git() {
    let w = SerializedLane {
        id: 1,
        kind: LaneKind::Git {
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
    let w = SerializedLane {
        id: 1,
        kind: LaneKind::Git {
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
    let w = SerializedLane::default_for_path(0, PathBuf::from("/Users/alice/scratch"));
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
    let w: SerializedLane = serde_json::from_str(json).unwrap();
    assert!(w.base_ref.is_none());
    assert!(w.description.is_none());
}

#[test]
fn serialized_worktree_round_trips_base_ref_and_description() {
    let mut w = SerializedLane::default_for_path(0, PathBuf::from("/tmp/scratch"));
    w.base_ref = Some("origin/main".into());
    w.description = Some("PR #123 review".into());
    let json = serde_json::to_string(&w).unwrap();
    let back: SerializedLane = serde_json::from_str(&json).unwrap();
    assert_eq!(back.base_ref.as_deref(), Some("origin/main"));
    assert_eq!(back.description.as_deref(), Some("PR #123 review"));
}

// ---- Dock / right-panel / usage view enums ----

#[test]
fn dock_view_round_trips_as_snake_case() {
    for (v, expect) in [
        (LeftDockView::Lanes, "\"worktrees\""),
        (LeftDockView::GitChanges, "\"git_changes\""),
        (LeftDockView::Files, "\"files\""),
    ] {
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, expect);
        let back: LeftDockView = serde_json::from_str(&json).unwrap();
        assert_eq!(back, v);
    }
    let legacy: LeftDockView = serde_json::from_str("\"lanes\"").unwrap();
    assert_eq!(legacy, LeftDockView::Lanes);
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

// ---- SerializedTab (low-level) round-trips ----

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

// ---- LaneRef / WindowOpenPolicy ----

#[test]
fn window_open_policy_default_is_ask() {
    assert_eq!(WindowOpenPolicy::default(), WindowOpenPolicy::Ask);
    let json = serde_json::to_string(&WindowOpenPolicy::AddHere).unwrap();
    assert_eq!(json, "\"add_here\"");
    let json = serde_json::to_string(&WindowOpenPolicy::NewWindow).unwrap();
    assert_eq!(json, "\"new_window\"");
}

// ---- New UUID-keyed schema round-trip tests ----

#[cfg(test)]
mod new_schema_fixtures {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use crate::project::{
        DockStates, LeftDockView, ProjectOverride, ProjectState, ProjectUuid, RightDockView,
        WORKSPACE_SCHEMA_VERSION, WindowState, WorkspaceState, WorkspaceUuid,
    };

    pub(super) fn sample_project() -> ProjectState {
        ProjectState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: ProjectUuid::new(),
            root: PathBuf::from("/Users/test/repo"),
            name: Some("repo".into()),
            lanes: vec![],
            last_active_lane_id: Default::default(),
            next_lane_id: Default::default(),
            default_branch: Some("main".into()),
            base_branch: Some("develop".into()),
        }
    }

    pub(super) fn sample_workspace(project: ProjectUuid) -> WorkspaceState {
        WorkspaceState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: WorkspaceUuid::new(),
            project_ids: vec![project],
            project_overrides: BTreeMap::from([(project, ProjectOverride::default())]),
            groups: vec![],
            active_project: Some(project),
            active_lane: Some(Default::default()),
            docks: DockStates::default(),
            window: WindowState {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            },
            font_size: 13.0,
            vertical_spacing: 1.0,
            horizontal_spacing: 1.0,
            focused_pane_id: Default::default(),
            active_dock_view: LeftDockView::default(),
            active_right_panel_view: RightDockView::default(),
            window_open_policy: Default::default(),
            next_group_id: Default::default(),
            project_tabs: BTreeMap::new(),
        }
    }
}

mod new_schema {
    use super::new_schema_fixtures::{sample_project, sample_workspace};
    use crate::project::{
        ProjectOverride, ProjectState, ProjectUuid, RecentEntry, WorkspaceState, WorkspaceUuid,
    };

    #[test]
    fn project_state_roundtrip() {
        let p = sample_project();
        let json = serde_json::to_string(&p).unwrap();
        let back: ProjectState = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn project_state_branch_fields_roundtrip() {
        // `default_branch` / `base_branch` survive a serialize →
        // deserialize cycle with their values intact.
        let mut p = sample_project();
        p.default_branch = Some("trunk".into());
        p.base_branch = Some("release".into());
        let json = serde_json::to_string(&p).unwrap();
        let back: ProjectState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.default_branch.as_deref(), Some("trunk"));
        assert_eq!(back.base_branch.as_deref(), Some("release"));
    }

    #[test]
    fn legacy_project_state_without_branch_fields_defaults_to_none() {
        // A pre-Task-2 state file has neither `default_branch` nor
        // `base_branch`. `#[serde(default)]` must deserialize both to
        // `None` so legacy files load without a schema bump.
        let json = r#"{
            "schema_version": 3,
            "uuid": "00000000-0000-0000-0000-000000000000",
            "root": "/Users/test/repo",
            "name": "repo",
            "worktrees": [],
            "last_active_worktree_id": 0,
            "next_worktree_id": 0
        }"#;
        let back: ProjectState = serde_json::from_str(json).unwrap();
        assert_eq!(back.default_branch, None);
        assert_eq!(back.base_branch, None);
    }

    #[test]
    fn workspace_state_roundtrip() {
        let p = ProjectUuid::new();
        let w = sample_workspace(p);
        let json = serde_json::to_string(&w).unwrap();
        let back: WorkspaceState = serde_json::from_str(&json).unwrap();
        assert_eq!(w, back);
    }

    #[test]
    fn recent_entry_roundtrip() {
        let e = RecentEntry::now(WorkspaceUuid::new(), "io.whatap".into());
        let json = serde_json::to_string(&e).unwrap();
        let back: RecentEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn workspace_with_same_project_uuid_keeps_independent_overrides() {
        // Policy B: same ProjectUuid in two workspaces, each with its
        // own ProjectOverride. Mutation of one workspace's override
        // must not affect the other workspace's view.
        let proj = ProjectUuid::new();
        let mut ws_a = sample_workspace(proj);
        let mut ws_b = sample_workspace(proj);

        ws_a.project_overrides.insert(
            proj,
            ProjectOverride {
                color: Some("#f87171".into()),
                tab_order: 0,
                group_id: Some(Default::default()),
                is_collapsed: false,
            },
        );
        ws_b.project_overrides.insert(
            proj,
            ProjectOverride {
                color: Some("#60a5fa".into()),
                tab_order: 2,
                group_id: None,
                is_collapsed: true,
            },
        );

        assert_ne!(ws_a.project_overrides[&proj], ws_b.project_overrides[&proj]);
    }
}

mod new_schema_persistence {
    use super::new_schema_fixtures::{sample_project, sample_workspace};
    use crate::project::persistence::{
        RECENT_MAX, for_each_project_state_in, is_uuid_filename_stem, load_project_state_in,
        load_recent_in, load_workspace_state_in, projects_dir_in, save_project_state_in,
        save_workspace_state_in, touch_recent_in,
    };
    use crate::project::{ProjectUuid, WorkspaceUuid};

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn workspace_state_disk_roundtrip() {
        let dir = tmp();
        let p = ProjectUuid::new();
        let w = sample_workspace(p);
        save_workspace_state_in(dir.path(), &w).unwrap();
        let back = load_workspace_state_in(dir.path(), w.uuid).unwrap();
        assert_eq!(w, back);
    }

    #[test]
    fn project_state_disk_roundtrip() {
        let dir = tmp();
        let p = sample_project();
        save_project_state_in(dir.path(), &p).unwrap();
        let back = load_project_state_in(dir.path(), p.uuid).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn recent_touch_inserts_and_dedupes() {
        let dir = tmp();
        let uuid = WorkspaceUuid::new();
        touch_recent_in(dir.path(), uuid, "first".into()).unwrap();
        touch_recent_in(dir.path(), uuid, "second".into()).unwrap();
        let r = load_recent_in(dir.path());
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].display_name, "second");
        assert_eq!(r[0].workspace_uuid, uuid);
    }

    #[test]
    fn recent_truncates_to_max() {
        let dir = tmp();
        for i in 0..(RECENT_MAX + 5) {
            touch_recent_in(dir.path(), WorkspaceUuid::new(), format!("ws-{i}")).unwrap();
        }
        assert_eq!(load_recent_in(dir.path()).len(), RECENT_MAX);
    }

    #[test]
    fn for_each_project_skips_legacy_hex_files() {
        let dir = tmp();
        std::fs::create_dir_all(projects_dir_in(dir.path())).unwrap();
        // legacy hex-hash file — must be skipped
        std::fs::write(
            projects_dir_in(dir.path()).join("a70eef01bcd37417.json"),
            r#"{"root":"/x","name":null,"worktrees":[]}"#,
        )
        .unwrap();
        // new UUID-keyed file — must be returned
        let p = sample_project();
        save_project_state_in(dir.path(), &p).unwrap();

        let mut seen = Vec::new();
        for_each_project_state_in(dir.path(), |s| seen.push(s.uuid));
        assert_eq!(seen, vec![p.uuid], "legacy hex-hash file must be skipped");
    }

    #[test]
    fn missing_recent_returns_empty_vec_and_does_not_read_legacy() {
        let dir = tmp();
        // legacy recent.json (different filename) — daruda must not read this
        std::fs::write(
            dir.path().join("recent.json"),
            r#"[{"root":"/tmp/old","name":"old","last_opened":0}]"#,
        )
        .unwrap();
        assert!(
            load_recent_in(dir.path()).is_empty(),
            "new code must ignore legacy recent.json"
        );
    }

    #[test]
    fn is_uuid_filename_stem_rejects_legacy_hex_hash() {
        assert!(!is_uuid_filename_stem("a70eef01bcd37417"));
        assert!(!is_uuid_filename_stem("ec6658194494cce5"));
        assert!(is_uuid_filename_stem(
            "550e8400-e29b-41d4-a716-446655440000"
        ));
        assert!(!is_uuid_filename_stem(
            "550E8400-E29B-41D4-A716-446655440000"
        )); // uppercase rejected
        assert!(!is_uuid_filename_stem(""));
    }

    #[test]
    fn two_workspaces_can_reference_same_project_uuid() {
        // N:N invariant — single ProjectState file on disk shared by
        // two WorkspaceState files.
        let dir = tmp();
        let proj = sample_project();
        save_project_state_in(dir.path(), &proj).unwrap();

        let mut ws_a = sample_workspace(proj.uuid);
        let mut ws_b = sample_workspace(proj.uuid);
        ws_a.uuid = WorkspaceUuid::new();
        ws_b.uuid = WorkspaceUuid::new();
        save_workspace_state_in(dir.path(), &ws_a).unwrap();
        save_workspace_state_in(dir.path(), &ws_b).unwrap();

        let loaded_a = load_workspace_state_in(dir.path(), ws_a.uuid).unwrap();
        let loaded_b = load_workspace_state_in(dir.path(), ws_b.uuid).unwrap();
        assert_eq!(loaded_a.project_ids, vec![proj.uuid]);
        assert_eq!(loaded_b.project_ids, vec![proj.uuid]);
    }
}

// ---- insta snapshots: persisted on-disk schema ----
//
// Lock the serde JSON shape of the persistence types most prone to
// silent drift — the `worktree`-renamed `LaneRef` field and the
// `type`-tagged `LaneKind`. A field rename or tag change surfaces as a
// snapshot diff; `cargo insta review` approves intentional changes.

#[test]
fn serialized_lane_git_json_snapshot() {
    let lane = SerializedLane {
        id: 2,
        kind: LaneKind::Git {
            branch: Some("feat/sidebar".into()),
            repo_root: PathBuf::from("/repo"),
            worktree_root: PathBuf::from("/repo-feat-sidebar"),
        },
        path: PathBuf::from("/repo-feat-sidebar"),
        name: Some("Sidebar".into()),
        tab_order: 1,
        is_unread: true,
        last_activity: 1_700_000_000,
        tabs: Vec::new(),
        active_tab_index: 0,
        base_ref: Some("main".into()),
        description: Some("PR #123 review".into()),
    };
    insta::assert_snapshot!(serde_json::to_string_pretty(&lane).unwrap());
}

#[test]
fn serialized_lane_default_json_snapshot() {
    let lane = SerializedLane::default_for_path(0, PathBuf::from("/plain/dir"));
    insta::assert_snapshot!(serde_json::to_string_pretty(&lane).unwrap());
}

#[test]
fn lane_ref_json_snapshot() {
    // The `lane` field serializes as `worktree` (back-compat alias).
    let r = LaneRef {
        project: 7,
        lane: 3,
    };
    insta::assert_snapshot!(serde_json::to_string_pretty(&r).unwrap());
}
