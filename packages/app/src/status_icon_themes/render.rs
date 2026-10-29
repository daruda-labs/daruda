//! Paint immutable SVG bytes; discovery and file I/O never run during rendering.
use super::Frame;
use gpui::{Hsla, IntoElement, Pixels, Styled, canvas};

pub(crate) fn frame(frame: Frame, size: Pixels, color: Hsla) -> gpui::AnyElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, cx| {
            if let Err(error) = window.paint_svg(
                bounds,
                frame.key.clone().into(),
                Some(&frame.svg),
                Default::default(),
                color,
                cx,
            ) {
                use daruda_store::observability::{
                    error_report::ErrorReport, log_writer::LogWriter,
                };
                LogWriter::log(
                    ErrorReport::new("Status icon could not be painted")
                        .message(error.to_string())
                        .at(file!(), line!())
                        .dedup(frame.key.clone())
                        .build(),
                );
            }
        },
    )
    .size(size)
    .flex_none()
    .into_any_element()
}

#[cfg(test)]
mod tests {
    // Metal rendering, live selection and fallback are covered by
    // workspace::left_dock::projects::agent_badges::tests::visual.
}
