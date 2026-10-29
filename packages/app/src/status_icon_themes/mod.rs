//! Installed status artwork, shared by dock rendering and Settings.
mod load;
mod model;
pub(crate) mod render;
use gpui::{App, Global};
pub(crate) use model::{Base, Catalog, Frame, State};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(crate) struct StatusIconThemes(pub Catalog);
impl Global for StatusIconThemes {}
pub(crate) fn init(cx: &mut App) {
    if !cx.has_global::<StatusIconThemes>() {
        cx.set_global(StatusIconThemes::default());
    }
}
pub(crate) fn current(cx: &App) -> &Catalog {
    static EMPTY: std::sync::OnceLock<Catalog> = std::sync::OnceLock::new();
    cx.try_global::<StatusIconThemes>()
        .map(|c| &c.0)
        .unwrap_or_else(|| EMPTY.get_or_init(Catalog::default))
}

/// Plugin artwork is read from user-scope installations, never marketplace previews.
pub(crate) fn sources(
    user: &Path,
    installs: &[daruda_agent::skills::plugins::PluginInstall],
) -> Vec<(String, PathBuf)> {
    let mut sources = vec![("user".into(), user.to_path_buf())];
    let mut plugins: Vec<_> = installs.iter().filter(|p| p.scope == "user").collect();
    plugins.sort_by(|a, b| (&a.id, &a.install_path).cmp(&(&b.id, &b.install_path)));
    sources.extend(plugins.into_iter().map(|p| {
        (
            format!("plugin/{}", p.id),
            p.install_path.join("daruda/status-icon-themes"),
        )
    }));
    sources
}

pub(crate) fn load(sources: &[(String, PathBuf)]) -> Catalog {
    let (catalog, errors) = load::catalog(sources);
    for error in errors {
        use daruda_store::observability::{
            error_report::{ErrorReport, ErrorSeverity},
            log_writer::LogWriter,
        };
        LogWriter::log(
            ErrorReport::new("Status icon theme could not be loaded")
                .severity(ErrorSeverity::Warning)
                .from_error(&error)
                .at(file!(), line!())
                .dedup(format!("status_icon_theme:{error}"))
                .build(),
        );
    }
    catalog
}

#[cfg(test)]
mod tests;
