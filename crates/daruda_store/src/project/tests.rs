use std::path::{Path, PathBuf};

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
                content: SerializedPaneContent::Terminal {
                    cwd: Some(PathBuf::from("/a")),
                    account_id: None,
                },
            },
            SerializedLayout::Leaf {
                pane_id: 2,
                content: SerializedPaneContent::Terminal {
                    cwd: Some(PathBuf::from("/b")),
                    account_id: None,
                },
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
        content: SerializedPaneContent::File(SerializedFileContent {
            lane_id: 1,
            path: PathBuf::from("src/main.rs"),
            staged: false,
            view_mode: SerializedFileViewMode::Raw,
        }),
    };

    let json = serde_json::to_string(&leaf).unwrap();
    let restored: SerializedLayout = serde_json::from_str(&json).unwrap();
    match restored {
        SerializedLayout::Leaf {
            content: SerializedPaneContent::File(fc),
            ..
        } => {
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
        SerializedLayout::Leaf {
            content: SerializedPaneContent::Terminal { cwd, .. },
            ..
        } => assert_eq!(cwd, Some(PathBuf::from("/some/dir"))),
        other => panic!("a leaf naming no kind is a terminal, got {other:?}"),
    }
}

#[test]
fn file_leaf_skips_serialization_when_terminal() {
    // Terminal leaves (`file: None`) emit no `file` key in JSON so
    // saved state stays small for the common case.
    let leaf = SerializedLayout::Leaf {
        pane_id: 1,
        content: SerializedPaneContent::Terminal {
            cwd: Some(PathBuf::from("/tmp")),
            account_id: None,
        },
    };
    let json = serde_json::to_string(&leaf).unwrap();
    assert!(
        !json.contains("\"file\""),
        "terminal leaves must not write a `file` key, got: {json}"
    );
}

#[test]
fn terminal_leaf_round_trip_preserves_account_id() {
    // A terminal pane pinned to a managed account persists that override
    // through a save/restore cycle, and an unset override round-trips as
    // `None` without writing an `account_id` key at all.
    use crate::accounts::AccountId;
    let id = AccountId::new();
    let leaf = SerializedLayout::Leaf {
        pane_id: 3,
        content: SerializedPaneContent::Terminal {
            cwd: Some(PathBuf::from("/repo")),
            account_id: Some(id),
        },
    };
    let json = serde_json::to_string(&leaf).unwrap();
    let restored: SerializedLayout = serde_json::from_str(&json).unwrap();
    match restored {
        SerializedLayout::Leaf {
            content: SerializedPaneContent::Terminal { account_id, .. },
            ..
        } => assert_eq!(account_id, Some(id)),
        other => panic!("expected a terminal leaf, got {other:?}"),
    }

    let unset = SerializedLayout::Leaf {
        pane_id: 4,
        content: SerializedPaneContent::Terminal {
            cwd: Some(PathBuf::from("/repo")),
            account_id: None,
        },
    };
    let json = serde_json::to_string(&unset).unwrap();
    assert!(
        !json.contains("\"account_id\""),
        "unset account_id must not write a key, got: {json}"
    );
}

#[test]
fn legacy_leaf_without_account_id_field_loads_as_none() {
    // Forward-compat: state files written before per-pane accounts existed
    // omit the `account_id` key entirely.
    let legacy_json = r#"{"type":"Leaf","pane_id":1,"cwd":"/some/dir"}"#;
    let restored: SerializedLayout = serde_json::from_str(legacy_json).unwrap();
    match restored {
        SerializedLayout::Leaf {
            content: SerializedPaneContent::Terminal { account_id, .. },
            ..
        } => assert!(account_id.is_none()),
        other => panic!("expected a terminal leaf, got {other:?}"),
    }
}

