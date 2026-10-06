use crate::surface::strings as s;

/// A model choice's display label. The default-model entry carries a fixed
/// "Default (recommended)" name and names the model it resolves to only in its
/// description, so that model is shown instead.
pub(crate) fn agent_model_choice_label(
    value: &str,
    name: &str,
    description: Option<&str>,
) -> String {
    match daruda_acp::resolved_default_model(value, description) {
        Some(model) => s::agent_chat::config_default_model(model),
        None => name.to_string(),
    }
}

/// One category segment of a tool-group header, e.g. "3 files read". The group
/// is a run of adjacent calls, so several of these sit side by side when the
/// agent mixed kinds — which the wire logs show it does about half the time on
/// one agent, up to four categories in a single group.
///
/// A segment of one is common — a group of three mixed kinds is three ones — so
/// each category carries a singular form rather than printing "1 files read".
pub(crate) fn group_category(category: &str, count: usize) -> String {
    let one = count == 1;
    match (category, one) {
        ("read", true) => std::borrow::Cow::<str>::Owned(s::agent_chat::group_read_one()),
        ("read", false) => std::borrow::Cow::<str>::Owned(s::agent_chat::group_read(count)),
        ("edit", true) => std::borrow::Cow::<str>::Owned(s::agent_chat::group_edit_one()),
        ("edit", false) => std::borrow::Cow::<str>::Owned(s::agent_chat::group_edit(count)),
        ("search", true) => std::borrow::Cow::<str>::Owned(s::agent_chat::group_search_one()),
        ("search", false) => std::borrow::Cow::<str>::Owned(s::agent_chat::group_search(count)),
        ("run", true) => std::borrow::Cow::<str>::Owned(s::agent_chat::group_run_one()),
        ("run", false) => std::borrow::Cow::<str>::Owned(s::agent_chat::group_run(count)),
        ("delete", true) => std::borrow::Cow::<str>::Owned(s::agent_chat::group_delete_one()),
        ("delete", false) => std::borrow::Cow::<str>::Owned(s::agent_chat::group_delete(count)),
        ("fetch", true) => std::borrow::Cow::<str>::Owned(s::agent_chat::group_fetch_one()),
        ("fetch", false) => std::borrow::Cow::<str>::Owned(s::agent_chat::group_fetch(count)),
        ("mcp", true) => std::borrow::Cow::<str>::Owned(s::agent_chat::group_mcp_one()),
        ("mcp", false) => std::borrow::Cow::<str>::Owned(s::agent_chat::group_mcp(count)),
        ("agent", true) => std::borrow::Cow::<str>::Owned(s::agent_chat::group_agent_one()),
        ("agent", false) => std::borrow::Cow::<str>::Owned(s::agent_chat::group_agent(count)),
        (_, true) => std::borrow::Cow::<str>::Owned(s::agent_chat::group_other_one()),
        (_, false) => std::borrow::Cow::<str>::Owned(s::agent_chat::group_other(count)),
    }
    .into_owned()
}

/// Collapsed thinking-group header title, e.g. "3 thoughts". Every run earns a
/// header, so a count of one is reachable and carries its own form.
pub(crate) fn thinking_group_count(count: usize) -> String {
    if count == 1 {
        s::agent_chat::thinking_group_count_one()
    } else {
        rust_i18n::t!("agent_chat.thinking_group_count", count = count).into_owned()
    }
}

/// Collapsed label for the filter's per-run disclosure: how many blocks the
/// filter took out of this run. Singular handled explicitly. The unit is a
/// block rather than a row because a group the filter empties comes back as one
/// thing — hence "items", not "rows".
pub(crate) fn filtered_show(count: usize) -> String {
    if count == 1 {
        s::agent_chat::filtered_show_one()
    } else {
        rust_i18n::t!("agent_chat.filtered_show", count = count).into_owned()
    }
}

/// The tail window's boundary row while it is closed: `count` is how many
/// earlier steps opening it brings back.
pub(crate) fn tail_more_show(count: usize) -> String {
    if count == 1 {
        s::agent_chat::tail_more_show_one()
    } else {
        rust_i18n::t!("agent_chat.tail_more_show", count = count).into_owned()
    }
}

/// The same row while it is open. Names the window it collapses *back to* —
/// `count` is the steps kept, not the ones hidden — because a label built from
/// the hidden count reads as a description of the current state ("6 earlier
/// steps, hidden") exactly when the steps are on screen, which is the misread
/// that made the row's two states indistinguishable.
pub(crate) fn tail_more_collapse(count: usize) -> String {
    if count == 1 {
        s::agent_chat::tail_more_collapse_one()
    } else {
        rust_i18n::t!("agent_chat.tail_more_collapse", count = count).into_owned()
    }
}

/// The same boundary where the unit is a call rather than a step: inside a tool
/// group, and inside a subagent card, whose flattened children are one group of
/// calls. The group is itself the step the axis counted, so what the boundary
/// holds back are the calls it is made of. Shared by both hosts — a third one
/// wants this copy too, not a new key.
pub(crate) fn tail_more_show_calls(count: usize) -> String {
    if count == 1 {
        s::agent_chat::tail_more_show_calls_one()
    } else {
        rust_i18n::t!("agent_chat.tail_more_show_calls", count = count).into_owned()
    }
}

/// A call-unit boundary while it is open — either host. Names the kept calls,
/// for the same reason [`s::agent_chat::tail_more_collapse`] names the kept steps.
pub(crate) fn tail_more_collapse_calls(count: usize) -> String {
    if count == 1 {
        s::agent_chat::tail_more_collapse_calls_one()
    } else {
        rust_i18n::t!("agent_chat.tail_more_collapse_calls", count = count).into_owned()
    }
}

/// Marker shown below a tool-output text block that was capped before
/// reaching the render model (any text block's `truncated_from`),
/// e.g. "… (truncated, 1.2 MB total)". `original_bytes` is the untruncated
/// byte length.
pub(crate) fn tool_output_truncated(original_bytes: usize) -> String {
    let size = s::agent_chat::format_byte_size(original_bytes);
    rust_i18n::t!("agent_chat.tool_output_truncated", size = size).into_owned()
}

/// Descriptor label for a non-rendered binary tool-output block (audio, or an
/// embedded blob resource), e.g. "[audio/mp3 · 128 KB]". `mime` may be empty
/// (the source omitted it, an explicitly allowed case) — rendering it through
/// the `mime`-carrying key then would produce a stray leading space and
/// middot ("[ · 128 KB]"), so an empty `mime` renders through a dedicated
/// no-mime key instead, e.g. "[128 KB]".
pub(crate) fn tool_media_label(mime: &str, byte_len: usize) -> String {
    let size = s::agent_chat::format_byte_size(byte_len);
    if mime.is_empty() {
        s::agent_chat::tool_media_label_no_mime(size)
    } else {
        rust_i18n::t!("agent_chat.tool_media_label", mime = mime, size = size).into_owned()
    }
}

/// Human-readable byte size in KB/MB, e.g. "64 KB" / "1.2 MB".
pub(crate) fn format_byte_size(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= MB {
        format!("{:.1} MB", bytes / MB)
    } else {
        format!("{:.0} KB", bytes / KB)
    }
}
