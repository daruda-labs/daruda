//! The text an attempt hands the runner: a command's rendered `run`, or an
//! agent's prompt with the retry hint and the output contract appended
//! after it. Separated from the drive loop because the loop only passes the
//! text on — composing it, and reading the files it comes from, is a
//! question of its own.

use super::policy::PolicyKind;
use super::{Run, doing};
use crate::model::{Node, NodeKind, Prompt};
use crate::runner::{NodeFailure, RunContext};
use crate::template::{Surface, TemplateContext, render};
use std::path::PathBuf;

/// A read that failed, labelled and with the path it was given. Everything
/// a `FlowIoError` needs except the node and attempt, which the reader does
/// not have.
type ReadFailure = (&'static str, PathBuf, std::io::Error);

impl Run<'_> {
    /// The text this attempt hands the runner: for a command the rendered
    /// `run`, for an agent the rendered prompt plus the channels
    /// [`push_channel`] appends to it.
    /// Fallible because `Prompt::File` and a file-backed hint are both read
    /// here; nothing downstream can read them. The error carries which of
    /// the two it was, so a missing hint is never reported as a missing
    /// prompt; the caller adds the node and attempt it has and this does
    /// not.
    pub(super) fn node_text(
        &self,
        node: &Node,
        ctx: &RunContext<'_>,
        evidence: &[PathBuf],
        failure: Option<&NodeFailure>,
    ) -> Result<String, ReadFailure> {
        let tctx = TemplateContext {
            run_dir: self.run_dir,
            output: ctx.output,
            node_outputs: &self.node_outputs,
            failure,
            attempts: evidence,
        };
        match &node.kind {
            NodeKind::Command { run, .. } => Ok(render(run, &tctx, Surface::Shell)),
            NodeKind::Agent(body) => {
                let mut text = render(
                    &self.read_prompt(&body.prompt, doing::READ_PROMPT)?,
                    &tctx,
                    Surface::Prompt,
                );
                if let (Some(_), PolicyKind::Retry { hint }) = (failure, self.policy_of(node).kind)
                {
                    // The node's own prompt stays unchanged; the hint
                    // answers the failure that made this attempt happen.
                    push_channel(
                        &mut text,
                        &render(
                            &self.read_prompt(&hint, doing::READ_HINT)?,
                            &tctx,
                            Surface::Prompt,
                        ),
                    );
                }
                // Keyed on the file being owed rather than on the node's
                // kind, so a repair's fix session — an agent run with no
                // output — is never told to write one.
                if let Some(output) = ctx.output {
                    push_channel(
                        &mut text,
                        &crate::contract::prompt::block(
                            output,
                            node.kind.output_schema(),
                            node.kind.continue_until(),
                        ),
                    );
                }
                Ok(text)
            }
        }
    }

    /// `doing` is the caller's label because this reads both the node's
    /// prompt and its hint, and only the caller knows which one it asked for.
    fn read_prompt(&self, prompt: &Prompt, doing: &'static str) -> Result<String, ReadFailure> {
        match prompt {
            Prompt::Inline(text) => Ok(text.clone()),
            Prompt::File(path) => {
                let path = self.flow_dir.join(path);
                std::fs::read_to_string(&path).map_err(|e| (doing, path, e))
            }
        }
    }
}

/// Append one channel to a composed prompt, separated so the agent can see
/// where the node's own words end. Three of them, always in this order:
/// the node's prompt, the retry hint, the output contract — a model weights
/// the end of a prompt most, and a correction turn refers to the contract
/// as being above whatever it says.
fn push_channel(text: &mut String, channel: &str) {
    text.push_str("\n\n---\n");
    text.push_str(channel);
}
