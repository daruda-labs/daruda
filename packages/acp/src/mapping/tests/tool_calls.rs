use super::*;

#[test]
fn split_content_types_output_blocks() {
    let content = vec![
        ToolCallContent::Content(Content::new(ContentBlock::Text(TextContent::new("hello")))),
        // Empty text is dropped, not carried as an empty block.
        ToolCallContent::Content(Content::new(ContentBlock::Text(TextContent::new("")))),
        ToolCallContent::Content(Content::new(ContentBlock::ResourceLink(
            ResourceLink::new("file.rs", "file:///tmp/file.rs").mime_type("image/png"),
        ))),
        ToolCallContent::Diff(Diff::new("/tmp/x.txt", "hi\n")),
    ];
    let (diffs, output) = split(&content);
    assert_eq!(diffs.len(), 1);
    assert_eq!(
        output,
        vec![
            ToolOutputBlock::Text {
                text: "hello".to_string(),
                truncated_from: None,
            },
            ToolOutputBlock::ResourceLink {
                uri: "file:///tmp/file.rs".to_string(),
                name: "file.rs".to_string(),
                mime: Some("image/png".to_string()),
            },
        ]
    );
}

#[test]
fn apply_update_with_routes_parent_id_through_adapter() {
    // The mapper must consult the injected strategy for the parent id, not a
    // hardcoded meta read — proven by a stub that returns a fixed sentinel
    // regardless of the (empty) meta.
    struct StubAdapter;
    impl AcpAdapter for StubAdapter {
        fn message_phase(
            &self,
            _meta: &Option<agent_client_protocol::schema::v1::Meta>,
        ) -> crate::adapter::MessagePhase {
            crate::adapter::MessagePhase::Answer
        }

        fn parent_tool_id(
            &self,
            _meta: &Option<agent_client_protocol::schema::v1::Meta>,
        ) -> Option<String> {
            Some("stub-parent".to_owned())
        }

        fn tool_name(
            &self,
            _meta: &Option<agent_client_protocol::schema::v1::Meta>,
        ) -> Option<String> {
            Some("stub-tool".to_owned())
        }

        fn command_exit(
            &self,
            _raw_output: &Option<serde_json::Value>,
            _meta: &Option<agent_client_protocol::schema::v1::Meta>,
        ) -> Option<crate::model::CommandExit> {
            None
        }

        fn sideband_output(
            &self,
            _meta: &Option<agent_client_protocol::schema::v1::Meta>,
        ) -> Option<String> {
            None
        }
    }
    let mut items = Vec::new();
    apply_update_with(
        &mut items,
        &SessionUpdate::ToolCall(ToolCall::new("c1", "x")),
        &StubAdapter,
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    assert_eq!(tc.parent_tool_id, Some("stub-parent".to_owned()));
    // The tool name must likewise come from the injected strategy, not a
    // hardcoded meta read.
    assert_eq!(tc.tool_name, Some("stub-tool".to_owned()));
}

#[test]
fn codex_command_output_surfaces_from_raw_output() {
    // codex-acp reports a shell command's output only in `raw_output`
    // (`{ formatted_output, exit_code }`); its `content` is an embedded
    // terminal block we drop. The output must still fill the card body.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("c1", "ls -la")
                .kind(ToolKind::Execute)
                .content(vec![ToolCallContent::Terminal(Terminal::new("term-1"))]),
        ),
    );
    // The in-progress insert carries no renderable text yet.
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    assert!(tc.output.is_empty());

    apply_update(
        &mut items,
        &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "c1",
            ToolCallUpdateFields::new()
                .status(ToolCallStatus::Completed)
                .raw_output(serde_json::json!({
                    "formatted_output": "total 0\ndrwxr-xr-x  2 me  staff",
                    "exit_code": 0,
                })),
        )),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    // Regression anchor: exit-status recovery must not change output-block
    // recovery, which already worked before this field existed.
    assert_eq!(
        tc.output,
        vec![ToolOutputBlock::RawText {
            text: "total 0\ndrwxr-xr-x  2 me  staff".to_string(),
            truncated_from: None,
        }]
    );
    assert_eq!(
        tc.exit,
        Some(CommandExit {
            code: Some(0),
            signal: None
        })
    );
}

