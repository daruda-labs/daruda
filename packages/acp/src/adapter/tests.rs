use super::*;
use serde_json::json;

fn meta(v: serde_json::Value) -> Option<Meta> {
    Some(v.as_object().unwrap().clone())
}

#[test]
fn default_reads_claude_parent_tool_id() {
    // The Claude adapter stamps the parent id here on a subagent's inner call.
    assert_eq!(
        DefaultAdapter.parent_tool_id(&meta(
            json!({"claudeCode": {"parentToolUseId": "toolu_parent"}})
        )),
        Some("toolu_parent".to_owned())
    );
}

#[test]
fn daruda_parent_tool_id_beats_the_claude_one() {
    // In native subagent mode both are present, and the claude id names a
    // launch call that mode never emits.
    assert_eq!(
        DefaultAdapter.parent_tool_id(&meta(json!({
            "daruda": {"parentToolId": "subagent:child-1"},
            "claudeCode": {"parentToolUseId": "toolu_suppressed"},
        }))),
        Some("subagent:child-1".to_owned())
    );
}

#[test]
fn default_parent_tool_id_is_none_without_the_key() {
    assert_eq!(DefaultAdapter.parent_tool_id(&None), None);
    assert_eq!(
        DefaultAdapter.parent_tool_id(&meta(json!({"claudeCode": {"toolName": "Bash"}}))),
        None
    );
}

#[test]
fn default_reads_claude_tool_name() {
    // The Claude adapter surfaces its own tool name here (confirmed on real
    // wire traffic: a Bash tool_call carries `_meta.claudeCode.toolName`).
    assert_eq!(
        DefaultAdapter.tool_name(&meta(json!({"claudeCode": {"toolName": "Bash"}}))),
        Some("Bash".to_owned())
    );
}

#[test]
fn default_tool_name_is_none_without_the_key() {
    assert_eq!(DefaultAdapter.tool_name(&None), None);
    assert_eq!(
        DefaultAdapter.tool_name(&meta(json!({"claudeCode": {"parentToolUseId": "p"}}))),
        None
    );
}

#[test]
fn default_tool_name_treats_blank_as_absent() {
    // A blank name would render as a blank header label with no fallback to
    // the kind, breaking the "never blank" guarantee — treat empty and
    // whitespace-only alike as absent.
    assert_eq!(
        DefaultAdapter.tool_name(&meta(json!({"claudeCode": {"toolName": ""}}))),
        None
    );
    assert_eq!(
        DefaultAdapter.tool_name(&meta(json!({"claudeCode": {"toolName": "   "}}))),
        None
    );
}

#[test]
fn codex_delegates_to_default_for_now() {
    let m = meta(json!({"claudeCode": {"parentToolUseId": "toolu_x", "toolName": "Bash"}}));
    assert_eq!(
        CodexAdapter.parent_tool_id(&m),
        DefaultAdapter.parent_tool_id(&m)
    );
    assert_eq!(CodexAdapter.tool_name(&m), DefaultAdapter.tool_name(&m));
}

#[test]
fn adapter_for_known_and_unknown_ids_extract_parent_id() {
    // codex and unknown ids both currently behave like Default for this hook.
    let m = meta(json!({"claudeCode": {"parentToolUseId": "toolu_y"}}));
    assert_eq!(
        adapter_for(None, "codex-acp").parent_tool_id(&m),
        Some("toolu_y".to_owned())
    );
    assert_eq!(
        adapter_for(None, "some-custom-agent").parent_tool_id(&m),
        Some("toolu_y".to_owned())
    );
}

#[test]
fn command_exit_reads_codex_raw_output_exit_code() {
    let raw = json!({"formatted_output": "ok", "exit_code": 1});
    assert_eq!(
        DefaultAdapter.command_exit(&Some(raw), &None),
        Some(CommandExit {
            code: Some(1),
            signal: None
        })
    );
}

