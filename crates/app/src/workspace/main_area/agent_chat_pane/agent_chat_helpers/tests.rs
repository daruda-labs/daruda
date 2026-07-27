use super::*;
use daruda_acp::{ChatItem, ModeStateView, SessionModeView, ToolCallItem};

fn asst(text: &str) -> ChatItem {
    ChatItem::AssistantText {
        text: text.to_owned(),
        streaming: false,
        message_id: None,
    }
}

#[test]
fn has_conversation_is_false_for_an_empty_pane() {
    assert!(!has_conversation(&[]));
}

#[test]
fn has_conversation_ignores_error_only_content() {
    // The pane that hit a usage limit holds nothing but the notice — the
    // switch that follows has no transcript to protect.
    let items = [ChatItem::Error("session limit reached".to_owned())];
    assert!(!has_conversation(&items));
}

#[test]
fn has_conversation_counts_any_real_transcript_item() {
    for item in [
        ChatItem::UserText("hi".to_owned()),
        asst("hello"),
        ChatItem::Error("boom".to_owned()),
    ]
    .windows(2)
    .map(|w| w.to_vec())
    {
        assert!(has_conversation(&item), "{item:?} holds a transcript");
    }
}

#[test]
fn activity_bar_title_prefers_the_session_title() {
    let items = [ChatItem::UserText("run the tests".to_owned())];
    assert_eq!(
        activity_bar_title(Some("Refactor fold state"), &items).as_deref(),
        Some("Refactor fold state")
    );
}

#[test]
fn activity_bar_title_falls_back_to_first_user_prompt() {
    // No session title yet (pre first turn-end): the first prompt stands in.
    let items = [
        ChatItem::UserText("  fix the   parser  ".to_owned()),
        asst("sure"),
        ChatItem::UserText("second".to_owned()),
    ];
    assert_eq!(
        activity_bar_title(None, &items).as_deref(),
        Some("fix the parser")
    );
}

#[test]
fn activity_bar_title_ignores_blank_session_title_and_falls_back() {
    let items = [ChatItem::UserText("hello".to_owned())];
    assert_eq!(
        activity_bar_title(Some("   "), &items).as_deref(),
        Some("hello")
    );
}

#[test]
fn summary_preview_line_flattens_inline_markdown() {
    // Bold one-liner (the common reasoning-block opener) reads as prose.
    assert_eq!(
        summary_preview_line("**Planning the change** and more").as_deref(),
        Some("Planning the change and more")
    );
    // Inline code, links, and italics all flatten to their visible text.
    assert_eq!(
        summary_preview_line("Call `foo()` in [the module](https://x)").as_deref(),
        Some("Call foo() in the module")
    );
    // Leading blank lines and a heading marker are skipped / stripped.
    assert_eq!(
        summary_preview_line("\n\n# Title here\nbody").as_deref(),
        Some("Title here")
    );
    // A list marker on the first line is dropped, keeping the item text.
    assert_eq!(
        summary_preview_line("- first item\n- second").as_deref(),
        Some("first item")
    );
}

#[test]
fn summary_preview_line_is_none_when_empty() {
    assert_eq!(summary_preview_line(""), None);
    assert_eq!(summary_preview_line("   \n\t\n"), None);
}

#[test]
fn activity_bar_title_is_none_for_an_empty_session() {
    // Neither a session title nor a user prompt → blank bar (no placeholder).
    assert_eq!(activity_bar_title(None, &[]), None);
    // Non-user leading items don't seed a title.
    assert_eq!(activity_bar_title(None, &[asst("greeting")]), None);
    // A whitespace-only prompt yields nothing.
    assert_eq!(
        activity_bar_title(None, &[ChatItem::UserText("   ".to_owned())]),
        None
    );
}

#[test]
fn normalize_prompt_title_truncates_long_prompts_on_a_char_boundary() {
    let long = "가".repeat(100);
    let title = normalize_prompt_title(&long);
    // 69 kept glyphs + the ellipsis (never a split multibyte char).
    assert_eq!(title.chars().count(), FALLBACK_TITLE_HEAD + 1);
    assert!(title.ends_with('…'));
}

#[test]
fn normalize_prompt_title_keeps_short_prompts_verbatim() {
    assert_eq!(normalize_prompt_title("short one"), "short one");
}

