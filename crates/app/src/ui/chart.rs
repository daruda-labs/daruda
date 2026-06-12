//! Bar-chart wrapper over `gpui_component::chart::BarChart`.
//!
//! `BarChart<T, X, Y>` is a `Plot` — it canvas-renders its own axis,
//! grid, and bars. It is not `Sizable` and has no daruda default to
//! inject, so this is a plain re-export (shape C). Note it lays out as
//! `Size::full()`, so the call site must wrap it in a fixed-height,
//! full-width container.

pub use gpui_component::chart::BarChart;
