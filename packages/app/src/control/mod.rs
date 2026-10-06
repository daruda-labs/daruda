//! External control surface: the command vocabulary daruda accepts from
//! outside the app, and what it answers with.
//!
//! Split three ways. `spec` and `result` are GPUI-free and pure — they are the
//! contract every adapter shares, owned by `daruda_control_types` and
//! re-exported here. `exec` is the app-level dispatcher that turns a resolved
//! command into a result, and is the only file here that touches GPUI.
//!
//! The core is stateless. Which pane an ordinal names, and which pane is
//! currently selected, are conversation context owned by the adapter — see
//! `crate::telegram::command`.

pub(crate) use daruda_control_types::agent_text;
pub(crate) mod approval;
pub(crate) mod ask;
pub(crate) mod exec;
pub(crate) mod guards;
pub(crate) mod mcp;
pub(crate) mod resolve;
pub(crate) use daruda_control_types::{result, spec};

#[cfg(test)]
mod tests {
    use super::spec;

    /// The names BotFather is handed have to be the names `parse` accepts, or
    /// a menu entry sends a command daruda answers with "unknown".
    #[test]
    fn the_botfather_registration_lists_every_command_name() {
        let registered: Vec<&str> = crate::surface::strings::control::botfather_commands()
            .lines()
            .filter_map(|line| line.split(" - ").next())
            .map(str::trim)
            .map(|name| {
                spec::COMMANDS
                    .iter()
                    .copied()
                    .find(|c| *c == name)
                    .unwrap_or_else(|| panic!("{name} is registered but not a command"))
            })
            .collect();
        for command in spec::COMMANDS {
            assert!(
                registered.contains(&command),
                "/{command} is a command but is not registered"
            );
        }
    }
}
