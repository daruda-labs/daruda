//! Geometry shared by text fields, selects and read-only field values.

use crate::theme;
use gpui::{Styled, px};

#[derive(Clone, Copy)]
pub(crate) struct FieldGeometry {
    pub height: f32,
    pub pad_y: f32,
}

impl FieldGeometry {
    pub const STANDARD: Self = Self {
        height: theme::INPUT_STANDARD_HEIGHT,
        pad_y: theme::INPUT_STANDARD_PAD_Y,
    };
    pub const FORM: Self = Self {
        height: theme::INPUT_FORM_HEIGHT,
        pad_y: theme::INPUT_FORM_PAD_Y,
    };

    pub fn apply<E: Styled>(self, element: E) -> E {
        element
            .px(px(theme::INPUT_PAD_X))
            .py(px(self.pad_y))
            .text_size(px(theme::FONT_SIZE_MD))
            .line_height(px(theme::INPUT_LINE_HEIGHT))
    }
}