#[test]
fn successful_edit_drops_raw_output_confirmation() {
    // Mirrors a captured claude-agent-acp wire session: the initial insert
    // carries only the diff, and completion carries no `content` at all —
    // just a self-referential `rawOutput` confirmation string with nothing
    // the diff and the "Done" badge don't already say.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("c1", "Edit /tmp/x.txt")
                .kind(ToolKind::Edit)
                .content(vec![ToolCallContent::Diff(Diff::new("/tmp/x.txt", "hi\n"))]),
        ),
    );
    apply_update(
        &mut items,
        &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "c1",
            ToolCallUpdateFields::new()
                .status(ToolCallStatus::Completed)
                .raw_output(serde_json::json!(
                    "The file /tmp/x.txt has been updated successfully. \
                     (file state is current in your context — no need to Read it back)"
                )),
        )),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    assert!(tc.output.is_empty());
    assert_eq!(tc.diffs.len(), 1);
}

#[test]
fn failed_edit_keeps_its_error_output() {
    // A rejected/failed edit reports the real reason via `content`, which
    // has no diff-shaped substitute — must survive the success-only strip.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("c1", "Edit /tmp/x.txt")
                .kind(ToolKind::Edit)
                .content(vec![ToolCallContent::Diff(Diff::new("/tmp/x.txt", "hi\n"))]),
        ),
    );
    apply_update(
        &mut items,
        &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "c1",
            ToolCallUpdateFields::new()
                .status(ToolCallStatus::Failed)
                .content(vec![ToolCallContent::Content(Content::new(
                    ContentBlock::Text(TextContent::new("The user rejected this edit.")),
                ))]),
        )),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    assert_eq!(
        tc.output,
        vec![ToolOutputBlock::Text {
            text: "The user rejected this edit.".to_string(),
            truncated_from: None,
        }]
    );
}

#[test]
fn terminal_sideband_output_fills_the_card() {
    // claude-agent-acp, once `_meta.terminal_output` is advertised, replaces
    // a Bash result's content with a bare terminal handle and ships the
    // captured bytes on `_meta`. They must land as raw (unfenced) text.
    let mut items = Vec::new();
    let effect = apply_update(
        &mut items,
        &terminal_result(Some(serde_json::json!({
            "terminal_output": { "terminal_id": "c1", "data": "total 0\ndrwxr-xr-x  2 me" }
        }))),
    );
    assert_eq!(
        only_tool_call(&items).output,
        vec![ToolOutputBlock::RawText {
            text: "total 0\ndrwxr-xr-x  2 me".to_string(),
            truncated_from: None,
        }]
    );
    assert!(
        !effect.dropped_terminal_output,
        "nothing was dropped — the sideband carried the output"
    );
}

#[test]
fn terminal_block_without_a_sideband_is_dropped_and_flagged() {
    let mut items = Vec::new();
    let effect = apply_update(&mut items, &terminal_result(None));
    assert!(
        only_tool_call(&items).output.is_empty(),
        "with no channel to read, the terminal block renders nothing"
    );
    assert!(
        effect.dropped_terminal_output,
        "the host must be told the card was left empty"
    );
}

#[test]
fn a_reshaped_sideband_payload_is_dropped_and_flagged_not_panicked() {
    // The `_meta` shape is derived from adapter source, not a stable
    // contract: a renamed inner key degrades to the pre-existing drop.
    let mut items = Vec::new();
    let effect = apply_update(
        &mut items,
        &terminal_result(Some(
            serde_json::json!({ "terminal_output": { "output": "renamed" } }),
        )),
    );
    assert!(only_tool_call(&items).output.is_empty());
    assert!(effect.dropped_terminal_output);
}