#[test]
fn agent_chat_content_round_trip_preserves_account_id() {
    use crate::accounts::AccountId;
    let id = AccountId::new();
    let content = SerializedAgentChatContent {
        cwd: Some(PaneCwd::Local(PathBuf::from("/repo/lane"))),
        session_id: None,
        title: None,
        agent_id: Some("claude".to_string()),
        account_id: Some(id),
        mode_id: None,
        model_id: None,
        content_width: SerializedChatContentWidth::Full,
        tail_window: None,
        display_filter: None,
        fold_mode: None,
    };
    let json = serde_json::to_string(&content).unwrap();
    let restored: SerializedAgentChatContent = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.account_id, Some(id));

    // Legacy payload (pre-account_id) still loads, defaulting to None.
    let legacy_json = r#"{"cwd":"/repo/lane"}"#;
    let legacy: SerializedAgentChatContent = serde_json::from_str(legacy_json).unwrap();
    assert!(legacy.account_id.is_none());
}

#[test]
fn agent_chat_content_round_trip_preserves_mode_id() {
    // A restart resumes the session via `session/load`; the persisted mode
    // id is what lets the host reapply the last-known mode afterward (see
    // `SerializedAgentChatContent::mode_id`).
    let content = SerializedAgentChatContent {
        cwd: Some(PaneCwd::Local(PathBuf::from("/repo/lane"))),
        session_id: Some("sess-abc123".to_string()),
        title: None,
        agent_id: Some("claude".to_string()),
        account_id: None,
        mode_id: Some("acceptEdits".to_string()),
        model_id: None,
        content_width: SerializedChatContentWidth::Full,
        tail_window: None,
        display_filter: None,
        fold_mode: None,
    };
    let json = serde_json::to_string(&content).unwrap();
    let restored: SerializedAgentChatContent = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.mode_id, Some("acceptEdits".to_string()));

    // Legacy payload (pre-mode_id) still loads, defaulting to None.
    let legacy_json = r#"{"cwd":"/repo/lane"}"#;
    let legacy: SerializedAgentChatContent = serde_json::from_str(legacy_json).unwrap();
    assert!(legacy.mode_id.is_none());
}

#[test]
fn agent_chat_content_round_trip_preserves_model_id() {
    // Every connect reapplies this pane's model, so the pick has to survive
    // the restart that ends the session it was made in.
    let content = SerializedAgentChatContent {
        cwd: Some(PaneCwd::Local(PathBuf::from("/repo/lane"))),
        session_id: Some("sess-abc123".to_string()),
        title: None,
        agent_id: Some("claude".to_string()),
        account_id: None,
        mode_id: None,
        model_id: Some("opus".to_string()),
        content_width: SerializedChatContentWidth::Full,
        tail_window: None,
        display_filter: None,
        fold_mode: None,
    };
    let json = serde_json::to_string(&content).unwrap();
    assert!(
        !json.contains("mode_id"),
        "an unset mode/model must stay out of the payload: {json}"
    );
    let restored: SerializedAgentChatContent = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.model_id, Some("opus".to_string()));

    // Legacy payload (pre-model_id) still loads, defaulting to None.
    let legacy_json = r#"{"cwd":"/repo/lane","mode_id":"acceptEdits"}"#;
    let legacy: SerializedAgentChatContent = serde_json::from_str(legacy_json).unwrap();
    assert!(legacy.model_id.is_none());
    assert_eq!(legacy.mode_id, Some("acceptEdits".to_string()));
}

#[test]
fn agent_chat_content_width_round_trips_and_legacy_defaults_to_full() {
    let content = SerializedAgentChatContent {
        cwd: Some(PaneCwd::Local(PathBuf::from("/repo/lane"))),
        session_id: Some("sess-abc123".to_string()),
        title: None,
        agent_id: Some("claude".to_string()),
        account_id: None,
        mode_id: None,
        model_id: None,
        content_width: SerializedChatContentWidth::Reading,
        tail_window: None,
        display_filter: None,
        fold_mode: None,
    };
    let json = serde_json::to_string(&content).unwrap();
    let restored: SerializedAgentChatContent = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.content_width, SerializedChatContentWidth::Reading);

    let legacy_json = r#"{"cwd":"/repo/lane"}"#;
    let legacy: SerializedAgentChatContent = serde_json::from_str(legacy_json).unwrap();
    assert_eq!(legacy.content_width, SerializedChatContentWidth::Full);
}

