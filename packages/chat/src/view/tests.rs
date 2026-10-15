use super::super::fold::{FoldContext, FoldKey};
use super::super::rows::RowKind;
use daruda_transcript::fold_mode::FoldPreset;

mod activity;
mod attachments;
mod card_folding;
mod compaction;
mod diff_layout;
mod output_layout;
mod overage;
mod pane_choices;
mod projection;
mod queue;
mod session;
mod stop;
mod task_signals;
mod transitions;

fn assistant_text_item(text: &str) -> daruda_acp::ChatItem {
    daruda_acp::ChatItem::AssistantText {
        text: text.to_string(),
        streaming: false,
        message_id: None,
        phase: Default::default(),
    }
}

fn tool_call(
    id: &str,
    status: daruda_acp::ToolStatusView,
    parent_tool_id: Option<&str>,
) -> daruda_acp::ChatItem {
    daruda_acp::ChatItem::ToolCall(daruda_acp::ToolCallItem {
        id: id.to_string(),
        title: format!("Tool {id}"),
        kind: daruda_acp::ToolKindView::Read,
        tool_name: None,
        status,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: None,
        locations: Vec::new(),
        parent_tool_id: parent_tool_id.map(str::to_string),
        exit: None,
    })
}

pub(super) fn permission_card(id: u64) -> daruda_acp::ChatItem {
    daruda_acp::ChatItem::Permission(daruda_acp::PermissionItem {
        id,
        tool_call_id: String::new(),
        tool_title: Some(format!("Write /tmp/{id}")),
        raw_input_summary: None,
        options: vec![
            daruda_acp::PermissionChoice {
                option_id: "allow_once".to_string(),
                name: "Allow".to_string(),
                kind: daruda_acp::PermissionKindView::AllowOnce,
            },
            daruda_acp::PermissionChoice {
                option_id: "reject_once".to_string(),
                name: "Reject".to_string(),
                kind: daruda_acp::PermissionKindView::RejectOnce,
            },
        ],
        resolved: None,
    })
}

/// Build a minimal, offline `AgentChatView` (no cwd → `Idle` status, no
/// adapter spawned) as its own window root, so the activity methods
/// (`&mut self` + `Instant`/`Duration`) can be driven directly:
/// `AgentChatView::new` only needs a `Context<Self>`, no host at all.
pub(crate) fn make_test_view(
    cx: &mut gpui::TestAppContext,
) -> gpui::WindowHandle<super::AgentChatView> {
    crate::test_support::init_gpui_component(cx);
    add_test_view(cx)
}

/// A view whose "save as agent default" goes through `writer`.
pub(crate) fn make_test_view_with_writer(
    cx: &mut gpui::TestAppContext,
    writer: super::AgentDefaultWriter,
) -> gpui::WindowHandle<super::AgentChatView> {
    crate::test_support::init_gpui_component(cx);
    add_test_view_with_writer(cx, writer)
}

/// A pane as its window's root, for tests that measure what it paints.
fn open_view(
    cx: &mut gpui::TestAppContext,
) -> (gpui::AnyWindowHandle, gpui::Entity<super::AgentChatView>) {
    let handle = make_test_view(cx);
    cx.run_until_parked();
    let view = handle.root(cx).expect("the view's window is open");
    (handle.into(), view)
}

fn add_test_view(cx: &mut gpui::TestAppContext) -> gpui::WindowHandle<super::AgentChatView> {
    add_test_view_with_writer(cx, std::rc::Rc::new(|_, _, _| Ok(())))
}

fn add_test_view_with_writer(
    cx: &mut gpui::TestAppContext,
    writer: super::AgentDefaultWriter,
) -> gpui::WindowHandle<super::AgentChatView> {
    cx.add_window(|window, cx| {
        super::AgentChatView::new(
            0,
            window.window_handle(),
            None,
            super::AgentSessionStatus::Idle,
            None,
            super::AgentIdentity {
                id: "claude".to_string(),
                name: "Claude".to_string(),
                icon: None,
            },
            None,
            super::super::transcript_defaults::TranscriptDefaults {
                tail: super::super::rows::tail::StepWindow::default(),
                content_width: super::ChatContentWidth::Reading,
                tool_summary: super::ToolSummaryStyle::Labeled,
                fold_mode: daruda_transcript::fold_mode::FoldMode::default(),
                filter: daruda_transcript::display_filter::DisplayFilter::default(),
            },
            daruda_config::file_viewer::DEFAULT_SYNTAX_THEME.to_owned(),
            writer,
            cx,
        )
    })
}

pub(super) fn queued(id: u64, text: &str) -> super::QueuedPrompt {
    super::QueuedPrompt {
        id: super::PromptId(id),
        text: text.to_string(),
        attachments: Vec::new(),
        origin: super::PromptOrigin::InApp,
        task: None,
    }
}

fn texts(prompts: &[super::QueuedPrompt]) -> Vec<String> {
    prompts.iter().map(|q| q.text.clone()).collect()
}
