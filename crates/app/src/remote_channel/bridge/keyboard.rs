/// Channel-neutral rows of labeled callback buttons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineKeyboard {
    pub rows: Vec<Vec<(String, String)>>,
}

impl InlineKeyboard {
    /// One horizontal row — what a permission prompt wants.
    pub fn single_row(buttons: Vec<(String, String)>) -> Self {
        Self {
            rows: vec![buttons],
        }
    }
}