fn modes(ids: &[&str], current: &str) -> ModeStateView {
    ModeStateView {
        available: ids
            .iter()
            .map(|id| SessionModeView {
                id: (*id).to_string(),
                name: (*id).to_string(),
                description: None,
            })
            .collect(),
        current: current.to_string(),
    }
}

#[test]
fn next_mode_id_wraps_through_advertised_order() {
    let m = modes(&["default", "acceptEdits", "bypassPermissions"], "default");
    assert_eq!(next_mode_id(&m).as_deref(), Some("acceptEdits"));
    let m = modes(
        &["default", "acceptEdits", "bypassPermissions"],
        "acceptEdits",
    );
    assert_eq!(next_mode_id(&m).as_deref(), Some("bypassPermissions"));
    // Wrap: last → first.
    let m = modes(
        &["default", "acceptEdits", "bypassPermissions"],
        "bypassPermissions",
    );
    assert_eq!(next_mode_id(&m).as_deref(), Some("default"));
}

#[test]
fn next_mode_id_none_when_not_cyclable() {
    // Zero or one advertised mode → nothing to cycle.
    assert_eq!(next_mode_id(&modes(&[], "")), None);
    assert_eq!(next_mode_id(&modes(&["default"], "default")), None);
}

#[test]
fn next_mode_id_starts_from_first_when_current_unknown() {
    let m = modes(&["default", "acceptEdits"], "stale-id");
    assert_eq!(next_mode_id(&m).as_deref(), Some("acceptEdits"));
}

/// A syntax theme id every test reuses for the highlight passes.
const TEST_SYNTAX_THEME: &str = "base16-ocean.dark";

/// A flat `DiffColors` fixture so the pure model build is testable without a
/// live theme.
fn diff_colors() -> DiffColors {
    let c = |l: f32| gpui::Hsla {
        h: 0.,
        s: 0.,
        l,
        a: 1.,
    };
    DiffColors {
        add_bg: c(0.1),
        del_bg: c(0.11),
        hunk_bg: c(0.12),
        add_text: c(0.2),
        del_text: c(0.21),
        ctx_text: c(0.22),
        hunk_text: c(0.23),
        hunk_ctx_text: c(0.24),
        word_add_bg: c(0.3),
        word_del_bg: c(0.31),
    }
}

fn diff(old: Option<&str>, new: &str, path: &str) -> DiffView {
    DiffView {
        path: std::path::PathBuf::from(path),
        old_text: old.map(str::to_owned),
        new_text: new.to_owned(),
    }
}

/// `build_diff_view_model` turns a single-line modification into a
/// `DiffEditorModel` whose synthetic buffer carries the hunk header plus
/// both sides (no `+`/`-` markers — the kind is in the decorations) and
/// whose per-row decorations include add/del backgrounds.
#[test]
fn diff_view_model_builds_rows_and_decorations() {
    let d = diff(
        Some("fn a() {}\nlet x = 1;\nfn b() {}\n"),
        "fn a() {}\nlet y = 2;\nfn b() {}\n",
        "src/lib.rs",
    );
    let (m, _) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
        .expect("a modified file produces hunks");
    // Hunk header row + content rows, no marker prefix on content.
    assert!(m.text.starts_with("@@"), "buffer leads with a hunk header");
    assert!(m.text.contains("let x = 1;"), "removed line present");
    assert!(m.text.contains("let y = 2;"), "added line present");
    // Some rows carry an add/del background (the changed pair).
    let with_bg = m
        .decorations
        .iter()
        .filter(|d| d.background.is_some())
        .count();
    assert!(with_bg >= 2, "at least the changed pair is tinted");
    // One decoration per synthetic-buffer line (no trailing newline), so
    // `decorations.len()` is the editor's display-row count — the value
    // `create_diff_editor` seeds and the tool-card diff body uses to size
    // the editor to its full content. Lock that relationship.
    assert_eq!(
        m.decorations.len(),
        m.text.split('\n').count(),
        "one decoration per display row drives the inline diff height"
    );
}

/// A newly created file (`old_text == None`) diffs against an empty old
/// side — every line is an addition, so the model is built (non-empty).
#[test]
fn diff_view_model_handles_created_file() {
    let d = diff(None, "line one\nline two\n", "new.txt");
    let (m, _) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
        .expect("a created file produces an all-added hunk");
    assert!(m.text.contains("line one"));
    assert!(m.text.contains("line two"));
}

