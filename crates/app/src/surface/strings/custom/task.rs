use crate::surface::strings as s;

/// One line of the "save before closing?" listing. Both the bullet and the
/// draft marker live in the key so a locale can move or drop either.
pub(crate) fn close_dirty_line(title: &str, is_draft: bool) -> String {
    if is_draft {
        s::task::close_dirty_line_draft(title)
    } else {
        rust_i18n::t!("task.close_dirty_line", title => title).into_owned()
    }
}