#[test]
fn agent_chat_display_filter_round_trips_and_legacy_stays_unset() {
    let content = SerializedAgentChatContent {
        cwd: Some(PaneCwd::Local(PathBuf::from("/repo/lane"))),
        session_id: None,
        title: None,
        agent_id: Some("claude".to_string()),
        account_id: None,
        mode_id: None,
        model_id: None,
        content_width: SerializedChatContentWidth::Full,
        tail_window: None,
        display_filter: Some(vec!["tools".to_string(), "tool_edit".to_string()]),
        fold_mode: None,
    };
    let json = serde_json::to_string(&content).unwrap();
    let restored: SerializedAgentChatContent = serde_json::from_str(&json).unwrap();
    assert_eq!(
        restored.display_filter,
        Some(vec!["tools".to_string(), "tool_edit".to_string()])
    );

    let cleared = SerializedAgentChatContent {
        display_filter: Some(Vec::new()),
        ..content
    };
    let json = serde_json::to_string(&cleared).unwrap();
    let restored: SerializedAgentChatContent = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.display_filter, Some(Vec::new()));

    let legacy_json = r#"{"cwd":"/repo/lane"}"#;
    let legacy: SerializedAgentChatContent = serde_json::from_str(legacy_json).unwrap();
    assert_eq!(legacy.display_filter, None);

    let newer_json = r#"{"cwd":"/repo/lane","display_filter":["tools","from_the_future"]}"#;
    let newer: SerializedAgentChatContent = serde_json::from_str(newer_json).unwrap();
    assert_eq!(
        newer.display_filter,
        Some(vec!["tools".to_string(), "from_the_future".to_string()])
    );
}

#[test]
fn agent_chat_fold_mode_round_trips_and_legacy_stays_unset() {
    let content = SerializedAgentChatContent {
        cwd: Some(PaneCwd::Local(PathBuf::from("/repo/lane"))),
        session_id: None,
        title: None,
        agent_id: Some("claude".to_string()),
        account_id: None,
        mode_id: None,
        model_id: None,
        content_width: SerializedChatContentWidth::Full,
        tail_window: None,
        display_filter: None,
        fold_mode: Some(vec![
            "summary".to_string(),
            "last.tool=expanded".to_string(),
        ]),
    };
    let json = serde_json::to_string(&content).unwrap();
    let restored: SerializedAgentChatContent = serde_json::from_str(&json).unwrap();
    assert_eq!(
        restored.fold_mode,
        Some(vec![
            "summary".to_string(),
            "last.tool=expanded".to_string()
        ])
    );

    let auto = SerializedAgentChatContent {
        fold_mode: Some(vec!["auto".to_string()]),
        display_filter: None,
        ..content
    };
    let json = serde_json::to_string(&auto).unwrap();
    let restored: SerializedAgentChatContent = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.fold_mode, Some(vec!["auto".to_string()]));

    let legacy_json = r#"{"cwd":"/repo/lane"}"#;
    let legacy: SerializedAgentChatContent = serde_json::from_str(legacy_json).unwrap();
    assert_eq!(legacy.fold_mode, None);

    let newer_json = r#"{"cwd":"/repo/lane","fold_mode":["auto","last.wormhole=expanded"]}"#;
    let newer: SerializedAgentChatContent = serde_json::from_str(newer_json).unwrap();
    assert_eq!(
        newer.fold_mode,
        Some(vec![
            "auto".to_string(),
            "last.wormhole=expanded".to_string()
        ])
    );
}

#[test]
fn agent_chat_tail_window_round_trips_and_legacy_stays_unset() {
    let content = SerializedAgentChatContent {
        cwd: Some(PaneCwd::Local(PathBuf::from("/repo/lane"))),
        session_id: None,
        title: None,
        agent_id: Some("claude".to_string()),
        account_id: None,
        mode_id: None,
        model_id: None,
        content_width: SerializedChatContentWidth::Full,
        tail_window: Some(SerializedChatTailWindow::Last(5)),
        display_filter: None,
        fold_mode: None,
    };
    let json = serde_json::to_string(&content).unwrap();
    let restored: SerializedAgentChatContent = serde_json::from_str(&json).unwrap();
    assert_eq!(
        restored.tail_window,
        Some(SerializedChatTailWindow::Last(5))
    );

    let all = SerializedAgentChatContent {
        tail_window: Some(SerializedChatTailWindow::All),
        display_filter: None,
        fold_mode: None,
        ..content
    };
    let json = serde_json::to_string(&all).unwrap();
    let restored: SerializedAgentChatContent = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.tail_window, Some(SerializedChatTailWindow::All));

    let legacy_json = r#"{"cwd":"/repo/lane"}"#;
    let legacy: SerializedAgentChatContent = serde_json::from_str(legacy_json).unwrap();
    assert_eq!(legacy.tail_window, None);
}