#[test]
fn command_exit_reads_claude_terminal_exit_meta() {
    let m = meta(json!({"terminal_exit": {"exit_code": 2, "signal": null}}));
    assert_eq!(
        DefaultAdapter.command_exit(&None, &m),
        Some(CommandExit {
            code: Some(2),
            signal: None
        })
    );
}

#[test]
fn command_exit_signal_only_yields_no_code() {
    let m = meta(json!({"terminal_exit": {"signal": "SIGTERM"}}));
    assert_eq!(
        DefaultAdapter.command_exit(&None, &m),
        Some(CommandExit {
            code: None,
            signal: Some("SIGTERM".to_owned())
        })
    );
}

#[test]
fn command_exit_is_none_when_both_channels_absent() {
    assert_eq!(DefaultAdapter.command_exit(&None, &None), None);
    // Neither `exit_code` nor `terminal_exit` present in either channel.
    let raw = json!({"formatted_output": "ok"});
    let m = meta(json!({"claudeCode": {"toolName": "Bash"}}));
    assert_eq!(DefaultAdapter.command_exit(&Some(raw), &m), None);
}

#[test]
fn command_exit_zero_is_reported_as_some() {
    // Display judgment belongs to the renderer — the model carries a
    // reported zero exit code as-is, not as an absence.
    let raw = json!({"formatted_output": "", "exit_code": 0});
    assert_eq!(
        DefaultAdapter.command_exit(&Some(raw), &None),
        Some(CommandExit {
            code: Some(0),
            signal: None
        })
    );
}

#[test]
fn command_exit_prefers_terminal_exit_meta_over_raw_output() {
    let raw = json!({"exit_code": 9});
    let m = meta(json!({"terminal_exit": {"exit_code": 2}}));
    assert_eq!(
        DefaultAdapter.command_exit(&Some(raw), &m),
        Some(CommandExit {
            code: Some(2),
            signal: None
        })
    );
}

#[test]
fn command_exit_is_none_when_the_channel_reports_neither_field() {
    // `dist/tools.js` builds `terminal_exit` from `bashResult.return_code`,
    // which is `undefined` when the SDK omits it — `JSON.stringify` then
    // drops the key and ships `{terminal_id, signal: null}`. An all-`None`
    // exit reads as "absent" in the renderer, so the parser must say `None`
    // rather than hand back a value that overwrites a recorded exit.
    let m = meta(json!({"terminal_exit": {"terminal_id": "t1", "signal": null}}));
    assert_eq!(DefaultAdapter.command_exit(&None, &m), None);
    // Same for the codex channel with an explicitly null `exit_code`.
    assert_eq!(
        DefaultAdapter.command_exit(&Some(json!({"exit_code": null})), &None),
        None
    );
}

#[test]
fn command_exit_falls_back_to_raw_output_when_terminal_exit_is_empty() {
    let raw = json!({"formatted_output": "boom\n", "exit_code": 3});
    let m = meta(json!({"terminal_exit": {"signal": null}}));
    assert_eq!(
        DefaultAdapter.command_exit(&Some(raw), &m),
        Some(CommandExit {
            code: Some(3),
            signal: None
        })
    );
}

#[test]
fn sideband_output_reads_claude_terminal_output_data() {
    let m = meta(json!({"terminal_output": {"terminal_id": "toolu_1", "data": "hello\n"}}));
    assert_eq!(
        DefaultAdapter.sideband_output(&m),
        Some("hello\n".to_owned())
    );
}

#[test]
fn sideband_output_is_none_without_the_key() {
    // Absent `_meta`, an unrelated `_meta`, and a reshaped payload (the key
    // present but `data` renamed) must all degrade to `None`, not panic —
    // the shape is derived from adapter source, not a stable contract.
    assert_eq!(DefaultAdapter.sideband_output(&None), None);
    assert_eq!(
        DefaultAdapter.sideband_output(&meta(json!({"claudeCode": {"toolName": "Bash"}}))),
        None
    );
    assert_eq!(
        DefaultAdapter.sideband_output(&meta(json!({"terminal_output": {"output": "x"}}))),
        None
    );
    assert_eq!(
        DefaultAdapter.sideband_output(&meta(json!({"terminal_output": "x"}))),
        None
    );
}

