use super::*;
use daruda_acp::{ChatItem, ModeStateView, SessionModeView, ToolCallItem};

fn asst(text: &str) -> ChatItem {
    ChatItem::AssistantText {
        text: text.to_owned(),
        streaming: false,
        message_id: None,
        phase: Default::default(),
    }
}

fn rollup(items: &[ChatItem], range: std::ops::Range<usize>) -> Rollup {
    let live_units = LiveSubagentUnits::of(items);
    Rollup::of_run_with_live_units(items, range, &live_units)
}

#[test]
fn has_conversation_distinguishes_empty_error_only_and_real_transcript() {
    assert!(!has_conversation(&[]));

    // The pane that hit a usage limit holds nothing but the notice — the
    // switch that follows has no transcript to protect.
    let items = [ChatItem::Failure(daruda_acp::AcpFailure::unclassified(
        "session limit reached",
    ))];
    assert!(!has_conversation(&items));

    for item in [
        ChatItem::UserText("hi".to_owned()),
        asst("hello"),
        ChatItem::Failure(daruda_acp::AcpFailure::unclassified("boom")),
    ]
    .windows(2)
    .map(|w| w.to_vec())
    {
        assert!(has_conversation(&item), "{item:?} holds a transcript");
    }
}

/// A *classified* failure must be excluded exactly like an unclassified one.
/// The classes are what an auth failure now arrives as, and they are the whole
/// reason a user reaches for another account — so a pane holding only these is
/// still empty and must reconnect in place rather than stranding the user with
/// a fresh pane every time a login expires.
#[test]
fn has_conversation_excludes_every_failure_class() {
    let classes = [
        daruda_acp::AcpFailure::classify(&agent_client_protocol::Error::new(
            -32000,
            "Authentication required",
        )),
        daruda_acp::AcpFailure::Categorized {
            kind: daruda_acp::FailureKind::OauthOrgNotAllowed,
            message: "org disabled".to_owned(),
        },
        daruda_acp::AcpFailure::Runtime {
            kind: daruda_acp::RuntimeKind::Download,
            message: "no network".to_owned(),
        },
        daruda_acp::AcpFailure::unclassified("stream ended"),
    ];
    for failure in classes {
        let items = [ChatItem::Failure(failure.clone())];
        assert!(
            !has_conversation(&items),
            "{failure:?} is a notice, not a transcript"
        );
    }
}

/// Ties the exclusion to the decision it actually drives: an account switch on
/// a failure-only pane reuses the pane instead of opening a new one.
#[test]
fn a_failure_only_pane_switches_accounts_in_place() {
    use crate::workspace::account_ops::{SwitchKind, switch_kind};

    let items = [ChatItem::Failure(daruda_acp::AcpFailure::classify(
        &agent_client_protocol::Error::new(-32000, "Authentication required"),
    ))];
    assert_eq!(
        switch_kind(false, has_conversation(&items)),
        SwitchKind::InPlace,
        "an expired login leaves nothing to protect — reuse the pane"
    );
}