/// Identical sides yield no hunks, so the adapter returns `None` and the
/// caller keeps the inline fallback.
#[test]
fn diff_view_model_none_when_unchanged() {
    let d = diff(Some("same\n"), "same\n", "same.txt");
    assert!(build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors()).is_none());
}

/// A file creation (`old_text == None`) keeps its gutter line numbers —
/// they are the real file lines (1..N). An edit (`old_text == Some`) sends
/// only the replaced snippet, so its numbers would be relative to the
/// snippet, not the file; the gutter is blanked for it.
#[test]
fn line_numbers_shown_for_creation_hidden_for_edit_snippet() {
    // Creation: gutter carries a non-empty number for the added line.
    let created = diff(None, "line one\nline two\n", "new.txt");
    let (m, _) = build_diff_view_model(&created, TEST_SYNTAX_THEME, false, &diff_colors())
        .expect("a created file produces hunks");
    assert!(
        m.decorations
            .iter()
            .any(|d| d.gutter.as_deref().is_some_and(|g| !g.trim().is_empty())),
        "creation keeps real line numbers in the gutter"
    );

    // Edit snippet: every gutter is blank (numbers would mislead).
    let edited = diff(Some("let x = 1;\n"), "let y = 2;\n", "existing.rs");
    let (m, _) = build_diff_view_model(&edited, TEST_SYNTAX_THEME, false, &diff_colors())
        .expect("a modified snippet produces hunks");
    assert!(
        m.decorations
            .iter()
            .all(|d| d.gutter.as_deref() == Some("")),
        "an edit snippet blanks every gutter"
    );
}

/// A simple one-line modification must report the *changed* line on each
/// side — `added = 1, removed = 1` — not the file's total line counts.
#[test]
fn diff_stat_counts_changed_lines_not_totals() {
    let d = diff(
        Some("fn a() {}\nlet x = 1;\nfn b() {}\n"),
        "fn a() {}\nlet y = 2;\nfn b() {}\n",
        "src/lib.rs",
    );
    let (_, stat) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
        .expect("a modified file produces hunks");
    assert_eq!(
        stat,
        DiffStat {
            added: 1,
            removed: 1
        }
    );
}

/// A newly created file (`old_text == None`) diffs against an empty old
/// side, so every line is an addition: `added = N, removed = 0`.
#[test]
fn diff_stat_new_file_is_all_added() {
    let d = diff(None, "line one\nline two\nline three\n", "new.txt");
    let (_, stat) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
        .expect("a created file produces an all-added hunk");
    assert_eq!(
        stat,
        DiffStat {
            added: 3,
            removed: 0
        }
    );
}

/// A pure deletion — the new side drops every line of the old — reports
/// `added = 0, removed = N`, the mirror of the all-added created-file case.
#[test]
fn diff_stat_deleted_lines_are_all_removed() {
    let d = diff(Some("first\nsecond\n"), "", "old.rs");
    let (_, stat) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
        .expect("a fully-deleted file produces an all-removed hunk");
    assert_eq!(
        stat,
        DiffStat {
            added: 0,
            removed: 2
        }
    );
}

/// Identical sides produce no hunks → no editor and no stat. This pins the
/// pure tally directly on empty hunks for clarity.
#[test]
fn diff_stat_unchanged_is_zero() {
    assert_eq!(diff_stat_from_hunks(&[]), DiffStat::default());
}

/// The cache key is per-(tool-call, diff index) so two files in one tool
/// call get distinct editors.
#[test]
fn diff_editor_keys_are_per_file() {
    assert_eq!(diff_editor_key("call-1", 0), "call-1#0");
    assert_ne!(diff_editor_key("call-1", 0), diff_editor_key("call-1", 1));
    assert_ne!(diff_editor_key("call-1", 0), diff_editor_key("call-2", 0));
}

/// A diff whose `new_text` grows (the streaming-write case: an editor was
/// built from an early partial snapshot, then a later `ToolCallUpdate`
/// replaces the diff with the full content) must fingerprint differently
/// so `reconcile_diff_editors` rebuilds instead of keeping the stale
/// editor. Same content must fingerprint identically so an unrelated
/// tool-call touch doesn't churn the editor every pass.
#[test]
fn diff_source_fingerprint_changes_when_content_changes() {
    let partial = DiffView {
        path: std::path::PathBuf::from("/tmp/x.rs"),
        old_text: None,
        new_text: "fn greet() {}\n".to_owned(),
    };
    let full = DiffView {
        path: std::path::PathBuf::from("/tmp/x.rs"),
        old_text: None,
        new_text: "fn greet() {}\nfn farewell() {}\n".to_owned(),
    };
    let partial_again = DiffView {
        path: std::path::PathBuf::from("/tmp/x.rs"),
        old_text: None,
        new_text: "fn greet() {}\n".to_owned(),
    };
    assert_ne!(
        diff_source_fingerprint(&partial),
        diff_source_fingerprint(&full)
    );
    assert_eq!(
        diff_source_fingerprint(&partial),
        diff_source_fingerprint(&partial_again)
    );
}