/// Unknown preference tokens must not invalidate the surrounding pane state.
#[test]
fn a_future_view_preference_degrades_instead_of_losing_the_project() {
    let json = r#"{
        "cwd": "/repo/lane",
        "session_id": "sess-abc123",
        "tail_window": "first_and_last",
        "content_width": "half",
        "display_filter": ["tool_read", "tool_wormhole"],
        "fold_mode": ["auto", "last.wormhole=expanded"]
    }"#;
    let content: SerializedAgentChatContent =
        serde_json::from_str(json).expect("an unknown token drops, it does not fail the pane");
    assert_eq!(content.tail_window, None, "unreadable → treated as unset");
    assert_eq!(content.content_width, SerializedChatContentWidth::Full);
    assert!(content.display_filter.is_some());
    assert!(content.fold_mode.is_some());
    assert_eq!(
        content.cwd,
        Some(PaneCwd::Local(PathBuf::from("/repo/lane")))
    );
    assert_eq!(content.session_id.as_deref(), Some("sess-abc123"));

    let leaf: SerializedLayout = serde_json::from_str(&format!(
        r#"{{"type":"Leaf","pane_id":7,"cwd":null,"agent_chat":{json}}}"#
    ))
    .expect("one unreadable preference must not corrupt the pane tree");
    match leaf {
        SerializedLayout::Leaf {
            pane_id,
            content: SerializedPaneContent::AgentChat(chat),
        } => {
            assert_eq!(pane_id, 7);
            assert_eq!(chat.tail_window, None);
        }
        other => panic!("expected an agent-chat leaf, got {other:?}"),
    }

    // A wrong *type*, not just an unknown token: every view preference must
    // still cost only itself, or one bad value takes the whole project's
    // lane/tab/pane tree down with it.
    let mistyped = r#"{
        "cwd": "/repo/lane",
        "session_id": "sess-abc123",
        "tail_window": 5,
        "content_width": ["reading"],
        "display_filter": "tools",
        "fold_mode": {"preset": "summary"}
    }"#;
    let content: SerializedAgentChatContent =
        serde_json::from_str(mistyped).expect("a mistyped preference drops, it does not fail");
    assert_eq!(content.tail_window, None);
    assert_eq!(content.content_width, SerializedChatContentWidth::Full);
    assert_eq!(
        content.display_filter, None,
        "unreadable → treated as unset"
    );
    assert_eq!(content.fold_mode, None, "unreadable → treated as unset");
    assert_eq!(
        content.cwd,
        Some(PaneCwd::Local(PathBuf::from("/repo/lane")))
    );
    assert_eq!(content.session_id.as_deref(), Some("sess-abc123"));

    let leaf: SerializedLayout = serde_json::from_str(&format!(
        r#"{{"type":"Leaf","pane_id":7,"cwd":null,"agent_chat":{mistyped}}}"#
    ))
    .expect("a mistyped preference must not corrupt the pane tree");
    assert!(matches!(
        leaf,
        SerializedLayout::Leaf {
            content: SerializedPaneContent::AgentChat(_),
            ..
        }
    ));
}

#[test]
fn unset_view_preferences_are_left_out_of_the_json() {
    let content = SerializedAgentChatContent {
        cwd: Some(PaneCwd::Local(PathBuf::from("/repo/lane"))),
        session_id: None,
        title: None,
        agent_id: None,
        account_id: None,
        mode_id: None,
        model_id: None,
        content_width: SerializedChatContentWidth::Full,
        tail_window: None,
        display_filter: None,
        fold_mode: None,
    };
    let json = serde_json::to_string(&content).unwrap();
    for key in [
        "tail_window",
        "display_filter",
        "fold_mode",
        "content_width",
    ] {
        assert!(!json.contains(key), "{key} should be omitted: {json}");
    }
}

