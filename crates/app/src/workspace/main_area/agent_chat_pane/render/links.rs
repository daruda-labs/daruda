//! Link handling for rendered agent-chat Markdown.

use gpui::{AnyWindowHandle, App, Window};

use crate::window_registry::WindowRegistry;
use crate::workspace::main_area::pane_tree::PaneId;

#[derive(Clone, Copy)]
pub(super) struct AgentChatMarkdownLinks {
    pane_id: PaneId,
    window_handle: AnyWindowHandle,
}

impl AgentChatMarkdownLinks {
    pub(super) fn new(pane_id: PaneId, window_handle: AnyWindowHandle) -> Self {
        Self {
            pane_id,
            window_handle,
        }
    }

    pub(super) fn handler(self) -> impl Fn(&str, &mut Window, &mut App) -> bool + Clone + 'static {
        move |url, window, cx| {
            let Some(ws) = WindowRegistry::workspace_for_window(self.window_handle, cx)
                .and_then(|ws| ws.upgrade())
            else {
                return false;
            };
            ws.update(cx, |ws, cx| {
                ws.open_agent_chat_markdown_file_link(self.pane_id, url, window, cx)
            })
        }
    }
}
