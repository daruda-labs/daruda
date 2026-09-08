//! The command vocabulary, at its two resolution stages.
//!
//! [`ControlCommand`] is what a *text* adapter parses: it names panes by
//! ordinal, which only that adapter can resolve. [`ResolvedCommand`] is what
//! the executor sees — every target is already a concrete [`PaneRef`]. An
//! adapter that speaks structured parameters (MCP, a future mobile client)
//! builds `ResolvedCommand` directly and never mints an ordinal.
//!
//! GPUI-free.

use crate::telegram::bridge::PaneRef;

/// A pane's position in the most recent `/list` output. Only the adapter that
/// produced that listing can resolve one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ordinal(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UseTarget {
    Select(Ordinal),
    Clear,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FlowCommand {
    List,
    Run { name: String },
}

/// Parsed from text. Targets are ordinals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ControlCommand {
    List,
    Use(UseTarget),
    Say {
        target: Ordinal,
        text: String,
    },
    /// `None` = the adapter's currently selected target.
    Stop {
        target: Option<Ordinal>,
    },
    Flow(FlowCommand),
    Brief,
}

/// What the executor runs. Every target is concrete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResolvedCommand {
    List,
    Say { target: PaneRef, text: String },
    Stop { target: PaneRef },
    Flow(FlowCommand),
    Brief,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ParseError {
    /// Not addressed to us — no leading slash, or empty. The adapter falls
    /// back to its plain-text routing.
    NotACommand,
    Unknown {
        input: String,
        suggestion: Option<&'static str>,
    },
    MissingArgument {
        command: &'static str,
    },
    BadOrdinal {
        input: String,
    },
}

/// Every command name we own. Also the suggestion pool for a typo.
const COMMANDS: [&str; 6] = ["list", "use", "say", "stop", "flow", "brief"];

/// The token `/use -` uses to drop the current selection.
const CLEAR_TOKEN: &str = "-";

/// How far a typo may be from a real command name before we stop guessing.
const SUGGESTION_MAX_DISTANCE: usize = 2;

pub(crate) fn parse(input: &str) -> Result<ControlCommand, ParseError> {
    let trimmed = input.trim();
    let Some(body) = trimmed.strip_prefix('/') else {
        return Err(ParseError::NotACommand);
    };
    let mut parts = body.splitn(2, char::is_whitespace);
    let Some(name) = parts.next().filter(|n| !n.is_empty()) else {
        return Err(ParseError::NotACommand);
    };
    // Telegram appends `@botusername` when a command is picked from the bot's
    // own menu, and the paired chat may be a group — nothing restricts pairing
    // to a private one. Lowercased because a phone keyboard capitalises the
    // first letter of a line by default.
    let name = name.split('@').next().unwrap_or(name).to_ascii_lowercase();
    let rest = parts.next().map(str::trim).filter(|r| !r.is_empty());

    match name.as_str() {
        "list" => Ok(ControlCommand::List),
        "brief" => Ok(ControlCommand::Brief),
        "use" => match rest {
            None => Err(ParseError::MissingArgument { command: "use" }),
            Some(CLEAR_TOKEN) => Ok(ControlCommand::Use(UseTarget::Clear)),
            Some(token) => Ok(ControlCommand::Use(UseTarget::Select(ordinal(token)?))),
        },
        "stop" => match rest {
            None => Ok(ControlCommand::Stop { target: None }),
            Some(token) => Ok(ControlCommand::Stop {
                target: Some(ordinal(token)?),
            }),
        },
        "say" => {
            let rest = rest.ok_or(ParseError::MissingArgument { command: "say" })?;
            let mut split = rest.splitn(2, char::is_whitespace);
            let token = split.next().unwrap_or_default();
            let target = ordinal(token)?;
            let text = split
                .next()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .ok_or(ParseError::MissingArgument { command: "say" })?;
            Ok(ControlCommand::Say {
                target,
                text: unquote(text).to_string(),
            })
        }
        "flow" => Ok(ControlCommand::Flow(match rest {
            None => FlowCommand::List,
            Some(name) => FlowCommand::Run {
                name: unquote(name).to_string(),
            },
        })),
        other => Err(ParseError::Unknown {
            input: other.to_string(),
            suggestion: nearest(other),
        }),
    }
}

fn ordinal(token: &str) -> Result<Ordinal, ParseError> {
    token
        .parse::<u32>()
        .map(Ordinal)
        .map_err(|_| ParseError::BadOrdinal {
            input: token.to_string(),
        })
}

/// Drop one symmetric layer of double quotes. Phone keyboards add them out of
/// habit; the rest of the line is taken verbatim either way.
fn unquote(text: &str) -> &str {
    text.strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .unwrap_or(text)
}

/// Closest command name within [`SUGGESTION_MAX_DISTANCE`], so `/lst` suggests
/// `list` but `/deploy` suggests nothing.
fn nearest(input: &str) -> Option<&'static str> {
    COMMANDS
        .iter()
        .map(|c| (*c, edit_distance(input, c)))
        .filter(|(_, d)| *d <= SUGGESTION_MAX_DISTANCE)
        .min_by_key(|(_, d)| *d)
        .map(|(c, _)| c)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b_chars: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut cur = vec![0usize; b_chars.len() + 1];
    for (i, ca) in a.chars().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b_chars.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b_chars.len()]
}

#[cfg(test)]
mod tests {
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
}
