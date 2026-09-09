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
    /// `/daruda <text>` — the only variant that costs an LLM turn, and the
    /// only one this parser does not read past.
    ///
    /// Not *unexamined*, though: the payload still reaches the pane's own
    /// slash classifier, so `/daruda /clear` resets the orchestrator's session
    /// instead of asking it anything. That is reported back as
    /// `AskDisposition::HandledLocally` rather than as an answer on its way.
    Ask {
        text: String,
    },
}

/// What the executor runs. Every target is concrete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResolvedCommand {
    List,
    Say {
        target: PaneRef,
        text: String,
    },
    Stop {
        target: PaneRef,
    },
    Flow(FlowCommand),
    Brief,
    /// Carries no target: the orchestrator names itself, so there is no
    /// ordinal for an adapter to resolve. Resolution is the identity, and the
    /// executor is what knows how to reach it.
    Ask {
        text: String,
    },
    /// Every worktree, including ones with no agent chat in them — the
    /// listing `/list` answers with is chat-scoped and cannot name one.
    LaneList,
}

/// A command that cannot be answered in one turn of the event loop.
///
/// Separate from [`ResolvedCommand`] rather than two more variants of it,
/// because the difference is not cosmetic: each of these has to wait — for
/// the user to tap an approval card, and for `git worktree add` on the
/// background executor — so the executor hands back a channel instead of an
/// outcome. Folding them in would leave `run` with two arms it could never
/// answer, which is the unreachable state this split removes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GatedCommand {
    /// Create a worktree and open a chat in it.
    ///
    /// Window-qualified: `ProjectId` is monotonic *per workspace*, so the id
    /// alone names a project in every open window (see
    /// [`crate::control::result::LaneHandle`]).
    LaneCreate {
        workspace: daruda_store::project::WorkspaceUuid,
        project: daruda_store::project::ProjectId,
        name: String,
        base_ref: Option<String>,
        agent: Option<String>,
        prompt: Option<String>,
    },
    /// Another agent chat in a worktree that already exists.
    ChatNew {
        lane: crate::control::result::LaneHandle,
        agent: Option<String>,
    },
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
const COMMANDS: [&str; 7] = ["list", "use", "say", "stop", "flow", "brief", "daruda"];

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
        "daruda" => {
            let rest = rest.ok_or(ParseError::MissingArgument { command: "daruda" })?;
            Ok(ControlCommand::Ask {
                text: unquote(rest).to_string(),
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
///
/// Only when nothing inside is quoted. `"a" is not "b"` also opens and closes
/// with a quote, and stripping there would hand on `a" is not "b` — a silently
/// corrupted, unbalanced string. Free-form prose is far likelier to hit that
/// than an ordinal-prefixed message, which is why the guard lives here rather
/// than at one call site.
fn unquote(text: &str) -> &str {
    text.strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .filter(|inner| !inner.contains('"'))
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
}
