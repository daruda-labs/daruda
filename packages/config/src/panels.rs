use serde::{Deserialize, Serialize};

/// Bottom-dock panel layout configuration.
///
/// Macro tiles render as a fixed-column grid; `grid_columns` controls
/// the column count. Clamped to `[GRID_COLUMNS_MIN, GRID_COLUMNS_MAX]`
/// at load time so a misconfigured value never produces a zero-column
/// or unbounded grid.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct PanelsConfig {
    /// Number of columns in the bottom-dock macro tile grid.
    pub grid_columns: u8,
}

const GRID_COLUMNS_MIN: u8 = 1;
const GRID_COLUMNS_MAX: u8 = 16;
const GRID_COLUMNS_DEFAULT: u8 = 5;

impl Default for PanelsConfig {
    fn default() -> Self {
        Self {
            grid_columns: GRID_COLUMNS_DEFAULT,
        }
    }
}

impl PanelsConfig {
    pub fn clamp(&mut self) {
        self.grid_columns = self.grid_columns.clamp(GRID_COLUMNS_MIN, GRID_COLUMNS_MAX);
    }
}
