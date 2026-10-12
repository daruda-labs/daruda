use super::*;
use gpui::{TestAppContext, VisualTestContext};

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
