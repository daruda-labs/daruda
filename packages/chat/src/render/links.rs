//! Link handling for rendered agent-chat Markdown.

use daruda_content::link_target::LinkTarget;
use gpui::{App, Pixels, Point, WeakEntity, Window};

use crate::workspace::main_area::agent_chat_pane::view::{AgentChatEvent, AgentChatView};

/// The chat a rendered link belongs to. It classifies the link itself and
/// hands the host only what to open.
#[derive(Clone)]
pub(super) struct AgentChatMarkdownLinks {
    view: WeakEntity<AgentChatView>,
}

impl AgentChatMarkdownLinks {
    pub(super) fn new(view: WeakEntity<AgentChatView>) -> Self {
        Self { view }
    }

    /// `false` for a link the pane does not handle, which leaves it to the
    /// Markdown view's default opener — the answer is needed now, so the view
    /// classifies before it emits.
    pub(super) fn handler(self) -> impl Fn(&str, &mut Window, &mut App) -> bool + Clone + 'static {
        move |url, _window, cx| {
            let Some(view) = self.view.upgrade() else {
                return false;
            };
            let target = view.read(cx).classify_link(url);
            if target == LinkTarget::Opaque {
                return false;
            }
            view.update(cx, |_, cx| cx.emit(AgentChatEvent::OpenLink(target)));
            true
        }
    }

    /// The opener for a tool's resource-link URI — a file by definition, so
    /// it resolves as one even where the Markdown rules would read a word.
    pub(super) fn open_resource(&self, uri: &str, mime: Option<&str>, cx: &mut App) {
        let Some(view) = self.view.upgrade() else {
            return;
        };
        let target = view.read(cx).classify_resource(uri, mime);
        view.update(cx, |_, cx| cx.emit(AgentChatEvent::OpenLink(target)));
    }

    /// Record a right press on a resource link, so the pane menu that press
    /// opens classifies it as [`Self::open_resource`] would.
    pub(super) fn record_resource_right_click(
        &self,
        position: Point<Pixels>,
        uri: String,
        mime: Option<String>,
        cx: &mut App,
    ) {
        let Some(view) = self.view.upgrade() else {
            return;
        };
        view.update(cx, |_, cx| {
            cx.emit(AgentChatEvent::ResourceRightClicked {
                position,
                uri,
                mime,
            })
        });
    }
}
