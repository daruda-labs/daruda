//! Filling the template variables a flow may use: the five run variables
//! and the flow's declared inputs. Every value is substituted as an absolute
//! path, a rendered line or an input's text; only the shell surface quotes
//! them, because a prompt is prose and a command is not.

use crate::NodeId;
use crate::runner::NodeFailure;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Where a rendered string is about to go. A prompt is prose an agent
/// reads; a command is a line a shell parses, so only the latter quotes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Prompt,
    Shell,
}

/// Everything the variables can resolve to for one render.
pub struct TemplateContext<'a> {
    pub run_dir: &'a Path,
    /// The rendering node's own output, if it has one.
    pub output: Option<&'a Path>,
    /// Every node's output path, for `{{node.<id>.output}}`.
    pub node_outputs: &'a HashMap<NodeId, PathBuf>,
    /// The failure a repair or retry is responding to.
    pub failure: Option<&'a NodeFailure>,
    /// Archived evidence from earlier attempts.
    pub attempts: &'a [PathBuf],
    /// The run's input values, already resolved to text, for
    /// `{{input.<name>}}`.
    pub inputs: &'a BTreeMap<String, String>,
}

/// Substitute the variables. An unknown `{{…}}` is left verbatim —
/// static validation already rejected a bad node reference, and a stray
/// brace pair in prose is not this function's to police.
pub fn render(text: &str, ctx: &TemplateContext<'_>, surface: Surface) -> String {
    let mut out = String::with_capacity(text.len());
    for piece in pieces(text) {
        match piece {
            Piece::Text(text) => out.push_str(text),
            Piece::Token(token) => match resolve(token, ctx) {
                Some(value) => out.push_str(&emit(&value, surface)),
                None => {
                    out.push_str("{{");
                    out.push_str(token);
                    out.push_str("}}");
                }
            },
        }
    }
    out
}

/// One stretch of a template: literal text, or what sits between a `{{` and
/// its own nearest `}}`.
enum Piece<'t> {
    Text(&'t str),
    Token(&'t str),
}

/// `text` split the one way every reader of a template splits it, so what
/// renders, what is checked and what is renamed agree on what a token is.
/// Each token is bounded by its own closing pair, so an unrecognised template
/// does not consume the variables after it; an unterminated `{{` is text.
fn pieces(text: &str) -> impl Iterator<Item = Piece<'_>> {
    let mut rest = text;
    let mut token_next: Option<&str> = None;
    std::iter::from_fn(move || {
        if let Some(token) = token_next.take() {
            return Some(Piece::Token(token));
        }
        if rest.is_empty() {
            return None;
        }
        let Some((start, end)) = rest
            .find("{{")
            .and_then(|start| rest[start + 2..].find("}}").map(|end| (start, end)))
        else {
            return Some(Piece::Text(std::mem::take(&mut rest)));
        };
        let before = &rest[..start];
        let token = &rest[start + 2..start + 2 + end];
        rest = &rest[start + 2 + end + 2..];
        if before.is_empty() {
            Some(Piece::Token(token))
        } else {
            token_next = Some(token);
            Some(Piece::Text(before))
        }
    })
}

