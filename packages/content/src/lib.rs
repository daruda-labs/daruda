//! Content processing the file viewer and the agent chat share: decoding
//! raster images, rendering mermaid through merman and resvg to BGRA, and
//! turning a diff into highlighted rows.
//!
//! GPUI-free and app-free. Colours arrive as a [`MermaidPalette`] of hex
//! strings; building that palette from a theme and wrapping a
//! [`visual::RasterImage`] for GPUI stay with the host.

pub mod diff;
mod mermaid_contrast;
mod mermaid_host_theme;
mod mermaid_label_geometry;
mod mermaid_label_stroke;
mod mermaid_node_contrast;
mod mermaid_text_measurer;
pub mod syntax;
pub mod visual;

/// Plain-data palette a diagram is themed with, resolved by the host at its
/// GPUI boundary so background-thread code never touches a GPUI colour. The
/// field set mirrors merman's `HostThemeRoles` — a diagram-type-agnostic role
/// palette — so every diagram kind (flowchart, sequence, pie, ...) picks up
/// the host's surface/text/border colours instead of mermaid's own defaults.
#[derive(Clone)]
pub struct MermaidPalette {
    pub dark: bool,
    pub background: String,
    pub primary_color: String,
    pub primary_text_color: String,
    pub primary_border_color: String,
    pub line_color: String,
    pub secondary_color: String,
    pub surface_muted: String,
    pub cluster_background: String,
    pub note_background: String,
    pub note_text: String,
    pub activation_background: String,
    pub error: String,
    pub warning: String,
    pub success: String,
}

/// The palette the app's default dark theme resolves to, so the pipeline tests
/// render under the colours a user actually sees.
#[cfg(test)]
fn test_palette() -> MermaidPalette {
    MermaidPalette {
        dark: true,
        background: "#0b0c0e".into(),
        primary_color: "#414243".into(),
        primary_text_color: "#d5d7db".into(),
        primary_border_color: "#23252a".into(),
        line_color: "#8a8f98".into(),
        secondary_color: "#2e2e30".into(),
        surface_muted: "#2e2e30".into(),
        cluster_background: "#2e2e30".into(),
        note_background: "#201c15".into(),
        note_text: "#e0c185".into(),
        activation_background: "#2e2e30".into(),
        error: "#e67f7f".into(),
        warning: "#e0c185".into(),
        success: "#82caa2".into(),
    }
}

#[cfg(test)]
mod tests;