#[test]
fn activity_bar_title_prefers_session_title_then_prompt_preview() {
    let items = [ChatItem::UserText("run the tests".to_owned())];
    assert_eq!(
        activity_bar_title(Some("Refactor fold state"), &items).as_deref(),
        Some("Refactor fold state")
    );

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

    let items = [ChatItem::UserText("hello".to_owned())];
    assert_eq!(
        activity_bar_title(Some("   "), &items).as_deref(),
        Some("hello")
    );

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
fn summary_preview_line_flattens_inline_markdown_and_blanks() {
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
    assert_eq!(summary_preview_line(""), None);
    assert_eq!(summary_preview_line("   \n\t\n"), None);
}

#[test]
fn normalize_prompt_title_keeps_short_and_truncates_long_on_char_boundary() {
    assert_eq!(normalize_prompt_title("short one"), "short one");

    let long = "가".repeat(100);
    let title = normalize_prompt_title(&long);
    // 69 kept glyphs + the ellipsis (never a split multibyte char).
    assert_eq!(title.chars().count(), FALLBACK_TITLE_HEAD + 1);
    assert!(title.ends_with('…'));
}

#[test]
fn normalize_prompt_title_collapses_whitespace_at_the_budget_boundary() {
    // Boundary armour for the bounded rewrite: the budget is decided on the
    // *normalized* glyph count, so whitespace runs must collapse first.
    assert_eq!(normalize_prompt_title("  a\t\tb \n c  "), "a b c");
    assert_eq!(normalize_prompt_title(""), "");
    assert_eq!(normalize_prompt_title(" \t\n "), "");

    // Exactly at the limit → kept whole; one glyph past → ellipsized.
    let at_limit = "x".repeat(FALLBACK_TITLE_MAX);
    assert_eq!(normalize_prompt_title(&at_limit), at_limit);
    let past_limit = "x".repeat(FALLBACK_TITLE_MAX + 1);
    let title = normalize_prompt_title(&past_limit);
    assert_eq!(title.chars().count(), FALLBACK_TITLE_HEAD + 1);
    assert!(title.ends_with('…'));

    // A word boundary landing on the cut must not leave a dangling space
    // before the ellipsis.
    let words = format!("{} tail", "y".repeat(FALLBACK_TITLE_HEAD - 1));
    let title = normalize_prompt_title(&words);
    assert!(
        !title.contains(" …"),
        "no dangling space before the ellipsis"
    );
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
fn next_mode_id_handles_cycle_edge_and_stale_current() {
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

    // Zero or one advertised mode → nothing to cycle.
    assert_eq!(next_mode_id(&modes(&[], "")), None);
    assert_eq!(next_mode_id(&modes(&["default"], "default")), None);

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

/// Diff model creation covers created files, unchanged input, and the gutter
/// policy for creation-vs-edit snippets.
#[test]
fn diff_view_model_handles_created_unchanged_and_line_number_policy() {
    // Creation: gutter carries a non-empty number for the added line.
    let created = diff(None, "line one\nline two\n", "new.txt");
    let (m, _) = build_diff_view_model(&created, TEST_SYNTAX_THEME, false, &diff_colors())
        .expect("a created file produces hunks");
    assert!(m.text.contains("line one"));
    assert!(m.text.contains("line two"));
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

    // Identical sides yield no hunks, so the adapter returns `None` and the
    // caller keeps the inline fallback.
    let unchanged = diff(Some("same\n"), "same\n", "same.txt");
    assert!(build_diff_view_model(&unchanged, TEST_SYNTAX_THEME, false, &diff_colors()).is_none());
}

/// Diff stats cover modified, created, deleted, and unchanged inputs.
#[test]
fn diff_stats_cover_modified_created_deleted_and_empty_hunks() {
    // A simple one-line modification must report the changed line on each side,
    // not the file's total line counts.
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

    // A newly created file diffs against an empty old side, so every line is an
    // addition.
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

    // A pure deletion is the mirror of the all-added created-file case.
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

    // Identical sides produce no hunks -> no editor and no stat.
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
fn diff_build_fingerprint_tracks_content_and_theme() {
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
    const THEME: u64 = 7;
    assert_ne!(
        diff_build_fingerprint(&partial, THEME),
        diff_build_fingerprint(&full, THEME)
    );
    assert_eq!(
        diff_build_fingerprint(&partial, THEME),
        diff_build_fingerprint(&partial_again, THEME)
    );
    // The theme is an input too: same diff, swapped palette, must not be
    // mistaken for unchanged — a built diff embed cannot re-theme itself.
    assert_ne!(
        diff_build_fingerprint(&partial, THEME),
        diff_build_fingerprint(&partial, THEME + 1)
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
        exit: None,
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

    // `prompt` alone (no `subagent_type`) still routes to `Subagent` — the two
    // fields are checked together via `is_subagent_launch` so this classification
    // can't drift out of step with `renders_subagent_instructions`, which is
    // gated on `prompt` alone.
    let mut prompt_only = tool_call("task-3", InProgress, 0);
    prompt_only.raw_input = Some(serde_json::json!({ "prompt": "Do the thing." }));
    assert_eq!(
        tool_fold_key(&prompt_only),
        FoldKey::Subagent("task-3".to_owned())
    );
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
        phase: Default::default(),
    }));
    assert!(!is_active(&ChatItem::AssistantText {
        text: "a".to_owned(),
        streaming: false,
        message_id: None,
        phase: Default::default(),
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
    assert!(!is_active(&ChatItem::Failure(
        daruda_acp::AcpFailure::unclassified("e")
    )));
}

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
            phase: Default::default(),
        },
        ChatItem::ToolCall(tool_call("t-live", InProgress, 0)),
        ChatItem::ToolCall(tool_call("t-done", Completed, 0)),
        ChatItem::UserText("q2".to_owned()),
        ChatItem::AssistantText {
            text: "b".to_owned(),
            streaming: false,
            message_id: None,
            phase: Default::default(),
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
    assert!(fold_active(&FoldKey::Response(0), &items));
    // Activity is independent of whether a key belongs to the newest turn.
    assert!(!fold_active(&FoldKey::Response(4), &items));
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

#[test]
fn fold_turn_places_each_key_relative_to_the_newest_prompt() {
    use crate::workspace::main_area::agent_chat_pane::fold_mode::TurnPosition;
    use daruda_acp::ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q1".to_owned()),
        ChatItem::ToolCall(tool_call("t-old", Completed, 0)),
        ChatItem::AssistantText {
            text: "a".to_owned(),
            streaming: false,
            message_id: None,
            phase: Default::default(),
        },
        ChatItem::UserText("q2".to_owned()),
        ChatItem::ToolCall(tool_call("t-new", Completed, 0)),
    ];
    let past = |key: FoldKey| assert_eq!(fold_turn(&key, &items), TurnPosition::Past, "{key:?}");
    let last = |key: FoldKey| assert_eq!(fold_turn(&key, &items), TurnPosition::Last, "{key:?}");

    past(FoldKey::Response(0));
    past(FoldKey::Assistant(2));
    past(FoldKey::Step(1));
    past(FoldKey::Tool("t-old".to_owned()));
    past(FoldKey::ToolGroup("t-old".to_owned()));
    past(FoldKey::Diff("t-old#0".to_owned()));
    past(FoldKey::ToolRawInput("t-old".to_owned()));

    last(FoldKey::Response(3));
    last(FoldKey::Tail(4));
    last(FoldKey::Filtered(4));
    last(FoldKey::Tool("t-new".to_owned()));
    last(FoldKey::Diff("t-new#0".to_owned()));

    past(FoldKey::Tool("nope".to_owned()));
    last(FoldKey::Assistant(99));

    let leading = [ChatItem::AssistantText {
        text: "a".to_owned(),
        streaming: false,
        message_id: None,
        phase: Default::default(),
    }];
    assert_eq!(
        fold_turn(&FoldKey::Assistant(0), &leading),
        TurnPosition::Last
    );
}

/// `chat_item_mermaid_texts` covers every markdown body that can carry a
/// mermaid fence: assistant/thinking/user text, and each `Text` output block
/// of a tool call (a `ResourceLink` block contributes none). Permission /
/// error items carry none.
#[test]
fn chat_item_mermaid_texts_includes_tool_output_text() {
    let tc = daruda_acp::ToolCallItem {
        id: "t1".into(),
        title: "Write".into(),
        kind: daruda_acp::ToolKindView::Edit,
        tool_name: None,
        status: daruda_acp::ToolStatusView::Completed,
        diffs: Vec::new(),
        output: vec![
            daruda_acp::ToolOutputBlock::Text {
                text: "```mermaid\nflowchart TD\n  A-->B\n```".into(),
                truncated_from: None,
            },
            daruda_acp::ToolOutputBlock::ResourceLink {
                uri: "file:///x".into(),
                name: "x".into(),
            },
        ],
        raw_input: None,
        parent_tool_id: None,
        exit: None,
    };
    let item = daruda_acp::ChatItem::ToolCall(tc);
    let texts = chat_item_mermaid_texts(&item);
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("flowchart"));

    assert!(
        chat_item_mermaid_texts(&daruda_acp::ChatItem::Failure(
            daruda_acp::AcpFailure::unclassified("boom")
        ))
        .is_empty()
    );
    assert_eq!(
        chat_item_mermaid_texts(&daruda_acp::ChatItem::UserText("hi".into())),
        vec!["hi"]
    );
}

/// A subagent-launch `prompt` is scanned too — it's the text the "Instructions"
/// section renders as markdown, so a mermaid fence there must be rasterized by
/// the same reconcile pass, not just handed a `code_block_render` hook that
/// then finds nothing in the cache.
#[test]
fn chat_item_mermaid_texts_includes_subagent_prompt() {
    let tc = daruda_acp::ToolCallItem {
        id: "t1".into(),
        title: "Implement Task 2".into(),
        kind: daruda_acp::ToolKindView::Think,
        tool_name: None,
        status: daruda_acp::ToolStatusView::InProgress,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: Some(serde_json::json!({
            "subagent_type": "general-purpose",
            "prompt": "Draw the flow:\n```mermaid\nflowchart TD\n  A-->B\n```",
        })),
        parent_tool_id: None,
        exit: None,
    };
    let item = daruda_acp::ChatItem::ToolCall(tc);
    let texts = chat_item_mermaid_texts(&item);
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("flowchart"));
}

#[test]
fn mermaid_sources_extract_closed_multiple_unterminated_and_non_mermaid_cases() {
    // A single closed mermaid fence yields its verbatim body.
    let text = "intro\n```mermaid\ngraph TD\nA-->B\n```\noutro";
    assert_eq!(mermaid_sources(text), vec!["graph TD\nA-->B".to_string()]);

    // Multiple closed fences are returned in document order.
    let text = "```mermaid\nA\n```\nmid\n```mermaid\nB\n```";
    assert_eq!(
        mermaid_sources(text),
        vec!["A".to_string(), "B".to_string()]
    );

    // An unterminated trailing fence (still streaming) is skipped.
    let text = "```mermaid\nA\n```\n```mermaid\nstill streaming";
    assert_eq!(mermaid_sources(text), vec!["A".to_string()]);
    // A lone unterminated fence yields nothing.
    assert!(mermaid_sources("```mermaid\ngraph TD").is_empty());

    // Non-mermaid fences (other languages, or none) are ignored.
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
            phase: Default::default(),
        },
        ChatItem::Thinking {
            text: "t".to_owned(),
            streaming: false,
            message_id: None,
        },
        ChatItem::ToolCall(tool_call("c1", Completed, 2)),
        ChatItem::Failure(daruda_acp::AcpFailure::unclassified("e")),
    ];
    let keys = collect_foldable_keys(&items);
    assert_eq!(
        keys,
        vec![
            FoldKey::Response(0),
            FoldKey::Step(1),
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
            phase: Default::default(),
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
        exit: None,
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

/// `renders_subagent_instructions` is the single gate shared by the renderer
/// and `renders_raw_input`; a subagent launch (a `prompt` field present) gets
/// the purpose-built, always-visible "Instructions" section instead of the
/// generic raw-input JSON dump, so the two gates must be mutually exclusive.
/// Unlike `ToolRawInput`, "Instructions" has no fold key of its own — it isn't
/// a disclosure — so `collect_foldable_keys` must not contribute one for it.
#[test]
fn subagent_instructions_gate_excludes_generic_raw_input() {
    use daruda_acp::{ChatItem, ToolKindView, ToolStatusView};
    let subagent = ToolCallItem {
        id: "c1".to_owned(),
        title: "Implement Task 2".to_owned(),
        kind: ToolKindView::Think,
        tool_name: None,
        status: ToolStatusView::InProgress,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: Some(serde_json::json!({
            "description": "Implement Task 2",
            "subagent_type": "general-purpose",
            "run_in_background": false,
            "prompt": "Full instructions…",
        })),
        parent_tool_id: None,
        exit: None,
    };
    assert!(renders_subagent_instructions(&subagent));
    assert!(!renders_raw_input(&subagent));
    let keys = collect_foldable_keys(&[ChatItem::ToolCall(subagent.clone())]);
    assert!(!keys.iter().any(|k| matches!(k, FoldKey::ToolRawInput(_))));

    // A `Task`-shaped call without a `prompt` field falls back to the generic
    // disclosure so its args stay reachable.
    let no_prompt = ToolCallItem {
        raw_input: Some(serde_json::json!({ "subagent_type": "general-purpose" })),
        ..subagent
    };
    assert!(!renders_subagent_instructions(&no_prompt));
    assert!(renders_raw_input(&no_prompt));
}

/// A live subagent launch's `output` echoes the same `prompt` the always-on
/// "Instructions" section already shows (see `fold_output`'s replace
/// semantics), so the generic `Output` section must stay suppressed while it
/// runs — and reappear once it settles into the subagent's real result.
#[test]
fn suppresses_live_subagent_output_only_while_the_launch_is_live() {
    use daruda_acp::{ToolKindView, ToolOutputBlock, ToolStatusView};
    let subagent = ToolCallItem {
        id: "c1".to_owned(),
        title: "Implement Task 2".to_owned(),
        kind: ToolKindView::Think,
        tool_name: None,
        status: ToolStatusView::InProgress,
        diffs: Vec::new(),
        output: vec![ToolOutputBlock::Text {
            text: "Full instructions…".to_owned(),
            truncated_from: None,
        }],
        raw_input: Some(serde_json::json!({ "prompt": "Full instructions…" })),
        parent_tool_id: None,
        exit: None,
    };
    assert!(suppresses_live_subagent_output(&subagent));

    let completed = ToolCallItem {
        status: ToolStatusView::Completed,
        output: vec![ToolOutputBlock::Text {
            text: "Done: the result.".to_owned(),
            truncated_from: None,
        }],
        ..subagent.clone()
    };
    assert!(!suppresses_live_subagent_output(&completed));

    // A plain (non-subagent) live tool call is never suppressed.
    let plain = ToolCallItem {
        raw_input: None,
        ..subagent
    };
    assert!(!suppresses_live_subagent_output(&plain));
}

#[test]
fn rollup_of_run_covers_success_failure_running_partial_cancelled_and_ranges() {
    let items = [
        ChatItem::UserText("go".to_owned()),
        ChatItem::ToolCall(tool_call("c1", daruda_acp::ToolStatusView::Completed, 0)),
        asst("done"),
    ];
    assert_eq!(rollup(&items, 1..3), Rollup::Ok);

    let items = [
        ChatItem::ToolCall(tool_call("c1", daruda_acp::ToolStatusView::Failed, 0)),
        ChatItem::ToolCall(tool_call("c2", daruda_acp::ToolStatusView::InProgress, 0)),
    ];
    assert_eq!(rollup(&items, 0..2), Rollup::Running);

    // Produced prose counts as success, so an answered turn that also hit a tool
    // failure warns rather than reading as a hard failure.
    let items = [
        ChatItem::ToolCall(tool_call("c1", daruda_acp::ToolStatusView::Failed, 0)),
        asst("here is what I found anyway"),
    ];
    assert_eq!(rollup(&items, 0..2), Rollup::Partial);

    let items = [
        ChatItem::ToolCall(tool_call("c1", daruda_acp::ToolStatusView::Failed, 0)),
        ChatItem::Failure(daruda_acp::AcpFailure::unclassified("boom")),
        // Empty prose is not output, so it cannot lift this to Partial.
        asst("   "),
    ];
    assert_eq!(rollup(&items, 0..3), Rollup::Failed);

    // Settled but neither success nor failure: the run stops pulsing without
    // turning the glyph red.
    let items = [ChatItem::ToolCall(tool_call(
        "c1",
        daruda_acp::ToolStatusView::Cancelled,
        0,
    ))];
    assert_eq!(rollup(&items, 0..1), Rollup::Ok);

    let items = [ChatItem::AssistantText {
        text: "thinking out loud".to_owned(),
        streaming: true,
        message_id: None,
        phase: Default::default(),
    }];
    assert_eq!(rollup(&items, 0..1), Rollup::Running);

    // A single-item run is addressed as `ix..ix + 1` by the top-level assistant
    // block, and group ranges come from a projection — neither should have to
    // clamp against `items.len()`.
    let items = [asst("only")];
    assert_eq!(rollup(&items, 0..9), Rollup::Ok);
    assert_eq!(rollup(&items, 5..9), Rollup::Ok);
}

#[test]
fn agent_run_covers_next_user_empty_and_out_of_bounds_cases() {
    let items = [
        ChatItem::UserText("q1".to_owned()),
        asst("a1"),
        ChatItem::ToolCall(tool_call("c1", daruda_acp::ToolStatusView::Completed, 0)),
        ChatItem::UserText("q2".to_owned()),
        asst("a2"),
    ];
    // From the first anchor's reply through the tool call, stopping at `q2`.
    assert_eq!(agent_run(&items, 1), 1..3);
    // The last turn runs to the end of the conversation.
    assert_eq!(agent_run(&items, 4), 4..5);

    // `anchor + 1 == items.len()` — the in-flight first turn. Must be an empty
    // range rather than a panic or an inverted one.
    let items = [ChatItem::UserText("q".to_owned())];
    let run = agent_run(&items, 1);
    assert!(run.is_empty(), "{run:?} should be empty");
    assert_eq!(run, 1..1);

    // Two prompts back to back (a re-prompt before any output): the first turn's
    // run is empty, not the whole tail.
    let items = [
        ChatItem::UserText("q1".to_owned()),
        ChatItem::UserText("q2".to_owned()),
        asst("a"),
    ];
    assert_eq!(agent_run(&items, 1), 1..1);

    let items = [asst("only")];
    assert_eq!(agent_run(&items, 9), 1..1);
}

/// A header glyph sits on a disclosure, so it must summarize what expanding
/// the row shows. The shipped bug: an Edits filter hid a failed shell command
/// but its ✗ stayed on the header, marking a run whose every visible card
/// succeeded.
#[test]
fn a_rollup_ignores_the_calls_the_display_filter_removed() {
    use crate::workspace::main_area::agent_chat_pane::display_filter::{
        DisplayFilter, FilterFacet,
    };
    use crate::workspace::main_area::agent_chat_pane::rows::FilterMatchIndex;
    use daruda_acp::{ToolKindView, ToolStatusView};

    let call = |id: &str, kind, status| {
        ChatItem::ToolCall(ToolCallItem {
            id: id.to_owned(),
            title: format!("Tool {id}"),
            kind,
            tool_name: None,
            status,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: None,
            exit: None,
        })
    };
    let items = [
        call("edit", ToolKindView::Edit, ToolStatusView::Completed),
        call("shell", ToolKindView::Execute, ToolStatusView::Failed),
    ];
    let live_units = LiveSubagentUnits::of(&items);
    let rollup_under = |filter: DisplayFilter| {
        let index = FilterMatchIndex::of(&items, filter);
        Rollup::of_kept_run(&items, 0..items.len(), &live_units, |item| {
            index.matches(item)
        })
    };

    assert_eq!(
        rollup_under(DisplayFilter::default()),
        Rollup::Partial,
        "unfiltered, the failed command is on screen and the glyph says so"
    );
    assert_eq!(
        rollup_under(DisplayFilter::default().toggled(FilterFacet::ToolRun)),
        Rollup::Ok,
        "hiding commands takes the failure off screen, and the glyph with it"
    );
}

/// Progress is why a row survives an enclosing fold at all, and the projection
/// decides that without consulting the filter. So a run still working reads as
/// working even when the only live call is one the filter took off screen —
/// otherwise the glyph denies the reason its own row is on screen.
#[test]
fn a_rollup_still_reads_running_when_the_live_call_is_filtered_away() {
    use crate::workspace::main_area::agent_chat_pane::display_filter::{
        DisplayFilter, FilterFacet,
    };
    use crate::workspace::main_area::agent_chat_pane::rows::FilterMatchIndex;
    use daruda_acp::{ToolKindView, ToolStatusView};

    let call = |id: &str, kind, status| {
        ChatItem::ToolCall(ToolCallItem {
            id: id.to_owned(),
            title: format!("Tool {id}"),
            kind,
            tool_name: None,
            status,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: None,
            exit: None,
        })
    };
    let items = [
        call("edit", ToolKindView::Edit, ToolStatusView::Completed),
        call("shell", ToolKindView::Execute, ToolStatusView::InProgress),
    ];
    let live_units = LiveSubagentUnits::of(&items);
    let index = FilterMatchIndex::of(
        &items,
        DisplayFilter::default().toggled(FilterFacet::ToolEdit),
    );
    assert_eq!(
        Rollup::of_kept_run(&items, 0..items.len(), &live_units, |item| index
            .matches(item)),
        Rollup::Running,
        "the hidden command is still running, and the row is on screen because of it"
    );
}