/// `None` means "not a variable" and is left verbatim; `Some("")` means a
/// known variable with nothing behind it right now.
fn resolve(token: &str, ctx: &TemplateContext<'_>) -> Option<String> {
    match token {
        "run_dir" => Some(ctx.run_dir.display().to_string()),
        "output" => Some(
            ctx.output
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        ),
        "failure" => Some(ctx.failure.map(|f| f.to_string()).unwrap_or_default()),
        "attempts" => Some(
            ctx.attempts
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => {
            if let Some(name) = token.strip_prefix(INPUT_PREFIX) {
                // Static validation refused an undeclared name, so a miss here
                // is a value the run never had; left verbatim like any other.
                return ctx.inputs.get(name).cloned();
            }
            token
                .strip_prefix("node.")
                .and_then(|rest| rest.strip_suffix(".output"))
                .map(|id| {
                    ctx.node_outputs
                        .get(id)
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                })
        }
    }
}

const INPUT_PREFIX: &str = "input.";

/// Every input name `text` writes as `{{input.<name>}}`, read through the
/// same [`pieces`] [`render`] substitutes from.
pub(crate) fn input_refs(text: &str) -> impl Iterator<Item = &str> {
    pieces(text).filter_map(|piece| match piece {
        Piece::Token(token) => token.strip_prefix(INPUT_PREFIX),
        Piece::Text(_) => None,
    })
}

/// Point every `{{node.<from>.output}}` in `text` at `to`.
///
/// Here rather than in the editor that calls it: this file is what decides
/// what a template token means, and a rewriter that spelled the token itself
/// would be a second answer to that — the kind that stays right until one of
/// them changes.
///
/// Reads through the same `pieces` `render` does, so an unterminated `{{`
/// or a token that is not an output reference is copied through rather than
/// swallowed. Only exact matches move: a node called `design` is not `design2`.
pub fn rename_output_refs(text: &str, from: &NodeId, to: &NodeId) -> String {
    let mut out = String::with_capacity(text.len());
    for piece in pieces(text) {
        match piece {
            Piece::Text(text) => out.push_str(text),
            Piece::Token(token) => {
                let renamed = token
                    .strip_prefix("node.")
                    .and_then(|rest| rest.strip_suffix(".output"))
                    .filter(|id| *id == from.as_str())
                    .map(|_| format!("node.{to}.output"));
                out.push_str("{{");
                out.push_str(renamed.as_deref().unwrap_or(token));
                out.push_str("}}");
            }
        }
    }
    out
}

fn emit(value: &str, surface: Surface) -> String {
    match surface {
        Surface::Prompt => value.to_string(),
        // `try_quote` only fails on an interior NUL, which no path or
        // rendered failure can hold. The fallback is a quoted empty word,
        // not nothing: dropping the argument entirely would leave a command
        // like `grep -q x ` reading stdin, which hangs instead of failing.
        Surface::Shell => shlex::try_quote(value)
            .map(|q| q.into_owned())
            .unwrap_or_else(|_| "''".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::NodeFailure;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::Duration;

    fn ctx_with(run_dir: &str) -> (PathBuf, HashMap<crate::NodeId, PathBuf>) {
        let run_dir = PathBuf::from(run_dir);
        let mut outputs = HashMap::new();
        outputs.insert("review".into(), run_dir.join("review.md"));
        outputs.insert("implement".into(), run_dir.join("implement.md"));
        (run_dir, outputs)
    }

    #[test]
    fn a_prompt_gets_absolute_paths_and_no_quoting() {
        let (run_dir, outputs) = ctx_with("/repo/.daruda/flow-runs/01J");
        let own = run_dir.join("design.md");
        let ctx = TemplateContext {
            inputs: &Default::default(),
            run_dir: &run_dir,
            output: Some(&own),
            node_outputs: &outputs,
            failure: None,
            attempts: &[],
        };
        let text = render(
            "write {{output}}, having read {{node.implement.output}} under {{run_dir}}",
            &ctx,
            Surface::Prompt,
        );
        assert_eq!(
            text,
            format!(
                "write {}, having read {} under {}",
                own.display(),
                run_dir.join("implement.md").display(),
                run_dir.display()
            )
        );
    }

    /// An input's value is prose in a prompt and one quoted argument in a
    /// command, so a value cannot add flags or chain a second command.
    #[test]
    fn an_input_is_quoted_on_the_shell_and_plain_in_a_prompt() {
        let (run_dir, outputs) = ctx_with("/repo/.daruda/flow-runs/01J");
        let inputs = BTreeMap::from([
            ("pr".to_string(), "1234".to_string()),
            ("title".to_string(), "fix it; rm -rf /".to_string()),
        ]);
        let ctx = TemplateContext {
            inputs: &inputs,
            run_dir: &run_dir,
            output: None,
            node_outputs: &outputs,
            failure: None,
            attempts: &[],
        };
        assert_eq!(
            render(
                "review {{input.pr}}: {{input.title}}",
                &ctx,
                Surface::Prompt
            ),
            "review 1234: fix it; rm -rf /"
        );
        let line = render(
            "gh pr view {{input.pr}} --title {{input.title}}",
            &ctx,
            Surface::Shell,
        );
        assert_eq!(
            shlex::split(&line).unwrap(),
            ["gh", "pr", "view", "1234", "--title", "fix it; rm -rf /"]
        );
        assert_eq!(
            render("{{input.missing}} {{.Names}}", &ctx, Surface::Shell),
            "{{input.missing}} {{.Names}}",
            "an unknown token is left verbatim"
        );
    }

    #[test]
    fn input_refs_reads_tokens_the_way_render_does() {
        let refs: Vec<_> =
            input_refs("{{input.a}} {{node.x.output}} {{input.b}} {{input.c").collect();
        assert_eq!(refs, ["a", "b"]);
    }

    /// The reason `Surface` exists: a run directory with a space in it must
    /// still reach `grep` as one argument.
    #[test]
    fn a_command_quotes_a_path_with_a_space_into_one_argument() {
        let (run_dir, outputs) = ctx_with("/Users/me/my repo/.daruda/flow-runs/01J");
        let ctx = TemplateContext {
            inputs: &Default::default(),
            run_dir: &run_dir,
            output: None,
            node_outputs: &outputs,
            failure: None,
            attempts: &[],
        };
        let text = render(
            "grep -q '^VERDICT: PASS' {{node.review.output}}",
            &ctx,
            Surface::Shell,
        );
        assert_eq!(
            shlex::split(&text).unwrap(),
            [
                "grep",
                "-q",
                "^VERDICT: PASS",
                &run_dir.join("review.md").to_string_lossy()
            ]
        );
    }

    /// Each match is bounded by its own nearest `}}`, so a template this
    /// function does not recognise leaves the variables after it alone.
    #[test]
    fn an_unrelated_template_does_not_swallow_a_later_variable() {
        let (run_dir, outputs) = ctx_with("/repo/run");
        let ctx = TemplateContext {
            inputs: &Default::default(),
            run_dir: &run_dir,
            output: None,
            node_outputs: &outputs,
            failure: None,
            attempts: &[],
        };
        let text = render(
            "{{unknown}} then {{node.review.output}}",
            &ctx,
            Surface::Prompt,
        );
        assert_eq!(
            text,
            format!(
                "{{{{unknown}}}} then {}",
                run_dir.join("review.md").display()
            )
        );
    }

    #[test]
    fn failure_and_attempts_render_for_a_repair_prompt() {
        let (run_dir, outputs) = ctx_with("/repo/run");
        let failure = NodeFailure::Timeout {
            elapsed: Duration::from_secs(600),
        };
        let attempts = vec![
            PathBuf::from("/repo/run/logs/test.attempt-1.evidence-1.log"),
            PathBuf::from("/repo/run/logs/review.attempt-1.evidence-1.md"),
        ];
        let ctx = TemplateContext {
            inputs: &Default::default(),
            run_dir: &run_dir,
            output: None,
            node_outputs: &outputs,
            failure: Some(&failure),
            attempts: &attempts,
        };
        let text = render("{{failure}}\n{{attempts}}", &ctx, Surface::Prompt);
        assert!(text.starts_with("timed out after 600 seconds"));
        assert!(text.contains("/repo/run/logs/test.attempt-1.evidence-1.log\n/repo/run/logs/review.attempt-1.evidence-1.md"));
    }

    /// A variable with nothing behind it renders empty rather than leaking
    /// `{{failure}}` into a prompt an agent has to read.
    #[test]
    fn an_unavailable_variable_renders_empty() {
        let (run_dir, outputs) = ctx_with("/repo/run");
        let ctx = TemplateContext {
            inputs: &Default::default(),
            run_dir: &run_dir,
            output: None,
            node_outputs: &outputs,
            failure: None,
            attempts: &[],
        };
        assert_eq!(
            render("[{{failure}}][{{output}}]", &ctx, Surface::Prompt),
            "[][]"
        );
    }

    /// On the shell an empty substitution has to stay a word. Dropping it
    /// would turn `grep -q x {{output}}` into `grep -q x`, which reads
    /// stdin and hangs rather than failing.
    #[test]
    fn an_empty_shell_substitution_is_still_one_argument() {
        let (run_dir, outputs) = ctx_with("/repo/run");
        let ctx = TemplateContext {
            inputs: &Default::default(),
            run_dir: &run_dir,
            output: None,
            node_outputs: &outputs,
            failure: None,
            attempts: &[],
        };
        assert_eq!(
            render("grep -q x {{output}}", &ctx, Surface::Shell),
            "grep -q x ''"
        );
    }

    /// A rename has to reach the prompts, or it produces a file whose text
    /// names a node that is not there — and the person who pressed rename has
    /// to go find them.
    #[test]
    fn a_rename_moves_only_the_reference_it_is_about() {
        let moved = rename_output_refs(
            "read {{node.design.output}} and {{node.design2.output}}, write {{output}}",
            &NodeId::from("design"),
            &NodeId::from("blueprint"),
        );
        assert_eq!(
            moved, "read {{node.blueprint.output}} and {{node.design2.output}}, write {{output}}",
            "an exact match moves; a longer name that starts the same does not"
        );
    }

    /// Scanned the way `render` scans, so text it does not understand comes
    /// out unchanged rather than being swallowed.
    #[test]
    fn a_rename_leaves_everything_it_does_not_understand_alone() {
        let from = NodeId::from("design");
        let to = NodeId::from("blueprint");
        for text in [
            "no templates here",
            "{{unknown}} and {{node.design.output}}",
            "an unterminated {{node.design.output",
            "{{node.design.other}}",
        ] {
            let moved = rename_output_refs(text, &from, &to);
            assert_eq!(
                moved.contains("{{node.design.output}}"),
                text.contains("{{node.design.output}}") && !text.contains("{{unknown}}"),
                "for {text:?} -> {moved:?}"
            );
            assert!(!moved.is_empty(), "nothing is swallowed: {text:?}");
        }
        assert_eq!(
            rename_output_refs("an unterminated {{node.design.output", &from, &to),
            "an unterminated {{node.design.output"
        );
    }
}
