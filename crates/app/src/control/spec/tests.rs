use super::*;

#[test]
fn parses_bare_commands() {
    assert_eq!(parse("/list"), Ok(ControlCommand::List));
    assert_eq!(parse("/brief"), Ok(ControlCommand::Brief));
    assert_eq!(parse("/stop"), Ok(ControlCommand::Stop { target: None }));
}

#[test]
fn parses_use_select_and_clear() {
    assert_eq!(
        parse("/use 2"),
        Ok(ControlCommand::Use(UseTarget::Select(Ordinal(2))))
    );
    assert_eq!(parse("/use -"), Ok(ControlCommand::Use(UseTarget::Clear)));
}

#[test]
fn say_keeps_the_rest_of_the_line_verbatim() {
    assert_eq!(
        parse(r#"/say 3 add tests too"#),
        Ok(ControlCommand::Say {
            target: Ordinal(3),
            text: "add tests too".into(),
        })
    );
}

#[test]
fn say_strips_one_layer_of_wrapping_quotes() {
    assert_eq!(
        parse(r#"/say 3 "add tests too""#),
        Ok(ControlCommand::Say {
            target: Ordinal(3),
            text: "add tests too".into(),
        })
    );
}

#[test]
fn flow_without_name_lists() {
    assert_eq!(parse("/flow"), Ok(ControlCommand::Flow(FlowCommand::List)));
    assert_eq!(
        parse("/flow deploy"),
        Ok(ControlCommand::Flow(FlowCommand::Run {
            name: "deploy".into()
        }))
    );
}

#[test]
fn daruda_takes_the_rest_of_the_line_verbatim() {
    assert_eq!(
        parse("/daruda make me a pane"),
        Ok(ControlCommand::Ask {
            text: "make me a pane".into()
        })
    );
}

#[test]
fn daruda_strips_one_layer_of_wrapping_quotes() {
    assert_eq!(
        parse(r#"/daruda "make me a pane""#),
        Ok(ControlCommand::Ask {
            text: "make me a pane".into()
        })
    );
}

/// A prompt that merely opens and closes with a quote is not a quoted
/// prompt — stripping there would hand on an unbalanced string.
#[test]
fn daruda_keeps_quotes_that_are_part_of_the_prompt() {
    assert_eq!(
        parse(r#"/daruda "foo" is not "bar""#),
        Ok(ControlCommand::Ask {
            text: r#""foo" is not "bar""#.into()
        })
    );
}

#[test]
fn daruda_without_text_reports_missing_argument() {
    assert_eq!(
        parse("/daruda"),
        Err(ParseError::MissingArgument { command: "daruda" })
    );
}

/// The rest of the line is a prompt, not a nested command surface.
#[test]
fn daruda_does_not_parse_its_payload_as_a_command() {
    assert_eq!(
        parse("/daruda /list"),
        Ok(ControlCommand::Ask {
            text: "/list".into()
        })
    );
}

/// It is in the suggestion pool like every other name, so a typo points
/// at it instead of at nothing.
#[test]
fn a_typo_of_daruda_suggests_it() {
    assert_eq!(
        parse("/darudo"),
        Err(ParseError::Unknown {
            input: "darudo".into(),
            suggestion: Some("daruda"),
        })
    );
}

#[test]
fn non_command_text_is_not_a_command() {
    assert_eq!(parse("hello"), Err(ParseError::NotACommand));
    assert_eq!(parse(""), Err(ParseError::NotACommand));
}

#[test]
fn unknown_command_suggests_nearest() {
    assert_eq!(
        parse("/lst"),
        Err(ParseError::Unknown {
            input: "lst".into(),
            suggestion: Some("list"),
        })
    );
}

/// Telegram's own command menu sends `/list@thebot` in a group chat, and a
/// phone keyboard capitalises the first letter of a line. Neither should
/// read as an unknown command.
#[test]
fn a_command_survives_a_bot_suffix_and_a_capital() {
    assert_eq!(parse("/list@daruda_bot"), Ok(ControlCommand::List));
    assert_eq!(parse("/List"), Ok(ControlCommand::List));
    assert_eq!(
        parse("/Say@daruda_bot 2 go on"),
        Ok(ControlCommand::Say {
            target: Ordinal(2),
            text: "go on".into(),
        })
    );
}

/// The names BotFather is handed have to be the names `parse` accepts, or
/// a menu entry sends a command daruda answers with "unknown".
#[test]
fn the_botfather_registration_lists_every_command_name() {
    let registered: Vec<&str> = crate::surface::strings::control_botfather_commands()
        .lines()
        .filter_map(|line| line.split(" - ").next())
        .map(str::trim)
        .map(|name| {
            COMMANDS
                .iter()
                .copied()
                .find(|c| *c == name)
                .unwrap_or_else(|| panic!("{name} is registered but not a command"))
        })
        .collect();
    for command in COMMANDS {
        assert!(
            registered.contains(&command),
            "/{command} is a command but is not registered"
        );
    }
}

#[test]
fn a_distant_typo_suggests_nothing() {
    assert_eq!(
        parse("/deploy"),
        Err(ParseError::Unknown {
            input: "deploy".into(),
            suggestion: None,
        })
    );
}

#[test]
fn missing_argument_names_the_command() {
    assert_eq!(
        parse("/use"),
        Err(ParseError::MissingArgument { command: "use" })
    );
    assert_eq!(
        parse("/say 2"),
        Err(ParseError::MissingArgument { command: "say" })
    );
}

#[test]
fn non_numeric_ordinal_is_rejected() {
    assert_eq!(
        parse("/use abc"),
        Err(ParseError::BadOrdinal {
            input: "abc".into()
        })
    );
}