/// A tool-call item with a given status and diff list, for `is_active` and
/// key-collection coverage.
fn tool_call(id: &str, status: daruda_acp::ToolStatusView, diffs: usize) -> ToolCallItem {
    ToolCallItem {
        id: id.to_owned(),
        title: "t".to_owned(),
        kind: daruda_acp::ToolKindView::Edit,
        tool_name: None,
        status,
        diffs: (0..diffs)
            .map(|i| DiffView {
                path: std::path::PathBuf::from(format!("f{i}.rs")),
                old_text: None,
                new_text: "x".to_owned(),
            })
            .collect(),
        output: Vec::new(),
        raw_input: None,
        parent_tool_id: None,
    }
}

/// A subagent launch (`Task` tool carrying `subagent_type`) keys to
/// `FoldKey::Subagent` (collapsed by default); every other tool call keys to
/// `FoldKey::Tool`.
#[test]
fn tool_fold_key_routes_subagent_launch_to_subagent_variant() {
    use daruda_acp::ToolStatusView::InProgress;

    let plain = tool_call("c1", InProgress, 0);
    assert_eq!(tool_fold_key(&plain), FoldKey::Tool("c1".to_owned()));

    let mut task = tool_call("task-1", InProgress, 0);
    task.raw_input = Some(serde_json::json!({ "subagent_type": "code-reviewer" }));
    assert_eq!(tool_fold_key(&task), FoldKey::Subagent("task-1".to_owned()));

    // An empty `subagent_type` is treated as absent (see `subagent_type`), so
    // it stays a plain tool rather than a subagent box.
    let mut empty = tool_call("task-2", InProgress, 0);
    empty.raw_input = Some(serde_json::json!({ "subagent_type": "" }));
    assert_eq!(tool_fold_key(&empty), FoldKey::Tool("task-2".to_owned()));
}

/// `is_active` is true while a block is streaming, or a tool call is live
/// (`Pending` or `InProgress` — see [`ToolStatusView::is_live`]).
#[test]
fn is_active_matches_streaming_and_in_progress() {
    use daruda_acp::ToolStatusView::*;
    assert!(is_active(&ChatItem::AssistantText {
        text: "a".to_owned(),
        streaming: true,
        message_id: None,
    }));
    assert!(!is_active(&ChatItem::AssistantText {
        text: "a".to_owned(),
        streaming: false,
        message_id: None,
    }));
    assert!(is_active(&ChatItem::Thinking {
        text: "t".to_owned(),
        streaming: true,
        message_id: None,
    }));
    assert!(!is_active(&ChatItem::Thinking {
        text: "t".to_owned(),
        streaming: false,
        message_id: None,
    }));
    assert!(is_active(&ChatItem::ToolCall(tool_call(
        "c1", InProgress, 0
    ))));
    // A live `Pending` tool means an in-flight call in the active turn
    // (leftover `Pending` is settled to `Cancelled` at turn end), so it
    // reads as active — same as `InProgress`.
    assert!(is_active(&ChatItem::ToolCall(tool_call("c1", Pending, 0))));
    assert!(!is_active(&ChatItem::ToolCall(tool_call(
        "c1", Completed, 0
    ))));
    assert!(!is_active(&ChatItem::ToolCall(tool_call("c1", Failed, 0))));
    // Non-foldable / inactive items.
    assert!(!is_active(&ChatItem::UserText("u".to_owned())));
    assert!(!is_active(&ChatItem::Error("e".to_owned())));
}

