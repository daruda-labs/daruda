use crate::surface::strings as s;

/// Body of that prompt: the lead-in, then one line per running pane.
pub(crate) fn close_running_detail(titles: &[&str]) -> String {
    let mut out = rust_i18n::t!("modal.close_running_detail").into_owned();
    for title in titles {
        out.push('\n');
        out.push_str(&std::borrow::Cow::<str>::Owned(
            s::modal::close_running_line(*title),
        ));
    }
    out
}

pub(crate) fn delete_panel_tab_modal_body(name: &str, widget_count: usize) -> String {
    match widget_count {
        0 => s::modal::delete_panel_tab_body_empty(name),
        1 => s::modal::delete_panel_tab_body_one(name),
        count => std::borrow::Cow::<str>::Owned(s::modal::delete_panel_tab_body_many(name, count))
            .into_owned(),
    }
}