#[test]
fn agent_chat_leaf_round_trip_preserves_cwd() {
    // AgentChat panes persist the anchored lane cwd plus the ACP
    // session id + title, so a restart restores the pane kind + cwd and
    // resumes the prior session via `session/load` on attach.
    let leaf = SerializedLayout::Leaf {
        pane_id: 9,
        content: SerializedPaneContent::AgentChat(SerializedAgentChatContent {
            cwd: Some(PaneCwd::Local(PathBuf::from("/repo/lane"))),
            session_id: Some("sess-abc123".to_string()),
            title: Some("Fix the parser".to_string()),
            agent_id: Some("claude".to_string()),
            account_id: None,
            mode_id: None,
            model_id: None,
            content_width: SerializedChatContentWidth::Full,
            tail_window: None,
            display_filter: None,
            fold_mode: None,
        }),
    };

    let json = serde_json::to_string(&leaf).unwrap();
    let restored: SerializedLayout = serde_json::from_str(&json).unwrap();
    match restored {
        SerializedLayout::Leaf {
            content: SerializedPaneContent::AgentChat(ac),
            ..
        } => {
            assert_eq!(ac.cwd, Some(PaneCwd::Local(PathBuf::from("/repo/lane"))));
            assert_eq!(ac.session_id, Some("sess-abc123".to_string()));
            assert_eq!(ac.title, Some("Fix the parser".to_string()));
            assert_eq!(ac.agent_id, Some("claude".to_string()));
        }
        _ => panic!("expected Leaf with agent_chat content"),
    }
}

