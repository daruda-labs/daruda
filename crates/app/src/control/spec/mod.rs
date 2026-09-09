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

/// Parsed from text. Names no worktree — a person who names a flow has not
/// said where it should run, so the adapter fills that in before the executor
/// sees the command, exactly as it turns an [`Ordinal`] into a [`PaneRef`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FlowCommand {
    List,
    Run { name: String },
}

/// What the executor runs. `Run` carries the worktree it lands in, so the
/// answer is no longer the first place a caller learns where that was.
///
/// `List` takes none: it reports every window's active worktree and each row
/// says which, so there is no target to name. The same reason
/// `daruda_chat_list` and `daruda_worktree_list` take no arguments — a listing
/// names nothing, only an action does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResolvedFlowCommand {
    List,
    Run {
        name: String,
        lane: crate::control::result::LaneHandle,
    },
    /// Stop whatever `lane` is running. Names no flow: a worktree runs one at
    /// a time, so the worktree *is* the identifier.
    Stop {
        lane: crate::control::result::LaneHandle,
    },
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
    Flow(ResolvedFlowCommand),
    Brief,
    /// Hand `text` to the orchestrator. Named for who receives it, not for
    /// the act: the answer is *not* in this command's result — it arrives
    /// later as that pane's own completion ping — and a bare `Ask` promised
    /// otherwise. Compare [`Self::AskPane`], where the answer *is* the result.
    ///
    /// `destination` is the orchestrator's pane and `connecting` says whether
    /// naming it had to start one. Neither comes from an ordinal, so an
    /// adapter fills them from the orchestrator rather than from its own
    /// listing — but it does fill them, before the executor sees the command,
    /// like every other target here.
    AskOrchestrator {
        text: String,
        destination: PaneRef,
        connecting: bool,
    },
    /// Prompt one lane's agent and wait for the turn it starts, so the answer
    /// comes back as this command's own result.
    ///
    /// The waiting is what separates it from [`Self::Say`], which reports only
    /// that a prompt went out. Not a flag on `Say`: the two answer with
    /// different shapes, and a `wait: bool` beside a text field would spell
    /// states neither means.
    AskPane {
        target: PaneRef,
        text: String,
    },
    /// Every worktree, including ones with no agent chat in them — the
    /// listing `/list` answers with is chat-scoped and cannot name one.
    LaneList,
    /// What one chat's agent last said. A read; it starts no turn.
    Read {
        target: PaneRef,
    },
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
mod tests;