#[test]
fn sideband_output_ignores_a_non_string_data() {
    assert_eq!(
        DefaultAdapter.sideband_output(&meta(json!({"terminal_output": {"data": 42}}))),
        None
    );
}

#[test]
fn codex_sideband_output_delegates_to_default() {
    // codex sends no `terminal_output` meta, so the shared shape read
    // yields `None` for it without any sender-identity branch.
    let m = meta(json!({"terminal_exit": {"exit_code": 0}}));
    assert_eq!(CodexAdapter.sideband_output(&m), None);
    assert_eq!(
        CodexAdapter.sideband_output(&m),
        DefaultAdapter.sideband_output(&m)
    );
}

#[test]
fn codex_command_exit_delegates_to_default() {
    let raw = json!({"exit_code": 1});
    assert_eq!(
        CodexAdapter.command_exit(&Some(raw.clone()), &None),
        DefaultAdapter.command_exit(&Some(raw), &None)
    );
}

mod selection_tests {
    use super::*;

    #[test]
    fn the_program_the_agent_reports_picks_the_strategy() {
        assert_eq!(
            adapter_id_for(Some("@agentclientprotocol/codex-acp"), "codex-acp"),
            AdapterId::Codex
        );
        assert_eq!(
            adapter_id_for(Some("@agentclientprotocol/claude-agent-acp"), "claude-acp"),
            AdapterId::Default
        );
    }

    /// The failure this selection exists to close: a user who registers
    /// codex-acp under a catalog id of their own still gets the codex strategy.
    #[test]
    fn a_catalog_id_of_the_users_own_still_reaches_the_right_strategy() {
        assert_eq!(
            adapter_id_for(Some("@agentclientprotocol/codex-acp"), "my-codex"),
            AdapterId::Codex
        );
    }

    /// `agent_info` is optional in the protocol, so the catalog id remains the
    /// fallback — an agent that reports nothing behaves exactly as before.
    #[test]
    fn an_agent_that_reports_nothing_falls_back_to_the_catalog_id() {
        assert_eq!(adapter_id_for(None, "codex-acp"), AdapterId::Codex);
        assert_eq!(adapter_id_for(None, "codex"), AdapterId::Codex);
        assert_eq!(
            adapter_id_for(None, "some-custom-agent"),
            AdapterId::Default
        );
    }

    /// The program on the other end of the pipe is the authority; the catalog
    /// id is only daruda's label for it.
    #[test]
    fn the_reported_program_outranks_the_catalog_id() {
        assert_eq!(
            adapter_id_for(Some("@agentclientprotocol/claude-agent-acp"), "codex-acp"),
            AdapterId::Default
        );
    }

    /// The scope is a publishing detail, so an unscoped report resolves the same.
    #[test]
    fn the_package_scope_is_not_part_of_the_identity() {
        assert_eq!(
            adapter_id_for(Some("codex-acp"), "custom"),
            AdapterId::Codex
        );
    }

    /// An unrecognised program is not evidence against the catalog id — a fork
    /// or wrapper reporting its own name still honours how it was registered.
    #[test]
    fn an_unrecognised_program_leaves_the_catalog_id_to_decide() {
        assert_eq!(
            adapter_id_for(Some("@someone/my-codex-wrapper"), "codex-acp"),
            AdapterId::Codex
        );
    }
}

mod phase_tests {
    use super::*;
    use serde_json::json;

    fn meta(v: serde_json::Value) -> Option<Meta> {
        Some(v.as_object().unwrap().clone())
    }

    /// Wire-captured (`acp-wire-codex-acp.log`): codex labels every message
    /// chunk, and across 3005 captured chunks it used exactly these two values.
    #[test]
    fn codex_reads_the_two_phases_it_actually_sends() {
        assert_eq!(
            CodexAdapter.message_phase(&meta(json!({"codex": {"phase": "commentary"}}))),
            MessagePhase::Commentary
        );
        assert_eq!(
            CodexAdapter.message_phase(&meta(json!({"codex": {"phase": "final_answer"}}))),
            MessagePhase::Answer
        );
    }