#[test]
fn a_live_terminal_block_is_not_flagged_as_dropped() {
    // Every adapter's *first* Bash event is a content-less handle with no
    // output yet. Flagging that would warn on every healthy command.
    let mut items = Vec::new();
    let effect = apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("c1", "ls -la")
                .kind(ToolKind::Execute)
                .content(vec![ToolCallContent::Terminal(Terminal::new("c1"))]),
        ),
    );
    assert!(only_tool_call(&items).output.is_empty());
    assert!(
        !effect.dropped_terminal_output,
        "a still-running call hasn't lost anything yet"
    );
}

#[test]
fn terminal_sideband_output_arrives_on_a_tool_call_update() {
    // The real sequence: content-less handle on the insert, bytes on the
    // completion update's `_meta`.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(ToolCall::new("c1", "ls").kind(ToolKind::Execute)),
    );
    let effect = apply_update(
        &mut items,
        &SessionUpdate::ToolCallUpdate(
            ToolCallUpdate::new(
                "c1",
                ToolCallUpdateFields::new()
                    .status(ToolCallStatus::Completed)
                    .content(vec![ToolCallContent::Terminal(Terminal::new("c1"))]),
            )
            .meta(meta_map(serde_json::json!({
                "terminal_output": { "data": "hello" },
                "terminal_exit": { "exit_code": 0 },
            }))),
        ),
    );
    let tc = only_tool_call(&items);
    assert_eq!(
        tc.output,
        vec![ToolOutputBlock::RawText {
            text: "hello".to_string(),
            truncated_from: None,
        }]
    );
    assert_eq!(
        tc.exit,
        Some(CommandExit {
            code: Some(0),
            signal: None
        }),
        "the exit sideband keeps working alongside the output one"
    );
    assert!(!effect.dropped_terminal_output);
}

#[test]
fn the_captured_bash_turn_recovers_its_output_and_exit() {
    let mut items = Vec::new();
    let effects: Vec<UpdateEffect> = CAPTURED_BASH_TURN
        .iter()
        .map(|line| {
            let update: SessionUpdate =
                serde_json::from_str(line).expect("captured notification deserializes");
            apply_update(&mut items, &update)
        })
        .collect();

    let tc = only_tool_call(&items);
    // `---` is in the payload on purpose: routed through the markdown path it
    // would become a horizontal rule, so this asserts the bytes stay verbatim.
    assert_eq!(
        tc.output,
        vec![ToolOutputBlock::RawText {
            text: "total 24\n---\ndrwxr-xr-x@  5 woo  staff   160 Jul 31 14:17 app".to_string(),
            truncated_from: None,
        }],
        "the content-less third notification carries the bytes, and the \
         completion behind it must neither blank them nor refill from rawOutput"
    );
    assert_eq!(
        tc.exit,
        Some(CommandExit {
            code: Some(0),
            signal: None
        })
    );
    assert!(
        effects.iter().all(|e| !e.dropped_terminal_output),
        "a healthy command must not burn the once-per-pane warning"
    );
}

#[test]
fn a_present_but_empty_content_clears_a_body_an_earlier_update_filled() {
    // `content` is replace semantics (schema: "Replace the content
    // collection"), so an update that explicitly sends nothing renderable
    // must not leave stale output on screen. Only a bare terminal handle is
    // exempt — its bytes ride their own notification.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("c1", "run")
                .kind(ToolKind::Execute)
                .content(vec![ToolCallContent::Content(Content::new(
                    ContentBlock::Text(TextContent::new("stale")),
                ))]),
        ),
    );
    assert!(!only_tool_call(&items).output.is_empty());

    apply_update(
        &mut items,
        &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "c1",
            ToolCallUpdateFields::new().content(Vec::new()),
        )),
    );
    assert!(
        only_tool_call(&items).output.is_empty(),
        "an explicit empty content replaces, it does not preserve"
    );
}

