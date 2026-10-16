//! Reusable UI size preference, independent of any panel or renderer.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UiSize {
    Small,
    #[default]
    Normal,
    Large,
}

impl UiSize {
    pub const ALL: [Self; 3] = [Self::Small, Self::Normal, Self::Large];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Normal => "normal",
            Self::Large => "large",
        }
    }

    pub fn from_value(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|size| size.as_str() == value)
    }
}

#[cfg(test)]
#[path = "tests/ui_size.rs"]
mod tests;
