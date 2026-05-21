use serde::{Deserialize, Serialize};

/// How file icons are rendered in the Files dock.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IconColorMode {
    /// Original material-icon-theme colors.
    Color,
    /// Monochrome: icon shape tinted with the row's text color.
    #[default]
    Monochrome,
}

/// Dock (left dock) startup configuration.
///
/// Defaults mirror `daruda_terminal::ux::theme::DOCK_LEFT_*`; duplicated
/// here so `daruda_config` stays GPUI-free. If the theme constants move,
/// update `LEFT_MIN_W` / `LEFT_MAX_W` / `Default::default` in lockstep.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct LeftDockConfig {
    /// Left dock width in pixels on fresh launch. Clamped to
    /// `[LEFT_MIN_W, LEFT_MAX_W]` at load time.
    pub left_default_width: f32,
    /// When true, the left dock starts closed on a new project.
    /// A saved `ProjectState` overrides this on subsequent launches.
    pub left_collapsed_by_default: bool,
    /// Show files whose name starts with `.` in the Files view. Sort
    /// rules push them to the end of their group regardless. Toggled
    /// at runtime via `FilesToggleHidden` (`Cmd+Shift+.`).
    pub files_show_hidden: bool,
    /// Use the lane's `.gitignore` + `.git/info/exclude` to grey
    /// out ignored entries in the Files view. Disable when working
    /// outside a git repository or when ignore rules are noisy.
    pub files_use_gitignore: bool,
    /// Icon color mode for the Files view.
    /// `color` renders material-icon-theme icons with their original colors.
    /// `monochrome` tints the icon shape with the row's text color.
    pub file_icon_color_mode: IconColorMode,
}

const LEFT_MIN_W: f32 = 150.0;
const LEFT_MAX_W: f32 = 400.0;
const LEFT_DEFAULT_W: f32 = 220.0;

impl Default for LeftDockConfig {
    fn default() -> Self {
        Self {
            left_default_width: LEFT_DEFAULT_W,
            left_collapsed_by_default: true,
            files_show_hidden: true,
            files_use_gitignore: true,
            file_icon_color_mode: IconColorMode::Monochrome,
        }
    }
}

impl LeftDockConfig {
    /// Clamp numeric fields to their valid ranges.
    pub fn clamp(&mut self) {
        self.left_default_width = self.left_default_width.clamp(LEFT_MIN_W, LEFT_MAX_W);
    }
}