/// `fold_active` — the single source both `rows::project` and
/// `toggle_fold` read — resolves the `active` flag per fold key.
#[test]
fn fold_active_resolves_per_key() {
    use daruda_acp::ToolStatusView::{Completed, InProgress};
    // items: user, streaming assistant, live tool, settled tool, user, assistant
    let items = [
        ChatItem::UserText("q1".to_owned()),
        ChatItem::AssistantText {
            text: "a".to_owned(),
            streaming: true,
            message_id: None,
        },
        ChatItem::ToolCall(tool_call("t-live", InProgress, 0)),
        ChatItem::ToolCall(tool_call("t-done", Completed, 0)),
        ChatItem::UserText("q2".to_owned()),
        ChatItem::AssistantText {
            text: "b".to_owned(),
            streaming: false,
            message_id: None,
        },
    ];
    // Block keys follow the item's own active state.
    assert!(fold_active(&FoldKey::Assistant(1), &items));
    assert!(fold_active(&FoldKey::Tool("t-live".to_owned()), &items));
    assert!(!fold_active(&FoldKey::Tool("t-done".to_owned()), &items));
    // The tool group starting at `t-live` is active (a member runs); a group
    // is scanned as the consecutive run from its gid.
    assert!(fold_active(
        &FoldKey::ToolGroup("t-live".to_owned()),
        &items
    ));
    // Response at anchor 0: not the last turn, but its run streams → active.
    assert!(fold_active(&FoldKey::Response(0), &items));
    // Response at anchor 4: the last turn (no user message after) → active
    // even though its lone block is settled.
    assert!(fold_active(&FoldKey::Response(4), &items));
    // Policy-independent keys ignore `active`.
    assert!(!fold_active(&FoldKey::Diff("t-live#0".to_owned()), &items));
    assert!(!fold_active(
        &FoldKey::ToolRawInput("t-live".to_owned()),
        &items
    ));
    // Unknown ids / out-of-range indices are inactive, not a panic.
    assert!(!fold_active(&FoldKey::Assistant(99), &items));
    assert!(!fold_active(&FoldKey::Tool("nope".to_owned()), &items));
}

/// A single closed mermaid fence yields its verbatim body.
#[test]
fn mermaid_sources_extracts_a_closed_fence() {
    let text = "intro\n```mermaid\ngraph TD\nA-->B\n```\noutro";
    assert_eq!(mermaid_sources(text), vec!["graph TD\nA-->B".to_string()]);
}

/// Multiple closed fences are returned in document order.
#[test]
fn mermaid_sources_extracts_multiple_fences() {
    let text = "```mermaid\nA\n```\nmid\n```mermaid\nB\n```";
    assert_eq!(
        mermaid_sources(text),
        vec!["A".to_string(), "B".to_string()]
    );
}

/// An unterminated trailing fence (still streaming) is skipped — only the
/// already-closed fence before it is returned.
#[test]
fn mermaid_sources_skips_unterminated_trailing_fence() {
    let text = "```mermaid\nA\n```\n```mermaid\nstill streaming";
    assert_eq!(mermaid_sources(text), vec!["A".to_string()]);
    // A lone unterminated fence yields nothing.
    assert!(mermaid_sources("```mermaid\ngraph TD").is_empty());
}

/// Non-mermaid fences (other languages, or none) are ignored.
#[test]
fn mermaid_sources_ignores_non_mermaid_fences() {
    let text = "```rust\nfn main() {}\n```\n```\nplain\n```";
    assert!(mermaid_sources(text).is_empty());
}

/// The cache key is stable per (source, appearance) and distinct across
/// sources *and* across the dark/light appearance — so a light/dark toggle
/// re-rasterizes rather than reusing a stale-coloured diagram.
#[test]
fn mermaid_key_is_stable_and_distinct() {
    assert_eq!(
        mermaid_key("graph TD\nA-->B", true),
        mermaid_key("graph TD\nA-->B", true)
    );
    assert_ne!(
        mermaid_key("graph TD\nA-->B", true),
        mermaid_key("graph LR\nA-->B", true)
    );
    // Same source, different appearance → different key.
    assert_ne!(
        mermaid_key("graph TD\nA-->B", true),
        mermaid_key("graph TD\nA-->B", false)
    );
}

/// The tool-image cache key is stable for the same base64 payload and
/// distinct across different payloads — mirrors `mermaid_key_is_stable_and_distinct`.
#[test]
fn tool_image_key_is_stable_and_distinct() {
    assert_eq!(tool_image_key("abcd1234"), tool_image_key("abcd1234"));
    assert_ne!(tool_image_key("abcd1234"), tool_image_key("wxyz5678"));
}

