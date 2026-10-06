use crate::surface::strings as s;

pub(crate) fn scope_display(scope: crate::agent::mcp::McpScope) -> String {
    match scope {
        crate::agent::mcp::McpScope::User => s::common::section_user(),
        crate::agent::mcp::McpScope::Project => s::common::section_project(),
        crate::agent::mcp::McpScope::Local => s::common::section_local(),
    }
}

pub(crate) fn user_local_scope_display() -> String {
    format!(
        "{}/{}",
        s::common::section_user(),
        s::common::section_local()
    )
}