    /// A phase daruda has no opinion about must read as an ordinary answer, so
    /// a codex release that adds one cannot make messages vanish from a pane.
    #[test]
    fn an_unknown_phase_is_an_ordinary_answer() {
        assert_eq!(
            CodexAdapter.message_phase(&meta(json!({"codex": {"phase": "something_new"}}))),
            MessagePhase::Answer
        );
        assert_eq!(CodexAdapter.message_phase(&None), MessagePhase::Answer);
    }

    /// The vendor namespace stays with the named adapter. Default absorbing it
    /// would make the strategy choice decorative and let two agents' keys
    /// accumulate in one place.
    #[test]
    fn default_does_not_read_the_codex_namespace() {
        assert_eq!(
            DefaultAdapter.message_phase(&meta(json!({"codex": {"phase": "commentary"}}))),
            MessagePhase::Answer
        );
    }
}

mod rate_limit_tests {
    use super::*;
    use serde_json::json;

    fn meta(v: serde_json::Value) -> Option<Meta> {
        Some(v.as_object().unwrap().clone())
    }

    /// The `_meta` of a wire-captured claude-agent-acp `usage_update` sent
    /// while the five-hour window was spent and the reply billed as overage.
    fn billed_sample() -> Option<Meta> {
        meta(json!({
            "_claude/rateLimit": {
                "status": "rejected",
                "resetsAt": 1791340200,
                "rateLimitType": "five_hour",
                "overageStatus": "allowed",
                "overageResetsAt": 1793491200,
                "isUsingOverage": true,
                "overageInUse": true,
                "unifiedWindows": {
                    "five_hour": {"utilization": 1, "resetsAt": 1791340200},
                    "seven_day": {"utilization": 0.82, "resetsAt": 1791424800}
                }
            },
            "_claude/model": "claude-opus-5-5"
        }))
    }

    #[test]
    fn default_reads_the_captured_overage_report() {
        let view = DefaultAdapter
            .rate_limit(&billed_sample())
            .expect("the sample carries a rate limit");
        assert_eq!(
            view,
            RateLimitView {
                window: Some(RateLimitWindow::FiveHour),
                resets_at: Some(1_791_340_200),
                is_using_overage: true,
            }
        );
        assert!(view.bills_overage());
    }

    #[test]
    fn an_absent_or_non_object_report_is_none() {
        assert_eq!(DefaultAdapter.rate_limit(&None), None);
        assert_eq!(DefaultAdapter.rate_limit(&meta(json!({}))), None);
        assert_eq!(
            DefaultAdapter.rate_limit(&meta(json!({"_claude/rateLimit": "rejected"}))),
            None
        );
        assert_eq!(
            DefaultAdapter.rate_limit(&meta(
                json!({"_claude/rateLimit": {"isUsingOverage": true}})
            )),
            None,
            "without a status the report names no standing"
        );
    }

    #[test]
    fn a_mistyped_optional_field_reads_as_absent() {
        let view = DefaultAdapter
            .rate_limit(&meta(json!({"_claude/rateLimit": {
                "status": "allowed_warning",
                "resetsAt": "soon",
                "rateLimitType": 7,
                "overageStatus": false,
                "isUsingOverage": "yes"
            }})))
            .expect("status alone is a report");
        assert_eq!(
            view,
            RateLimitView {
                window: None,
                resets_at: None,
                is_using_overage: false,
            }
        );
        assert!(!view.bills_overage());
    }

    #[test]
    fn an_absent_overage_flag_does_not_bill() {
        let view = DefaultAdapter
            .rate_limit(&meta(json!({"_claude/rateLimit": {"status": "rejected"}})))
            .unwrap();
        assert!(!view.bills_overage());
    }

    #[test]
    fn codex_reports_no_rate_limit() {
        assert_eq!(CodexAdapter.rate_limit(&billed_sample()), None);
    }
}