#[test]
fn a_codex_shell_failure_badges_its_exit_whatever_kind_it_was_labelled() {
    // Wire-captured (`acp-wire-codex-acp.log`): codex labels a failing `ls`
    // as `Read` — it classifies by intent, not by mechanism — and the
    // completion update carries no `kind` at all. Gating the exit on
    // `Execute` therefore blanks the badge for every codex command that is
    // not literally a shell invocation.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(ToolCall::new("c1", "List files").kind(ToolKind::Read)),
    );
    apply_update(
        &mut items,
        &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "c1",
            ToolCallUpdateFields::new()
                .status(ToolCallStatus::Failed)
                .raw_output(serde_json::json!({
                    "formatted_output": "ls: /nope: No such file or directory\n",
                    "exit_code": 1,
                })),
        )),
    );
    assert_eq!(
        only_tool_call(&items).exit,
        Some(CommandExit {
            code: Some(1),
            signal: None
        }),
        "a shell result is identified by its channel shape, not the tool kind"
    );
}

#[test]
fn a_stray_exit_code_without_command_output_is_not_a_command_exit() {
    // `raw_output` also carries MCP and other free-form results. Only the
    // `{ formatted_output, exit_code }` pair is codex's command shape, so an
    // `exit_code` on its own must not badge a tool that never ran a shell.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("c1", "fetch")
                .kind(ToolKind::Fetch)
                .raw_output(serde_json::json!({ "exit_code": 1, "result": "ok" })),
        ),
    );
    assert_eq!(only_tool_call(&items).exit, None);
}

#[test]
fn a_later_empty_terminal_exit_does_not_erase_a_recorded_exit() {
    // `dist/tools.js` builds `terminal_exit` from `bashResult.return_code`;
    // when that is absent the key is dropped and only `signal: null` ships.
    // The renderer reads `{None, None}` as "no exit", so letting it through
    // would silently drop an `Exit 1` badge recorded a moment earlier.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(ToolCall::new("c1", "false").kind(ToolKind::Execute)),
    );
    apply_update(
        &mut items,
        &SessionUpdate::ToolCallUpdate(
            ToolCallUpdate::new("c1", ToolCallUpdateFields::new()).meta(meta_map(
                serde_json::json!({ "terminal_exit": { "exit_code": 1, "signal": null } }),
            )),
        ),
    );
    apply_update(
        &mut items,
        &SessionUpdate::ToolCallUpdate(
            ToolCallUpdate::new(
                "c1",
                ToolCallUpdateFields::new().status(ToolCallStatus::Failed),
            )
            .meta(meta_map(serde_json::json!({
                "terminal_exit": { "terminal_id": "c1", "signal": null }
            }))),
        ),
    );
    assert_eq!(
        only_tool_call(&items).exit,
        Some(CommandExit {
            code: Some(1),
            signal: None
        }),
        "an exit report with no usable field must not overwrite the recorded one"
    );
}

#[test]
fn oversized_sideband_output_is_capped_and_records_its_original_length() {
    let original_len = MAX_TOOL_OUTPUT_TEXT_BYTES + 4096;
    let huge = "x".repeat(original_len);
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &terminal_result(Some(
            serde_json::json!({ "terminal_output": { "data": huge } }),
        )),
    );
    let [
        ToolOutputBlock::RawText {
            text,
            truncated_from,
        },
    ] = only_tool_call(&items).output.as_slice()
    else {
        panic!("expected one raw text block");
    };
    assert_eq!(text.len(), MAX_TOOL_OUTPUT_TEXT_BYTES);
    assert_eq!(
        *truncated_from,
        Some(original_len),
        "the sideband reuses the shared cap and its truncation marker"
    );
}

#[test]
fn a_silent_command_adds_no_empty_block_and_is_not_flagged() {
    let mut items = Vec::new();
    let effect = apply_update(
        &mut items,
        &terminal_result(Some(
            serde_json::json!({ "terminal_output": { "data": "  \n" } }),
        )),
    );
    assert!(only_tool_call(&items).output.is_empty());
    assert!(
        !effect.dropped_terminal_output,
        "the sideband reported an empty command — nothing was lost"
    );
}

