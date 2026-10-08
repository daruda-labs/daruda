//! Prepare external opens on the foreground and execute them off-thread.
//!
//! Tests record requests per GPUI app and replace native execution entirely.

use std::path::PathBuf;

#[derive(Clone)]
pub(in crate::workspace) struct Request {
    pub path: PathBuf,
    pub preset: Option<&'static daruda_config::ExternalEditorPreset>,
}

impl Request {
    pub(super) fn run(self) -> (PathBuf, std::io::Result<()>) {
        #[cfg(not(test))]
        let result = super::launcher::open_with_preset(&self.path, self.preset);
        #[cfg(test)]
        let result = Ok(());
        (self.path, result)
    }
}

pub(super) fn prepare(
    path: PathBuf,
    preset: Option<&'static daruda_config::ExternalEditorPreset>,
    _cx: &mut gpui::App,
) -> Request {
    let request = Request { path, preset };
    #[cfg(test)]
    _cx.default_global::<Requests>().0.push(request.clone());
    request
}

#[cfg(test)]
#[derive(Default)]
struct Requests(Vec<Request>);

#[cfg(test)]
impl gpui::Global for Requests {}

#[cfg(test)]
pub(super) fn take_requests(cx: &mut gpui::App) -> Vec<Request> {
    std::mem::take(&mut cx.default_global::<Requests>().0)
}
