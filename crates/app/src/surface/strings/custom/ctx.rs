use crate::surface::strings as s;

/// Named for the platform's own file manager — the one `reveal_path` opens.
pub(crate) fn reveal_in_file_manager() -> String {
    if cfg!(target_os = "macos") {
        s::ctx::reveal_in_finder()
    } else if cfg!(windows) {
        s::ctx::reveal_in_file_explorer()
    } else {
        rust_i18n::t!("ctx.reveal_in_file_manager").into_owned()
    }
}