#[test]
fn image_output_keeps_the_content_path_and_ignores_the_sideband() {
    // The adapter routes image results through normal content blocks rather
    // than the terminal handle. The sideband must not touch that path even
    // when the update happens to carry a `terminal_output` meta.
    let mut items = Vec::new();
    let effect = apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("c1", "screenshot")
                .kind(ToolKind::Execute)
                .status(ToolCallStatus::Completed)
                .content(vec![ToolCallContent::Content(Content::new(
                    ContentBlock::Image(ImageContent::new("BASE64", "image/png")),
                ))])
                .meta(meta_map(
                    serde_json::json!({ "terminal_output": { "data": "leaked" } }),
                )),
        ),
    );
    assert_eq!(
        only_tool_call(&items).output,
        vec![ToolOutputBlock::Image {
            data: "BASE64".to_string(),
            mime: "image/png".to_string(),
        }]
    );
    assert!(!effect.dropped_terminal_output);
}

#[test]
fn codex_terminal_then_raw_output_is_never_flagged_as_dropped() {
    // Regression anchor: codex's shell path (terminal handle on the insert,
    // output in `raw_output` on the completion) is unchanged by the
    // sideband, and must not produce a spurious drop warning either.
    let mut items = Vec::new();
    let insert = apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("c1", "ls -la")
                .kind(ToolKind::Execute)
                .content(vec![ToolCallContent::Terminal(Terminal::new("term-1"))]),
        ),
    );
    let complete = apply_update(
        &mut items,
        &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "c1",
            ToolCallUpdateFields::new()
                .status(ToolCallStatus::Completed)
                .content(vec![ToolCallContent::Terminal(Terminal::new("term-1"))])
                .raw_output(serde_json::json!({
                    "formatted_output": "total 0",
                    "exit_code": 0,
                })),
        )),
    );
    assert_eq!(
        only_tool_call(&items).output,
        vec![ToolOutputBlock::RawText {
            text: "total 0".to_string(),
            truncated_from: None,
        }],
        "codex still recovers its output through raw_output, verbatim"
    );
    assert!(!insert.dropped_terminal_output);
    assert!(
        !complete.dropped_terminal_output,
        "raw_output refilled the card, so nothing was lost"
    );
}

#[test]
fn command_exit_reads_codex_exit_code_through_the_full_pipeline() {
    // End-to-end through apply_update (DefaultAdapter), not just the
    // adapter unit test — proves the mapping wiring, not only the parse.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("c1", "run tests")
                .kind(ToolKind::Execute)
                .raw_output(serde_json::json!({ "formatted_output": "FAIL", "exit_code": 1 })),
        ),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    assert_eq!(
        tc.exit,
        Some(CommandExit {
            code: Some(1),
            signal: None
        })
    );
}

#[test]
fn command_exit_reads_claude_terminal_exit_meta_through_the_full_pipeline() {
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("c1", "run tests")
                .kind(ToolKind::Execute)
                .meta(meta_map(
                    serde_json::json!({ "terminal_exit": { "exit_code": 2, "signal": null } }),
                )),
        ),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    assert_eq!(
        tc.exit,
        Some(CommandExit {
            code: Some(2),
            signal: None
        })
    );
}

#[test]
fn command_exit_signal_only_has_no_code() {
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("c1", "run tests")
                .kind(ToolKind::Execute)
                .meta(meta_map(
                    serde_json::json!({ "terminal_exit": { "signal": "SIGTERM" } }),
                )),
        ),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    assert_eq!(
        tc.exit,
        Some(CommandExit {
            code: None,
            signal: Some("SIGTERM".to_string())
        })
    );
}

#[test]
fn command_exit_is_none_when_neither_channel_reports() {
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(ToolCall::new("c1", "run tests").kind(ToolKind::Execute)),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    assert_eq!(tc.exit, None);
}

#[test]
fn command_exit_is_none_for_a_non_execute_tool() {
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("t1", "Read src/main.rs")
                .kind(ToolKind::Read)
                .raw_input(serde_json::json!({"file_path": "src/main.rs"})),
        ),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    assert_eq!(tc.exit, None);
}

