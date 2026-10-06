use crate::lane::session_host::{SessionHostError, SessionHostField};
use crate::surface::strings as s;

/// The user-facing reason a host is refused, so every surface that refuses
/// one — a form, a connect, a flow — says the same thing about the same value.
pub(crate) fn host_error(err: SessionHostError) -> String {
    match err {
        SessionHostError::Empty(SessionHostField::Target) => s::session_host::err_target_empty(),
        SessionHostError::Empty(SessionHostField::Container) => {
            s::session_host::err_container_empty()
        }
        SessionHostError::Empty(SessionHostField::SessionPath) => {
            s::session_host::err_session_path_empty()
        }
        SessionHostError::Unsafe(SessionHostField::Target) => s::session_host::err_target_unsafe(),
        SessionHostError::Unsafe(SessionHostField::Container) => {
            s::session_host::err_container_unsafe()
        }
        SessionHostError::Unsafe(SessionHostField::SessionPath) => {
            s::session_host::err_session_path_unsafe()
        }
    }
}
