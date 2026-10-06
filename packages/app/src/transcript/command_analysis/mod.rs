//! Structured shell metadata for tool cards and future transcript queries.
//! Effects describe statically recognized operations, not permission decisions.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use daruda_acp::{ChatItem, ToolCallItem, ToolKindView};
use tree_sitter::{Language, Node, ParseOptions, Parser, Tree};

mod python;
mod rules;

const MAX_COMMAND_BYTES: usize = 64 * 1024;
const MAX_NODES: usize = 8192;
const MAX_DEPTH: usize = 64;
const PARSE_BUDGET: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommandEffect {
    Read,
    Edit,
}

/// Locale-independent metadata. Missing effects stay unknown; they are not reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CommandAnalysis {
    /// Distinct literal program names in source order, including substitutions.
    pub(crate) programs: Vec<String>,
    /// An edit takes precedence; a read requires every operation to be understood.
    pub(crate) effect: Option<CommandEffect>,
}

struct CachedAnalysis {
    source: String,
    analysis: CommandAnalysis,
}

/// Derived once per input change, keyed by the same ID as the tool call.
#[derive(Default)]
pub(crate) struct CommandAnalysisIndex {
    entries: HashMap<String, CachedAnalysis>,
}

impl CommandAnalysisIndex {
    pub(crate) fn get(&self, tool_id: &str) -> Option<&CommandAnalysis> {
        self.entries.get(tool_id).map(|entry| &entry.analysis)
    }

    /// Reconcile input changes, late input, replay, and removed calls together.
    pub(crate) fn reconcile(&mut self, items: &[ChatItem]) {
        let mut retained = HashSet::new();
        for item in items {
            let ChatItem::ToolCall(call) = item else {
                continue;
            };
            let Some(source) = command_source(call) else {
                continue;
            };
            retained.insert(call.id.as_str());
            if self
                .entries
                .get(&call.id)
                .is_some_and(|entry| entry.source == source)
            {
                continue;
            }
            self.entries.insert(
                call.id.clone(),
                CachedAnalysis {
                    source: source.to_owned(),
                    analysis: analyze(source),
                },
            );
        }
        self.entries.retain(|id, _| retained.contains(id.as_str()));
    }
}

fn command_source(call: &ToolCallItem) -> Option<&str> {
    if call.kind != ToolKindView::Execute {
        return None;
    }
    // Titles can be prose or truncated. Only the actual tool argument is code.
    let source = call.raw_input.as_ref()?.get("command")?.as_str()?;
    (!source.trim().is_empty() && source.len() <= MAX_COMMAND_BYTES).then_some(source)
}

fn parse(source: &str, language: Language) -> Option<Tree> {
    if source.len() > MAX_COMMAND_BYTES {
        return None;
    }
    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let start = Instant::now();
    let mut cancelled = |_: &tree_sitter::ParseState| start.elapsed() > PARSE_BUDGET;
    let tree = parser.parse_with_options(
        &mut |offset, _| &source.as_bytes()[offset..],
        None,
        Some(ParseOptions::new().progress_callback(&mut cancelled)),
    )?;
    (!tree.root_node().has_error()).then_some(tree)
}

fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

/// Decode only static shell words; expansions and globbing are not evaluated.
fn literal(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "command_name" => return literal(node.named_child(0)?, source),
        "word" | "number" => {
            if text(node, source).contains(['$', '`', '*', '?', '[', '{', '~']) {
                return None;
            }
        }
        "string" => {
            let mut cursor = node.walk();
            if node
                .named_children(&mut cursor)
                .any(|child| child.kind() != "string_content")
            {
                return None;
            }
        }
        "raw_string" => {}
        _ => return None,
    }
    let mut words = shell_words::split(text(node, source)).ok()?;
    (words.len() == 1).then(|| words.remove(0))
}

fn combine(left: Option<CommandEffect>, right: Option<CommandEffect>) -> Option<CommandEffect> {
    match (left, right) {
        (Some(CommandEffect::Edit), _) | (_, Some(CommandEffect::Edit)) => {
            Some(CommandEffect::Edit)
        }
        (Some(CommandEffect::Read), Some(CommandEffect::Read)) => Some(CommandEffect::Read),
        _ => None,
    }
}

fn analyze(source: &str) -> CommandAnalysis {
    let Some(tree) = parse(source, tree_sitter_bash::LANGUAGE.into()) else {
        return CommandAnalysis::default();
    };
    let mut visitor = ShellVisitor {
        source,
        programs: Vec::new(),
        remaining: MAX_NODES,
    };
    let mut effect = visitor.visit(tree.root_node(), 0);
    if visitor.remaining == 0 {
        return CommandAnalysis::default();
    }
    if visitor.programs.is_empty() && effect == Some(CommandEffect::Read) {
        effect = None;
    }
    CommandAnalysis {
        programs: visitor.programs,
        effect,
    }
}

struct ShellVisitor<'a> {
    source: &'a str,
    programs: Vec<String>,
    remaining: usize,
}

