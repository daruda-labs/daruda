//! What the orchestrator is told before it reads its first prompt. GPUI-free.
//!
//! English and not localized, like the tool definitions in
//! `control::mcp::tools`: this is machine data an LLM reads, and stable wording
//! is worth more than a translated one.
//!
//! It exists because an agent that is handed daruda's tools still boots as
//! whatever the user's own `~/.claude` configuration makes it. Observed without
//! this: asked to add a tab, the orchestrator loaded a brainstorming skill and
//! started grepping daruda's source for where to *implement* tabs — while
//! holding a tool that does it.
//!
//! Two channels, one text. `session/new` has no system-prompt field, so
//! [`briefing`] rides in front of the session's first prompt — the earliest
//! place it can land, and the only one that can name the MCP server, whose
//! name is per-run. [`instructions`] is the same words without that
//! sentence, for the file `window::install_instructions` leaves in the
//! session's directory: a prompt is read once, a directory file survives a
//! long conversation.
//!
//! Both are assembled from the same three constants, so the two channels
//! cannot drift.

/// The briefing for a session whose tools come from the MCP server called
/// `server`.
///
/// `server` is the per-run name (`daruda-0efc0655`), not a tool name — how an
/// agent spells a namespaced tool is its own business, so this names the
/// server once and the tools bare.
pub(crate) fn briefing(server: &str) -> String {
    format!("{PREAMBLE}\n\nYour tools come from the MCP server `{server}`.\n{TOOLS}\n{RULES}")
}

/// The same briefing as a standing instruction file.
///
/// No server sentence: a file outlives the run that wrote it, and a stale
/// server name is worse than none — the prompt form names the live one.
///
/// The header says it is generated because it is rewritten on every
/// orchestrator start; a person who wants different standing instructions has
/// to put them where their agent reads user-scoped ones, not here.
pub(crate) fn instructions() -> String {
    format!("{GENERATED_HEADER}\n\n{PREAMBLE}\n\nYour tools:\n{TOOLS}\n{RULES}\n")
}

const GENERATED_HEADER: &str =
    "<!-- Written by daruda every time the orchestrator starts. Edits are overwritten. -->";

const PREAMBLE: &str = "\
You are daruda's orchestrator. You are not a coding agent, and this session is \
not a place to write code.

A person is talking to you from their phone. They are not at a keyboard: they \
cannot read a plan, a table, or a code block, and they cannot answer a long \
question.";

const TOOLS: &str = "\
- daruda_chat_list, daruda_status — what is open and what it is doing
- daruda_chat_send, daruda_chat_stop — prompt one of those chats, or stop it
- daruda_chat_read — what one of those chats last said
- daruda_chat_ask — prompt one and wait for its reply, in one call
- daruda_worktree_list — every worktree, with the branch each is on
- daruda_chat_new — one more chat (a new tab) in a worktree that already exists
- daruda_worktree_create — a worktree on a *new* branch
- daruda_flow_list, daruda_flow_run, daruda_flow_stop — the saved flows";

const RULES: &str = "\
How to work here:
- Act. Do not load a skill, brainstorm, or write a plan — a message from the \
phone is an instruction, not a topic.
- Do not read or edit daruda's own source, and do not go looking through the \
filesystem for how daruda works. Everything you can do is in the tools above; \
if none of them does what was asked, say that.
- A worktree owns its branch, so a branch that already exists cannot get a \
second worktree. To add a tab to a worktree that already exists — including \
the one a project is currently on — use daruda_chat_new. Match a name the \
person says (\"main\", \"feat/x\") against daruda_worktree_list's rows rather \
than guessing a handle.
- daruda_chat_new and daruda_worktree_create ask the person to approve and \
block until they tap. That wait is normal; do not retry around it.
- daruda_chat_send does not return the answer. Reading straight after sending \
gives you what that chat said *before* — use daruda_chat_ask when you want the \
reply to your own prompt, and check daruda_chat_list for an `activity` of idle \
before reading one any other way.
- Answer in a sentence or two, as plain text. When a tool fails, say what it \
reported rather than guessing why.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_this_run_advertises_is_named() {
        let text = briefing("daruda-0efc0655");
        assert!(
            text.contains("`daruda-0efc0655`"),
            "the agent has to know which server its tools came from: {text}"
        );
    }

    /// Every tool in the table is named, or the orchestrator will not know it
    /// has one — which is how it ends up in the terminal instead.
    #[test]
    fn every_advertised_tool_appears() {
        let text = briefing("daruda-x");
        for tool in crate::control::mcp::tools::table_for_test() {
            assert!(
                text.contains(tool.name),
                "{} is advertised but not briefed",
                tool.name
            );
        }
    }

    /// The two failures this exists to prevent, pinned so a later edit cannot
    /// quietly drop them — in *both* channels, because either one alone is
    /// what the agent might be reading.
    #[test]
    fn the_two_observed_failures_are_addressed() {
        for text in [briefing("daruda-x"), instructions()] {
            let text = text.to_lowercase();
            assert!(
                text.contains("do not load a skill"),
                "it loaded a brainstorming skill instead of acting: {text}"
            );
            assert!(
                text.contains("do not read or edit daruda's own source"),
                "it went looking for where to implement what a tool already does: {text}"
            );
        }
    }

    /// A file outlives the run that wrote it, so it must not carry that run's
    /// server name — a stale one would send the agent looking for tools under
    /// a namespace that no longer exists.
    #[test]
    fn the_file_form_names_no_run() {
        let text = instructions();
        assert!(
            !text.contains("MCP server `"),
            "the per-run name belongs to the prompt form only: {text}"
        );
        assert!(text.starts_with("<!--"), "and it says it is generated");
        for tool in crate::control::mcp::tools::table_for_test() {
            assert!(text.contains(tool.name), "{} is not in the file", tool.name);
        }
    }
}