#[test]
fn agent_chat_leaf_round_trip_preserves_remote_cwd() {
    // A `PaneCwd::Remote` cwd (Task 4+ — no pane constructs one yet, but
    // the wire shape must round-trip and stay distinguishable from
    // `Local` so a future remote-backed pane doesn't get silently
    // reinterpreted as a local path on restore.
    let leaf = SerializedLayout::Leaf {
        pane_id: 9,
        content: SerializedPaneContent::AgentChat(SerializedAgentChatContent {
            cwd: Some(PaneCwd::Remote("host:/repo/lane".to_string())),
            session_id: None,
            title: None,
            agent_id: None,
            account_id: None,
            mode_id: None,
            model_id: None,
            content_width: SerializedChatContentWidth::Full,
            tail_window: None,
            display_filter: None,
            fold_mode: None,
        }),
    };

    let json = serde_json::to_string(&leaf).unwrap();
    // The wire shape distinguishes Remote from a bare-string Local path —
    // otherwise a Remote value round-tripping through disk could silently
    // become indistinguishable from a filesystem path.
    assert!(
        json.contains(r#""remote":"host:/repo/lane""#),
        "Remote must serialize as a distinguishable shape, got: {json}"
    );
    let restored: SerializedLayout = serde_json::from_str(&json).unwrap();
    match restored {
        SerializedLayout::Leaf {
            content: SerializedPaneContent::AgentChat(ac),
            ..
        } => {
            assert_eq!(ac.cwd, Some(PaneCwd::Remote("host:/repo/lane".to_string())));
            assert_eq!(ac.cwd.as_ref().and_then(PaneCwd::as_local), None);
        }
        _ => panic!("expected Leaf with agent_chat content"),
    }
}

#[test]
fn agent_chat_leaf_without_session_fields_loads_as_none() {
    // Back-compat: state files written before session persistence existed
    // carry an `agent_chat` object with only `cwd` — the new `session_id`,
    // `title`, and `agent_id` fields must default to `None`.
    let legacy_json = r#"{"type":"Leaf","pane_id":9,"agent_chat":{"cwd":"/repo/lane"}}"#;
    let restored: SerializedLayout = serde_json::from_str(legacy_json).unwrap();
    match restored {
        SerializedLayout::Leaf {
            content: SerializedPaneContent::AgentChat(ac),
            ..
        } => {
            assert_eq!(ac.cwd, Some(PaneCwd::Local(PathBuf::from("/repo/lane"))));
            assert_eq!(ac.session_id, None);
            assert_eq!(ac.title, None);
            assert_eq!(ac.agent_id, None);
        }
        _ => panic!("expected Leaf with agent_chat content"),
    }
}

#[test]
fn flow_graph_leaf_round_trips_its_path_and_nothing_else() {
    // The graph is re-derived from the file on open, so the path is the
    // whole payload — and a graph leaf has no cwd of its own.
    let leaf = SerializedLayout::Leaf {
        pane_id: 11,
        content: SerializedPaneContent::FlowGraph(SerializedFlowGraphContent {
            path: PathBuf::from("/repo/lane/.daruda/flows/ship.yaml"),
        }),
    };

    let json = serde_json::to_string(&leaf).unwrap();
    let restored: SerializedLayout = serde_json::from_str(&json).unwrap();
    match restored {
        SerializedLayout::Leaf {
            content: SerializedPaneContent::FlowGraph(fg),
            ..
        } => assert_eq!(fg.path, PathBuf::from("/repo/lane/.daruda/flows/ship.yaml")),
        other => panic!("expected a flow-graph leaf, got {other:?}"),
    }
}

#[test]
fn a_leaf_that_is_not_a_flow_graph_writes_no_flow_graph_key() {
    // Forward-compat both ways: a leaf written before flow-graph panes
    // existed omits the key and must load as `None`, and every other pane
    // kind keeps writing state files without it.
    let terminal = SerializedLayout::Leaf {
        pane_id: 1,
        content: SerializedPaneContent::Terminal {
            cwd: Some(PathBuf::from("/tmp")),
            account_id: None,
        },
    };
    let json = serde_json::to_string(&terminal).unwrap();
    assert!(
        !json.contains("\"flow_graph\""),
        "a non-graph leaf must not write a `flow_graph` key, got: {json}"
    );

    let legacy_json = r#"{"type":"Leaf","pane_id":1,"cwd":"/some/dir"}"#;
    let restored: SerializedLayout = serde_json::from_str(legacy_json).unwrap();
    match restored {
        SerializedLayout::Leaf {
            content: SerializedPaneContent::Terminal { .. },
            ..
        } => {}
        other => panic!("a leaf naming no kind is a terminal, got {other:?}"),
    }
}

#[test]
fn pane_cwd_local_serializes_as_bare_string_for_back_compat() {
    // The pre-`PaneCwd` on-disk shape of `cwd` was a bare
    // `Option<PathBuf>` string. `Local` must keep that exact shape so
    // existing saved state (all of it necessarily local — `Remote` never
    // existed before this type) loads without a migration step.
    let json = serde_json::to_string(&PaneCwd::Local(PathBuf::from("/repo/lane"))).unwrap();
    assert_eq!(json, r#""/repo/lane""#);
}

#[test]
fn pane_cwd_as_local_and_into_local_gate_remote() {
    let local = PaneCwd::Local(PathBuf::from("/repo/lane"));
    assert_eq!(local.as_local(), Some(Path::new("/repo/lane")));
    assert_eq!(local.into_local(), Some(PathBuf::from("/repo/lane")));

    let remote = PaneCwd::Remote("host:/repo/lane".to_string());
    assert_eq!(remote.as_local(), None);
    assert_eq!(remote.into_local(), None);
}

#[test]
fn agent_chat_leaf_skips_serialization_when_absent() {
    // A non-AgentChat leaf (`agent_chat: None`) emits no `agent_chat`
    // key so existing terminal/file state files stay byte-compatible.
    let leaf = SerializedLayout::Leaf {
        pane_id: 1,
        content: SerializedPaneContent::Terminal {
            cwd: Some(PathBuf::from("/tmp")),
            account_id: None,
        },
    };
    let json = serde_json::to_string(&leaf).unwrap();
    assert!(
        !json.contains("\"agent_chat\""),
        "non-agent-chat leaves must not write an `agent_chat` key, got: {json}"
    );
}

#[test]
fn legacy_leaf_without_agent_chat_field_loads_as_none() {
    // Forward-compat: state files written before AgentChat panes
    // existed omit the `agent_chat` key entirely — they must
    // deserialize with `agent_chat = None`.
    let legacy_json = r#"{"type":"Leaf","pane_id":1,"cwd":"/some/dir"}"#;
    let restored: SerializedLayout = serde_json::from_str(legacy_json).unwrap();
    match restored {
        SerializedLayout::Leaf {
            content: SerializedPaneContent::Terminal { .. },
            ..
        } => {}
        other => panic!("a leaf with no `agent_chat` key is a terminal, got {other:?}"),
    }
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
        remote_cwd: None,
        session_host: None,
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
        remote_cwd: None,
        session_host: None,
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
    // `remote_cwd` didn't exist when this JSON was saved either —
    // #[serde(default)] must fill it in as `None`.
    assert!(w.remote_cwd.is_none());
}

#[test]
fn serialized_worktree_round_trips_base_ref_and_description() {
    let mut w = SerializedLane::default_for_path(0, PathBuf::from("/tmp/scratch"));
    w.base_ref = Some("origin/main".into());
    w.description = Some("PR #123 review".into());
    w.remote_cwd = Some("/remote/path".into());
    let json = serde_json::to_string(&w).unwrap();
    let back: SerializedLane = serde_json::from_str(&json).unwrap();
    assert_eq!(back.base_ref.as_deref(), Some("origin/main"));
    assert_eq!(back.description.as_deref(), Some("PR #123 review"));
    assert_eq!(back.remote_cwd.as_deref(), Some("/remote/path"));
}

#[test]
fn serialized_worktree_remote_cwd_defaults_to_none() {
    let w = SerializedLane::default_for_path(0, PathBuf::from("/tmp/scratch"));
    assert!(w.remote_cwd.is_none());
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
            content: SerializedPaneContent::Terminal {
                cwd: None,
                account_id: None,
            },
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
            content: SerializedPaneContent::Terminal {
                cwd: None,
                account_id: None,
            },
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
        remote_cwd: None,
        session_host: None,
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

/// The shape on disk did not change with the enum. Every key a leaf used to
/// carry is still spelled the same way, and `content` — which is a Rust shape,
/// not a file one — is nowhere in the JSON.
///
/// This is what makes the change need no migration: a state file written by
/// any earlier build loads, and one written now is read by them.
#[test]
fn the_enum_writes_the_keys_the_file_always_had() {
    let graph = SerializedLayout::Leaf {
        pane_id: 2,
        content: SerializedPaneContent::FlowGraph(SerializedFlowGraphContent {
            path: PathBuf::from("/repo/.daruda/flows/ship.yaml"),
        }),
    };
    let json = serde_json::to_string(&graph).unwrap();
    assert!(json.contains(r#""flow_graph":{"path""#), "{json}");
    assert!(
        !json.contains("content"),
        "`content` is how Rust holds it, not how the file spells it: {json}"
    );

    let terminal = SerializedLayout::Leaf {
        pane_id: 1,
        content: SerializedPaneContent::Terminal {
            cwd: Some(PathBuf::from("/tmp")),
            account_id: None,
        },
    };
    assert_eq!(
        serde_json::to_string(&terminal).unwrap(),
        r#"{"type":"Leaf","pane_id":1,"cwd":"/tmp"}"#
    );
}

/// A file naming two kinds cannot be written by this app, and is not worth
/// refusing to open a window over. The first one wins, in the order restore
/// already read them.
#[test]
fn a_leaf_naming_two_kinds_takes_the_first() {
    let json = r#"{"type":"Leaf","pane_id":1,"cwd":"/tmp",
        "file":{"worktree_id":1,"path":"a.rs","staged":false,"view_mode":"raw"},
        "flow_graph":{"path":"ship.yaml"}}"#;
    let restored: SerializedLayout = serde_json::from_str(json).unwrap();
    match restored {
        SerializedLayout::Leaf {
            content: SerializedPaneContent::File(fc),
            ..
        } => assert_eq!(fc.path, PathBuf::from("a.rs")),
        other => panic!("expected the file leaf, got {other:?}"),
    }
}
