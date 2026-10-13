use super::*;
use gpui::{TestAppContext, VisualTestContext};

#[gpui::test]
fn only_a_successful_save_opens_the_connection_receipt(cx: &mut TestAppContext) {
    use crate::agent::configuration::{Configuration, Provider};
    let (window, workspace) = crate::workspace::tests::build_workspace(cx);
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    for fail in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let config = std::sync::Arc::new(
            Configuration::resolve(
                Provider::Codex,
                Some(temp.path().into()),
                Some(temp.path().into()),
                None,
            )
            .unwrap(),
        );
        if fail {
            std::fs::create_dir(&config.user_file).unwrap();
        }
        let modal = vcx.update(|window, cx| {
            cx.new(|cx| {
                AddMcpServerModal::new(
                    workspace.downgrade(),
                    AddMcpInitial {
                        targets: Vec::new(),
                        config: Some(config.clone()),
                        default_scope: McpScope::User,
                        project_root: None,
                        existing_names: Vec::new(),
                    },
                    window,
                    cx,
                )
            })
        });
        vcx.update(|window, cx| {
            modal.update(cx, |modal, cx| {
                modal.apply_import(
                    crate::agent::mcp::import::parse_for(
                        "codex mcp add docs --url https://example.com/mcp --bearer-token-env-var DOCS_TOKEN",
                        Some(Provider::Codex),
                    )
                    .unwrap()
                    .remove(0),
                    window,
                    cx,
                );
                modal.submit(window, cx);
            })
        });
        vcx.run_until_parked();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        assert_eq!(vcx.debug_bounds("mcp-saved").is_some(), !fail);
        if fail {
            assert!(modal.read_with(&vcx, |modal, _| modal.error.is_some() && !modal.submitting));
        } else {
            let doc: toml::Value =
                toml::from_str(&std::fs::read_to_string(&config.user_file).unwrap()).unwrap();
            assert_eq!(
                doc["mcp_servers"]["docs"]["url"].as_str(),
                Some("https://example.com/mcp")
            );
            assert_eq!(
                doc["mcp_servers"]["docs"]["bearer_token_env_var"].as_str(),
                Some("DOCS_TOKEN")
            );
            assert!(doc["mcp_servers"]["docs"].get("http_headers").is_none());
            vcx.simulate_keystrokes("escape");
            vcx.run_until_parked();
            assert!(!vcx.has_pending_prompt());
            assert!(!vcx.update(|window, cx| window.has_active_dialog(cx)));
        }
    }
}

#[gpui::test]
fn registration_errors_keep_the_paste_and_manual_draft(cx: &mut TestAppContext) {
    let (window, workspace) = crate::workspace::tests::build_workspace(cx);
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.update(|window, cx| {
        let modal = cx.new(|cx| {
            AddMcpServerModal::new(
                workspace.downgrade(),
                AddMcpInitial {
                    targets: Vec::new(),
                    config: None,
                    default_scope: McpScope::User,
                    project_root: None,
                    existing_names: Vec::new(),
                },
                window,
                cx,
            )
        });
        modal.update(cx, |modal, cx| {
            let source = "claude mcp add docs --scope project -- npx server";
            modal
                .name_input
                .update(cx, |input, cx| input.set_value("keep", window, cx));
            modal
                .paste_input
                .update(cx, |input, cx| input.set_value(source, window, cx));
            modal.review_paste(window, cx);
            assert_eq!(
                modal.error.as_deref(),
                Some(strings::mcp::registration_error().as_str())
            );
            assert_eq!(modal.name_input.read(cx).value().as_str(), "keep");
            assert_eq!(modal.paste_input.read(cx).value().as_str(), source);
            assert!(modal.pasting);
        });
    });
}

