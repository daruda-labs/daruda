//! Recognize explicit Python file writes without evaluating the script.

use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use super::{MAX_DEPTH, MAX_NODES, parse, text};

pub(super) fn edits_files(source: &str) -> bool {
    let Some(tree) = parse(source, tree_sitter_python::LANGUAGE.into()) else {
        return false;
    };
    let mut cursor = tree.root_node().walk();
    let functions: HashMap<_, _> = tree
        .root_node()
        .named_children(&mut cursor)
        .filter(|node| node.kind() == "function_definition")
        .filter_map(|node| {
            Some((
                text(node.child_by_field_name("name")?, source),
                node.child_by_field_name("body")?,
            ))
        })
        .collect();
    let mut followed = HashSet::new();
    let mut stack = vec![(tree.root_node(), 0)];
    let mut visited = 0;
    while let Some((node, depth)) = stack.pop() {
        visited += 1;
        if visited > MAX_NODES || depth > MAX_DEPTH {
            return false;
        }
        if matches!(
            node.kind(),
            "function_definition" | "class_definition" | "lambda"
        ) {
            continue;
        }
        if node.kind() == "call" {
            if writes(node, source) {
                return true;
            }
            if let Some(function) = node.child_by_field_name("function")
                && function.kind() == "identifier"
                && let name = text(function, source)
                && let Some(body) = functions.get(name)
                && followed.insert(name)
            {
                stack.push((*body, depth + 1));
            }
        }
        let mut cursor = node.walk();
        stack.extend(
            node.named_children(&mut cursor)
                .map(|child| (child, depth + 1)),
        );
    }
    false
}

fn writes(node: Node<'_>, source: &str) -> bool {
    let Some(function) = node.child_by_field_name("function") else {
        return false;
    };
    let Some(arguments) = node.child_by_field_name("arguments") else {
        return false;
    };
    if matches!(text(function, source), "open" | "io.open") {
        let mut cursor = arguments.walk();
        let args: Vec<_> = arguments.named_children(&mut cursor).collect();
        let mode = args
            .iter()
            .find_map(|arg| {
                (arg.kind() == "keyword_argument"
                    && arg
                        .child_by_field_name("name")
                        .is_some_and(|name| text(name, source) == "mode"))
                .then(|| arg.child_by_field_name("value"))
                .flatten()
            })
            .or_else(|| args.get(1).copied().filter(|arg| arg.kind() == "string"));
        return mode
            .and_then(|mode| string(mode, source))
            .is_some_and(|mode| mode.contains(['w', 'a', 'x', '+']));
    }
    if function.kind() == "attribute"
        && let Some(attribute) = function.child_by_field_name("attribute")
        && matches!(text(attribute, source), "write_text" | "write_bytes")
        && let Some(object) = function.child_by_field_name("object")
        && object.kind() == "call"
        && object
            .child_by_field_name("function")
            .is_some_and(|name| matches!(text(name, source), "Path" | "pathlib.Path"))
    {
        return true;
    }
    false
}

fn string<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    if node.kind() != "string" {
        return None;
    }
    let value = text(node, source);
    let quote = value.chars().next()?;
    if !matches!(quote, '\'' | '"') {
        return None;
    }
    let value = value.strip_prefix(quote)?.strip_suffix(quote)?;
    (!value.contains(['\\', '\'', '"'])).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_write_calls_not_mentions_or_definitions() {
        for source in [
            "open(p, 'w').write(s)",
            "open(p, mode='a')",
            "open(p, 'r+')",
            "Path(p).write_text(s)",
            "def patch(p):\n    open(p, 'w').write('x')\npatch('file')\n",
        ] {
            assert!(edits_files(source), "{source}");
        }
        for source in [
            "open(p).read()",
            "print(\"open(p, 'w')\")",
            "# open(p, 'w')",
            "def later():\n    open(p, 'w')\n",
            "open(p, mode=mode)",
            "open(",
            "def recurse():\n    recurse()\nrecurse()\n",
        ] {
            assert!(!edits_files(source), "{source}");
        }
    }
}
