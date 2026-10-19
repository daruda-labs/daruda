mod render;

use daruda_store::project::ProjectId;
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, Subscription, WeakEntity, Window,
};

use crate::surface::strings as s;
use crate::ui::{InputEvent, InputState, WindowExt as _};
use crate::workspace::{ModalView, Workspace};

pub(super) struct AliasModal {
    input: Entity<InputState>,
    workspace: WeakEntity<Workspace>,
    project: ProjectId,
    _input_subscription: Subscription,
}

impl AliasModal {
    pub(super) fn new(
        workspace: WeakEntity<Workspace>,
        project: ProjectId,
        initial: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(s::project::alias_placeholder())
                .default_value(initial)
                .validate(|value, _| super::preferences::valid_alias(value))
        });
        let input_subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            this.on_input(event, window, cx);
        });
        Self {
            input,
            workspace,
            project,
            _input_subscription: input_subscription,
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value().to_string();
        if !super::preferences::valid_alias(&value) {
            return;
        }
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |workspace, cx| {
                workspace.set_project_display_alias(self.project, Some(value), cx);
            });
        }
        window.close_dialog(cx);
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
    }

    fn on_input(&mut self, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(event, InputEvent::PressEnter { .. }) {
            self.submit(window, cx);
        }
    }
}

impl Focusable for AliasModal {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl ModalView for AliasModal {}

#[cfg(test)]
mod tests {
    use gpui::AppContext as _;

    #[gpui::test]
    async fn enter_saves_the_alias_without_changing_the_active_pane(cx: &mut gpui::TestAppContext) {
        let root = std::env::temp_dir().join("daruda-project-alias-enter");
        let (window, workspace) = crate::workspace::tests::build_workspace_with(
            cx,
            &daruda_config::Config::default(),
            Some(daruda_store::project::Project::from_path(root)),
        );
        let before = cx
            .update_window(window.into(), |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    let before = (workspace.active, workspace.active_runtime().focused_pane_id);
                    workspace.open_project_alias_editor(workspace.active.project, window, cx);
                    before
                })
            })
            .unwrap();
        let vcx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        vcx.simulate_keystrokes("d a enter");
        vcx.run_until_parked();
        workspace.read_with(vcx, |workspace, _| {
            assert_eq!(
                workspace.active_project().unwrap().display_alias.as_deref(),
                Some("da")
            );
            assert_eq!(
                (workspace.active, workspace.active_runtime().focused_pane_id),
                before
            );
        });
    }
}