#[gpui::test]
async fn quick_entry_reimport_requires_confirmation_and_cancel_keeps_edits(
    cx: &mut TestAppContext,
) {
    let (window, workspace) = crate::workspace::tests::build_workspace(cx);
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    let modal = vcx.update(|window, cx| {
        cx.new(|cx| {
            AddMcpServerModal::new(
                workspace.downgrade(),
                AddMcpInitial {
                    targets: Vec::new(),
                    config: None,
                    default_scope: McpScope::User,
                    project_root: None,
                    existing_names: Vec::new(),
                },
                window,
                cx,
            )
        })
    });
    vcx.update(|window, cx| {
        modal.update(cx, |modal, cx| {
            modal
                .name_input
                .update(cx, |input, cx| input.set_value("keep-my-edit", window, cx));
            modal.paste_input.update(cx, |input, cx| {
                input.set_value(
                    r#"{"mcpServers":{"imported":{"command":"server"}}}"#,
                    window,
                    cx,
                )
            });
            modal.review_paste(window, cx);
        })
    });
    assert!(vcx.has_pending_prompt());
    vcx.simulate_prompt_answer(&strings::common::btn_cancel());
    vcx.run_until_parked();
    assert_eq!(
        modal.read_with(&vcx, |modal, cx| modal
            .name_input
            .read(cx)
            .value()
            .to_string()),
        "keep-my-edit"
    );
    vcx.update(|window, cx| modal.update(cx, |modal, cx| modal.review_paste(window, cx)));
    assert!(vcx.has_pending_prompt());
    vcx.simulate_prompt_answer(&strings::common::btn_replace_fields());
    vcx.run_until_parked();
    assert_eq!(
        modal.read_with(&vcx, |modal, cx| modal
            .name_input
            .read(cx)
            .value()
            .to_string()),
        "imported"
    );
    assert!(!modal.read_with(&vcx, |modal, _| modal.pasting || modal.form.asking));
}

#[gpui::test]
async fn quick_entry_mcp_import_preserves_hidden_values_and_dirty_guard(cx: &mut TestAppContext) {
    let (window, workspace) = crate::workspace::tests::build_workspace(cx);
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.update(|window,cx| {
        let modal=cx.new(|cx|AddMcpServerModal::new(workspace.downgrade(),AddMcpInitial{targets: Vec::new(),config: None,
            default_scope:McpScope::User,project_root:None,existing_names:Vec::new()},window,cx));
        modal.update(cx,|modal,cx| {
            let source=r#"{"mcpServers":{"docs":{"url":"https://example.com/mcp","headers":{"Authorization":"Bearer secret","X-Multiline":"a\nb"},"env":{"MULTI":"a\nb"},"future":{"option":true}}}}"#;
            modal.paste_input.update(cx,|input,cx|input.set_value(source,window,cx));
            modal.review_paste(window,cx);
            assert!(!modal.pasting);
            assert!(!modal.advanced);
            assert_eq!(modal.transport_choice.selected(cx), Some(modal.transport));
            assert_eq!(modal.scope_choice.selected(cx), Some(modal.scope));
            let draft=modal.build_draft(cx).unwrap();
            assert_eq!(draft.headers["Authorization"],"Bearer secret");
            assert_eq!(draft.headers["X-Multiline"],"a\nb");
            assert_eq!(draft.env["MULTI"],"a\nb");
            assert_eq!(draft.extra["future"],serde_json::json!({"option":true}));
            assert!(modal.form.is_dirty(&modal.values(cx)));
            modal.auth.input.update(cx,|input,cx|input.set_value("changed",window,cx));
            assert_eq!(modal.build_draft(cx).unwrap().headers["Authorization"],"Bearer changed");
        });
    });
}

#[gpui::test]
async fn quick_entry_codex_keeps_native_auth_and_provider_fields(cx: &mut TestAppContext) {
    let (window, workspace) = crate::workspace::tests::build_workspace(cx);
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    let temp = tempfile::tempdir().unwrap();
    vcx.update(|window,cx| {
        let config = crate::agent::configuration::Configuration::resolve(crate::agent::configuration::Provider::Codex,Some(temp.path().into()),None,None).unwrap();
        let modal = cx.new(|cx| AddMcpServerModal::new(workspace.downgrade(),AddMcpInitial { config: Some(std::sync::Arc::new(config)),targets: Vec::new(),default_scope: McpScope::User, project_root: None, existing_names: Vec::new() },window,cx));
        modal.update(cx, |modal,cx| {
            modal.paste_input.update(cx,|input,cx| input.set_value("[mcp_servers.docs]\nurl='https://example.com/mcp'\nbearer_token_env_var='DOCS_TOKEN'\nstartup_timeout_sec=45\nenabled=false\n[ mcp_servers.docs.env_http_headers ]\nX-Key='API_KEY'\n",window,cx));
            modal.review_paste(window,cx);
            let draft = modal.build_draft(cx).unwrap();
            assert_eq!(draft.extra["bearer_token_env_var"],"DOCS_TOKEN");
            assert_eq!(draft.extra["startup_timeout_sec"],45);
            assert_eq!(draft.extra["env_http_headers"]["X-Key"],"API_KEY");
            assert!(draft.headers.is_empty());
            assert!(draft.disabled);
            assert!(!modal.scope_options.contains(&McpScope::Local));
        });
    });
}
