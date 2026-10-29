//! Workspace effects for evidence reads and opening the selected saved file.

use super::*;

impl Workspace {
    pub(super) fn load_run_evidence(&mut self, id: FlowDetailId, cx: &mut Context<Self>) {
        let Some(run) = self.pages.flows.run_mut(id) else {
            return;
        };
        let Inspection::Ready(history) = &run.execution.inspection else {
            return;
        };
        let Some(request) = run.execution.inspector.begin_read(history, &run.dir) else {
            return;
        };
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || super::super::execution_evidence::read(request.path),
            move |ws, result, cx| ws.land_execution_evidence(id, request.epoch, result, cx),
        )
        .detach();
    }

    fn land_execution_evidence(
        &mut self,
        id: FlowDetailId,
        epoch: u64,
        result: super::super::execution_evidence::ReadEvidence,
        cx: &mut Context<Self>,
    ) {
        use super::super::execution_evidence::ReadEvidence;
        let (evidence, error) = match result {
            ReadEvidence::Missing => (Evidence::Missing, None),
            ReadEvidence::Ready { text, truncated } => (
                Evidence::Ready {
                    text: text.into(),
                    truncated,
                },
                None,
            ),
            ReadEvidence::Failed(error) => (Evidence::Failed(error.clone().into()), Some(error)),
        };
        let Some(run) = self.pages.flows.run_mut(id) else {
            return;
        };
        if !run.execution.inspector.accept(epoch, evidence) {
            return;
        }
        if let Some(error) = error {
            self.report_error(
                daruda_store::observability::error_report::ErrorReport::new(
                    crate::surface::strings::flow::run_evidence_failed(),
                )
                .message(error)
                .at(file!(), line!())
                .dedup("flow.evidence.read")
                .build(),
                cx,
            );
        }
        cx.notify();
    }

    pub(in crate::workspace) fn open_selected_execution_file(
        &mut self,
        id: FlowDetailId,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(run) = self.pages.flows.run(id) else {
            return;
        };
        let Inspection::Ready(history) = &run.execution.inspection else {
            return;
        };
        if let Some(path) = run.execution.inspector.selected_path(history, &run.dir) {
            self.open_linked_file(path, window, cx);
        }
    }
}

#[cfg(test)]
mod tests {}