/// The visible foldable-key set the expand-all / collapse-all op builds.
#[test]
fn visible_fold_keys_cover_text_tools_and_diffs() {
    use daruda_acp::ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("u".to_owned()),
        ChatItem::AssistantText {
            text: "a".to_owned(),
            streaming: false,
            message_id: None,
        },
        ChatItem::Thinking {
            text: "t".to_owned(),
            streaming: false,
            message_id: None,
        },
        ChatItem::ToolCall(tool_call("c1", Completed, 2)),
        ChatItem::Error("e".to_owned()),
    ];
    let keys = collect_foldable_keys(&items);
    // Structural header keys (the response — non-trivial run) first, then
    // the per-block keys. The single tool call is not a group (run < 2). The
    // assistant text (item 1) is the run's conclusion, which carries its own
    // fold toggle, so it contributes an `Assistant` key; thinking keeps its
    // own fold.
    assert_eq!(
        keys,
        vec![
            FoldKey::Response(0),
            FoldKey::Assistant(1),
            FoldKey::Thinking(2),
            FoldKey::Tool("c1".to_owned()),
            FoldKey::Diff("c1#0".to_owned()),
            FoldKey::Diff("c1#1".to_owned()),
        ]
    );
}

/// A trivial single-block reply has no response bar, so its assistant prose
/// keeps the labeled, foldable block — its `Assistant` key is still
/// collected. Guards the inline-vs-block split in `collect_foldable_keys`.
#[test]
fn trivial_reply_keeps_assistant_fold_key() {
    let items = [
        ChatItem::UserText("u".to_owned()),
        ChatItem::AssistantText {
            text: "a".to_owned(),
            streaming: false,
            message_id: None,
        },
    ];
    assert_eq!(collect_foldable_keys(&items), vec![FoldKey::Assistant(1)]);
}

/// A consecutive tool-call run (≥ 2) contributes a `ToolGroup` key on top
/// of the per-tool keys, so expand/collapse-all reaches the group level.
#[test]
fn fold_keys_include_response_and_tool_group() {
    use daruda_acp::ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("u".to_owned()),
        ChatItem::ToolCall(tool_call("c1", Completed, 0)),
        ChatItem::ToolCall(tool_call("c2", Completed, 0)),
    ];
    let keys = collect_foldable_keys(&items);
    assert_eq!(
        keys,
        vec![
            FoldKey::Response(0),
            FoldKey::ToolGroup("c1".to_owned()),
            FoldKey::Tool("c1".to_owned()),
            FoldKey::Tool("c2".to_owned()),
        ]
    );
}

/// `renders_raw_input` is the single gate shared by the renderer and
/// `collect_foldable_keys`; pin both the predicate and the resulting fold
/// coverage so a future edit can't break renderer↔fold sync silently.
#[test]
fn raw_input_disclosure_gate_and_fold_coverage() {
    use daruda_acp::{ChatItem, ToolKindView, ToolStatusView};
    let generic = ToolCallItem {
        id: "c1".to_owned(),
        title: "Grep".to_owned(),
        kind: ToolKindView::Search,
        tool_name: None,
        status: ToolStatusView::Completed,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: Some(serde_json::json!({ "pattern": "foo" })),
        parent_tool_id: None,
    };
    // Generic tool with args and no diffs → disclosure shown, and the fold
    // key is collected (expand/collapse-all reaches it).
    assert!(renders_raw_input(&generic));
    let keys = collect_foldable_keys(&[ChatItem::ToolCall(generic.clone())]);
    assert!(keys.contains(&FoldKey::ToolRawInput("c1".to_owned())));

    // Execute (terminal): the command is already the title → no disclosure,
    // and no fold key for it.
    let exec = ToolCallItem {
        kind: ToolKindView::Execute,
        ..generic.clone()
    };
    assert!(!renders_raw_input(&exec));
    let exec_keys = collect_foldable_keys(&[ChatItem::ToolCall(exec)]);
    assert!(
        !exec_keys
            .iter()
            .any(|k| matches!(k, FoldKey::ToolRawInput(_)))
    );

    // No args, or a diff present (an edit shows the diff) → nothing to show.
    assert!(!renders_raw_input(&ToolCallItem {
        raw_input: None,
        ..generic.clone()
    }));
    assert!(!renders_raw_input(&ToolCallItem {
        diffs: vec![DiffView {
            path: std::path::PathBuf::from("f.rs"),
            old_text: None,
            new_text: "x".to_owned(),
        }],
        ..generic
    }));
}