#[test]
fn command_exit_arrives_on_a_tool_call_update_without_blanking_on_a_later_status_only_update() {
    // The completion update carries the exit status; a later status-only
    // update (no raw_output, no meta) must not wipe it back to None.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(ToolCall::new("c1", "ls").kind(ToolKind::Execute)),
    );
    apply_update(
        &mut items,
        &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "c1",
            ToolCallUpdateFields::new()
                .status(ToolCallStatus::Completed)
                .raw_output(serde_json::json!({ "formatted_output": "ok\n", "exit_code": 0 })),
        )),
    );
    apply_update(
        &mut items,
        &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "c1",
            ToolCallUpdateFields::new().title("ls (renamed)"),
        )),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    assert_eq!(
        tc.exit,
        Some(CommandExit {
            code: Some(0),
            signal: None
        }),
        "a later update with no exit channel must not clear the recorded exit"
    );
}

#[test]
fn raw_output_does_not_duplicate_text_content() {
    // Claude-style adapters embed the result as a `content` text block *and*
    // repeat it in `raw_output`. The text block wins — no duplicate block.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("c1", "read")
                .content(vec![ToolCallContent::Content(Content::new(
                    ContentBlock::Text(TextContent::new("real output")),
                ))])
                .raw_output(serde_json::json!({ "formatted_output": "dup" })),
        ),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    assert_eq!(
        tc.output,
        vec![ToolOutputBlock::Text {
            text: "real output".to_string(),
            truncated_from: None,
        }]
    );
}

#[test]
fn raw_output_object_pretty_printed_as_raw_text_when_no_output_field() {
    // An MCP tool call returns structured `raw_output` (`{ result, error }`)
    // with no content — fall back to pretty-printed raw text so it renders in
    // the bounded output editor rather than the markdown path.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("m1", "mcp.server.tool")
                .kind(ToolKind::Other)
                .raw_output(serde_json::json!({ "result": { "ok": true }, "error": null })),
        ),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    let [ToolOutputBlock::RawText { text, .. }] = tc.output.as_slice() else {
        panic!("expected one raw text block, got {:?}", tc.output);
    };
    assert!(text.contains("\"result\""), "pretty JSON, got: {text}");
    assert!(text.contains("\"ok\": true"), "pretty JSON, got: {text}");
}

#[test]
fn raw_output_output_field_is_promoted_to_raw_text() {
    // Some adapters put the user-facing stream under `output` instead of
    // codex's `formatted_output`. The wrapper is transport, not content.
    let printed = "Chunk ID: abc\nOutput:\n# literal heading\n";
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("c1", "Read")
                .kind(ToolKind::Read)
                .raw_output(serde_json::json!({ "output": printed })),
        ),
    );
    assert_eq!(
        only_tool_call(&items).output,
        vec![ToolOutputBlock::RawText {
            text: printed.to_string(),
            truncated_from: None,
        }]
    );
}

#[test]
fn formatted_output_keeps_markdown_significant_bytes_verbatim() {
    // A command may print `#`, `---` or `**` — as shell bytes those are
    // literal, so the block must be `RawText` carrying the input unchanged.
    let printed = "# heading\n---\n**bold**\n";
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("c1", "cat notes.md")
                .kind(ToolKind::Execute)
                .raw_output(serde_json::json!({
                    "formatted_output": printed,
                    "exit_code": 0,
                })),
        ),
    );
    assert_eq!(
        only_tool_call(&items).output,
        vec![ToolOutputBlock::RawText {
            text: printed.to_string(),
            truncated_from: None,
        }]
    );
}

#[test]
fn blank_raw_output_adds_no_block() {
    // A whitespace-only `formatted_output` (a command that printed nothing)
    // must not add an empty output block.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("c1", "noop")
                .kind(ToolKind::Execute)
                .raw_output(serde_json::json!({ "formatted_output": "  \n ", "exit_code": 0 })),
        ),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    assert!(tc.output.is_empty());
}