impl ShellVisitor<'_> {
    fn visit(&mut self, node: Node<'_>, depth: usize) -> Option<CommandEffect> {
        if self.remaining == 0 || depth > MAX_DEPTH {
            self.remaining = 0;
            return None;
        }
        self.remaining -= 1;
        match node.kind() {
            "program"
            | "list"
            | "pipeline"
            | "subshell"
            | "compound_statement"
            | "negated_command"
            | "command_substitution"
            | "process_substitution" => self.children(node, depth),
            "redirected_statement" => {
                let mut effect = self.children(node, depth);
                if let Some(code) = self.python_heredoc(node)
                    && python::edits_files(code)
                {
                    effect = Some(CommandEffect::Edit);
                }
                effect
            }
            "command" => {
                let effect = self.command(node);
                combine(effect, self.substitutions(node, depth))
            }
            "file_redirect" => {
                let mut cursor = node.walk();
                let operator = node
                    .children(&mut cursor)
                    .find(|child| !child.is_named())
                    .map(|child| text(child, self.source));
                let destination = node
                    .child_by_field_name("destination")
                    .and_then(|child| literal(child, self.source));
                let effect = match operator {
                    Some(">" | ">>" | ">|" | "&>" | "&>>")
                        if destination.as_deref() != Some("/dev/null") =>
                    {
                        Some(CommandEffect::Edit)
                    }
                    Some(">&")
                        if !destination
                            .as_deref()
                            .is_some_and(|s| s == "-" || s.bytes().all(|b| b.is_ascii_digit())) =>
                    {
                        None
                    }
                    _ => Some(CommandEffect::Read),
                };
                combine(effect, self.substitutions(node, depth))
            }
            "heredoc_redirect" => {
                let mut cursor = node.walk();
                let mut effect = Some(CommandEffect::Read);
                for child in node.named_children(&mut cursor) {
                    let next = match child.kind() {
                        "command"
                        | "pipeline"
                        | "list"
                        | "redirected_statement"
                        | "file_redirect" => self.visit(child, depth + 1),
                        _ => self.substitutions(child, depth + 1),
                    };
                    effect = combine(effect, next);
                }
                effect
            }
            "herestring_redirect" | "variable_assignment" => self.substitutions(node, depth),
            "comment" => Some(CommandEffect::Read),
            // Definitions and control flow cannot be flattened into executed calls.
            _ => None,
        }
    }

    fn children(&mut self, node: Node<'_>, depth: usize) -> Option<CommandEffect> {
        let mut cursor = node.walk();
        let mut effect = Some(CommandEffect::Read);
        // Unknown effects must not short-circuit a later edit or program name.
        for child in node.named_children(&mut cursor) {
            effect = combine(effect, self.visit(child, depth + 1));
        }
        effect
    }

    fn substitutions(&mut self, node: Node<'_>, depth: usize) -> Option<CommandEffect> {
        if depth > MAX_DEPTH || self.remaining == 0 {
            self.remaining = 0;
            return None;
        }
        self.remaining -= 1;
        let mut cursor = node.walk();
        let mut effect = Some(CommandEffect::Read);
        for child in node.named_children(&mut cursor) {
            let child_effect = match child.kind() {
                "command_substitution" | "process_substitution" | "file_redirect" => {
                    self.visit(child, depth + 1)
                }
                _ => self.substitutions(child, depth + 1),
            };
            effect = combine(effect, child_effect);
        }
        effect
    }

    fn command(&mut self, node: Node<'_>) -> Option<CommandEffect> {
        let name = literal(node.child_by_field_name("name")?, self.source)?;
        let program = name.rsplit('/').next()?;
        if program.is_empty()
            || program.len() > 64
            || !program
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._+-[".contains(&b))
        {
            return None;
        }
        if !self.programs.iter().any(|known| known == program) {
            self.programs.push(program.to_owned());
        }
        let args = arguments(node, self.source)?;
        // A workspace script named `cat` is not the system utility.
        if name.contains('/')
            && ![
                "/bin/",
                "/usr/bin/",
                "/usr/local/bin/",
                "/opt/homebrew/bin/",
            ]
            .iter()
            .any(|prefix| name.strip_prefix(prefix) == Some(program))
        {
            return None;
        }
        if matches!(program, "python" | "python3") {
            return (args.first().is_some_and(|arg| arg == "-c")
                && args.get(1).is_some_and(|code| python::edits_files(code)))
            .then_some(CommandEffect::Edit);
        }
        rules::effect(program, &args)
    }

    fn python_heredoc<'a>(&'a self, node: Node<'_>) -> Option<&'a str> {
        let body = node.child_by_field_name("body")?;
        if body.kind() != "command" {
            return None;
        }
        let name = body.child_by_field_name("name")?;
        let program = literal(name, self.source)?;
        if !matches!(program.as_str(), "python" | "python3" | "/usr/bin/python3") {
            return None;
        }
        let mut cursor = node.walk();
        let redirects: Vec<_> = node
            .named_children(&mut cursor)
            .filter(|child| child.kind() == "heredoc_redirect")
            .collect();
        let [redirect] = redirects.as_slice() else {
            return None;
        };
        // WORKAROUND: tree-sitter-bash 0.25.1 drops `-` before a heredoc.
        // Decode that header's source span until the upstream grammar is fixed.
        let args =
            shell_words::split(self.source.get(name.end_byte()..redirect.start_byte())?).ok()?;
        if !args.is_empty() && args != ["-"] {
            return None;
        }
        let mut cursor = redirect.walk();
        if redirect
            .children_by_field_name("argument", &mut cursor)
            .next()
            .is_some()
        {
            return None;
        }
        let mut cursor = redirect.walk();
        let start = redirect
            .named_children(&mut cursor)
            .find(|child| child.kind() == "heredoc_start")?;
        // Unquoted heredocs expand shell code before Python sees the text.
        let delimiter = text(start, self.source);
        if !delimiter.starts_with(['\'', '"']) {
            return None;
        }
        let mut cursor = redirect.walk();
        redirect
            .named_children(&mut cursor)
            .find(|child| child.kind() == "heredoc_body")
            .map(|body| text(body, self.source))
    }
}

fn arguments(node: Node<'_>, source: &str) -> Option<Vec<String>> {
    let mut cursor = node.walk();
    node.children_by_field_name("argument", &mut cursor)
        .map(|arg| literal(arg, source))
        .collect()
}

#[cfg(test)]
mod tests;
