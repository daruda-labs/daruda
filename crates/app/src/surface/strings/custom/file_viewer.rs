use crate::surface::strings as s;

/// Footer under a raw file cut at the size cap, which opens read-only.
pub(crate) fn truncated_read_only(max_bytes: usize) -> String {
    rust_i18n::t!(
        "file_viewer.truncated_read_only",
        size = format!("{} MB", max_bytes / (1024 * 1024))
    )
    .into_owned()
}

pub(crate) fn byte_truncated(shown: usize, max_bytes: usize, total_count: usize) -> String {
    let size = if max_bytes >= 1024 * 1024 {
        format!("{} MB", max_bytes / (1024 * 1024))
    } else {
        format!("{} KB", max_bytes / 1024)
    };
    if total_count > shown {
        std::borrow::Cow::<str>::Owned(s::file_viewer::byte_truncated_full(
            size,
            shown,
            total_count,
        ))
        .into_owned()
    } else {
        std::borrow::Cow::<str>::Owned(s::file_viewer::byte_truncated_short(size, shown))
            .into_owned()
    }
}
