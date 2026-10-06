//! Where a key path lands in the text of a flow file.
//!
//! A recursive descent over the tree-sitter CST rather than a query with a
//! depth cursor (Zed's shape for JSON): a flow file addresses sequence elements
//! by index — `nodes[1].output` — and a query over key/value pairs cannot say
//! which element of a sequence it is inside. Descending explicitly also gives
//! each step the one thing an edit needs and a query does not report: the
//! column the container is indented to.
//!
//! Node kinds are measured against `tree-sitter-yaml` 0.7.2, not assumed:
//! `document → block_node → block_mapping → block_mapping_pair{key,value}`, a
//! sequence value is `block_node → block_sequence → block_sequence_item`, and a
//! `|` value is a single `block_scalar` covering the header and every indented
//! line.

use std::ops::Range;

use tree_sitter::Node;

use super::FlowEditError;

/// One step of a path into a flow file's value tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Key(String),
    /// A sequence element, in two coordinates: where the **text** holds it, and
    /// where the **new tree** does.
    ///
    /// They differ the moment an earlier element is removed, and reading the new
    /// tree at a text index is how a surviving node came to be written with a
    /// different node's value.
    ///
    /// `value: None` is the element being removed — it has no place in the new
    /// tree. Nothing reads it today (a removal's path stops before any value is
    /// fetched), so it is what absence *means* here rather than a branch under
    /// test.
    Index {
        text: usize,
        value: Option<usize>,
    },
}

impl Step {
    /// An element that sits at the same index in both, which is every
    /// positional diff: nothing before it moved.
    pub(super) fn index(at: usize) -> Self {
        Step::Index {
            text: at,
            value: Some(at),
        }
    }
}

/// What the text holds at a path that exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Found {
    /// The value alone — what a replacement overwrites.
    pub value: Range<usize>,
    /// The whole entry, key included — what a deletion takes out, grown to
    /// whole lines by [`line_span`].
    pub entry: Range<usize>,
    /// The column the entry starts at, which a replacement value has to line
    /// its continuation lines up with.
    pub column: usize,
    /// The value's own kind, so a block scalar can be rewritten as one.
    pub kind: ValueKind,
    /// Just past the `:` of a mapping pair — where a value written on the same
    /// line begins. `None` for a sequence element, which has no key.
    ///
    /// Needed for the case where a value that was a block (`on_fail:` with a
    /// mapping under it) becomes a scalar: the value's own range starts on the
    /// next line, so replacing just that leaves `on_fail:` and `halt` on two
    /// lines. Taking the gap with it puts the value back beside its key.
    pub key_end: Option<usize>,
    /// Where the value's *content* ends. For a container this is the end of its
    /// last entry, which is not the end of the node: tree-sitter can include the
    /// trailing newline in a block container's range, and appending after that
    /// would leave the file without one.
    pub content_end: usize,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum ValueKind {
    /// A plain, quoted, or otherwise single-token value.
    Scalar,
    /// `|` or `|-`. Carries the content column so a rewrite indents the same.
    BlockScalar { header: BlockHeader, column: usize },
    /// A nested `block_mapping`.
    Mapping,
    /// A nested `block_sequence`.
    Sequence,
    /// `[a, b]`. A *value* this module can replace whole; not one it will step
    /// into — see [`descend`], which still refuses that.
    FlowSequence,
}

/// Which block header a `block_scalar` was written with. `>` folded scalars are
/// absent on purpose — see [`FlowEditError::FoldedScalar`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum BlockHeader {
    /// `|` — one trailing newline kept.
    Keep,
    /// `|-` — trailing newlines stripped.
    Strip,
}

impl BlockHeader {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            BlockHeader::Keep => "|",
            BlockHeader::Strip => "|-",
        }
    }
}

/// A path whose last step is missing, and where a new entry for it goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Vacancy {
    /// Insert here — the end of the container's last entry.
    pub at: usize,
    /// The column the container's entries start at.
    pub column: usize,
    /// What the container is, so the caller knows whether to write `key: value`
    /// or `- value`.
    pub container: Container,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum Container {
    Mapping,
    Sequence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Site {
    Found(Found),
    Vacant(Vacancy),
}

/// Parse `text` as YAML. Fails only if the grammar cannot be loaded, which is a
/// build problem rather than a data one.
pub(super) fn parse(text: &str) -> Result<tree_sitter::Tree, FlowEditError> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_yaml::LANGUAGE.into())
        .map_err(|e| FlowEditError::Unaddressable(e.to_string()))?;
    parser
        .parse(text, None)
        .ok_or_else(|| FlowEditError::Unaddressable("yaml parser returned no tree".into()))
}

/// Resolve `path` against the document's root mapping.
pub(super) fn locate(
    tree: &tree_sitter::Tree,
    text: &str,
    path: &[Step],
) -> Result<Site, FlowEditError> {
    let root = document_root(tree.root_node())
        .ok_or_else(|| FlowEditError::Unaddressable("the file holds no document".into()))?;
    descend(root, text, path)
}

/// The single document's top-level node. A file with two documents resolves to
/// nothing rather than to the first one — a flow file has no reason to hold two,
/// and picking one silently would edit a document nobody named.
fn document_root(root: Node) -> Option<Node> {
    let mut documents = (0..root.child_count())
        .filter_map(|i| root.child(i))
        .filter(|c| c.kind() == "document");
    let first = documents.next()?;
    if documents.next().is_some() {
        return None;
    }
    unwrap_node(first)
}

/// Step past the `block_node` / `flow_node` wrappers to the thing itself.
fn unwrap_node(node: Node) -> Option<Node> {
    let mut current = node;
    loop {
        match current.kind() {
            "document" | "block_node" | "flow_node" => {
                current = (0..current.child_count())
                    .filter_map(|i| current.child(i))
                    .find(|c| c.is_named())?;
            }
            _ => return Some(current),
        }
    }
}

fn descend(container: Node, text: &str, path: &[Step]) -> Result<Site, FlowEditError> {
    let Some((step, rest)) = path.split_first() else {
        return Err(FlowEditError::Unaddressable("empty path".into()));
    };
    match (container.kind(), step) {
        ("block_mapping", Step::Key(key)) => {
            let pair = pairs(container).find(|pair| key_text(*pair, text) == Some(key.as_str()));
            let Some(pair) = pair else {
                // A vacancy is only meaningful for the *last* step. A path that
                // continues through a key the file does not have names no place
                // in this text, and answering with this container's insertion
                // point would write the tail key straight into it — the right
                // shape, one level too shallow, and silently.
                if !rest.is_empty() {
                    return Err(FlowEditError::Unaddressable(format!(
                        "the path continues through `{key}`, which the file does not have"
                    )));
                }
                return Ok(Site::Vacant(Vacancy {
                    at: last_entry_end(container, text),
                    column: container.start_position().column,
                    container: Container::Mapping,
                }));
            };
            let value = pair
                .child_by_field_name("value")
                .ok_or_else(|| FlowEditError::Unaddressable(format!("`{key}` has no value")))?;
            step_into(pair, value, text, rest)
        }
        ("block_sequence", Step::Index { text: index, .. }) => {
            let items: Vec<Node> = (0..container.child_count())
                .filter_map(|i| container.child(i))
                .filter(|c| c.kind() == "block_sequence_item")
                .collect();
            let Some(item) = items.get(*index) else {
                return Ok(Site::Vacant(Vacancy {
                    at: last_entry_end(container, text),
                    column: container.start_position().column,
                    container: Container::Sequence,
                }));
            };
            let value = (0..item.child_count())
                .filter_map(|i| item.child(i))
                .find(|c| c.is_named())
                .ok_or_else(|| {
                    FlowEditError::Unaddressable(format!("sequence item {index} is empty"))
                })?;
            step_into(*item, value, text, rest)
        }
        ("flow_mapping" | "flow_sequence", _) => Err(FlowEditError::FlowStyle(
            text[container.byte_range()].chars().take(40).collect(),
        )),
        (kind, _) => Err(FlowEditError::Unaddressable(format!(
            "a {kind} cannot be stepped into by this path"
        ))),
    }
}

/// Either this entry *is* the target, or the path continues inside its value.
fn step_into(entry: Node, value: Node, text: &str, rest: &[Step]) -> Result<Site, FlowEditError> {
    if rest.is_empty() {
        let kind = value_kind(value, text)?;
        let content_end = match kind {
            ValueKind::Mapping | ValueKind::Sequence => unwrap_node(value)
                .map(|inner| last_entry_end(inner, text))
                .unwrap_or_else(|| value.end_byte()),
            _ => value.end_byte(),
        };
        return Ok(Site::Found(Found {
            value: value.byte_range(),
            entry: entry.byte_range(),
            column: entry.start_position().column,
            kind,
            content_end,
            key_end: colon_end(entry),
        }));
    }
    let inner = unwrap_node(value).ok_or_else(|| {
        FlowEditError::Unaddressable("the path continues into an empty value".into())
    })?;
    descend(inner, text, rest)
}

fn value_kind(value: Node, text: &str) -> Result<ValueKind, FlowEditError> {
    let Some(inner) = unwrap_node(value) else {
        return Ok(ValueKind::Scalar);
    };
    match inner.kind() {
        "block_mapping" => Ok(ValueKind::Mapping),
        "block_sequence" => Ok(ValueKind::Sequence),
        "block_scalar" => {
            let body = &text[inner.byte_range()];
            let header = block_header(body.lines().next().unwrap_or("").trim_end())?;
            // The content's own column, read off the first body line rather
            // than derived from the header: a block scalar may be indented
            // further than the inferred step, and rewriting it at the inferred
            // one would silently reflow it.
            let column = body
                .lines()
                .nth(1)
                .map(|line| line.len() - line.trim_start().len())
                .unwrap_or(inner.start_position().column);
            Ok(ValueKind::BlockScalar { header, column })
        }
        "flow_sequence" => Ok(ValueKind::FlowSequence),
        "flow_mapping" => Err(FlowEditError::FlowStyle(
            text[inner.byte_range()].chars().take(40).collect(),
        )),
        _ => Ok(ValueKind::Scalar),
    }
}

/// Read a block scalar's header the way YAML spells it: the style, then an
/// indentation indicator and a chomping indicator in either order.
///
/// The whole grammar rather than the two bare headers, because [`super::render`]
/// writes an indicator (`|2`) whenever a body's first line is indented, and
/// what this module writes it has to be able to read.
///
/// The indicator is not carried: it says where the content starts, which is
/// measured from the content itself, and a rewrite states it again from what
/// the text holds then.
fn block_header(header: &str) -> Result<BlockHeader, FlowEditError> {
    let Some(rest) = header.strip_prefix('|') else {
        return Err(FlowEditError::FoldedScalar(header.to_string()));
    };
    let mut chomping = BlockHeader::Keep;
    for c in rest.chars() {
        match c {
            '1'..='9' => {}
            '-' => chomping = BlockHeader::Strip,
            // `|+` keeps every trailing newline, and this module has no way to
            // say that: rewriting it as `|` would quietly collapse them to one.
            '+' => {
                return Err(FlowEditError::Unrepresentable(format!(
                    "a block scalar that keeps its trailing newlines: {header}"
                )));
            }
            _ => return Err(FlowEditError::FoldedScalar(header.to_string())),
        }
    }
    Ok(chomping)
}

/// The `:` token's end in a mapping pair. The grammar gives it as a child, so
/// this is exact rather than a search back through the text for a colon that
/// might be inside a quoted key.
fn colon_end(entry: Node) -> Option<usize> {
    (0..entry.child_count())
        .filter_map(|i| entry.child(i))
        .find(|c| c.kind() == ":")
        .map(|c| c.end_byte())
}

fn pairs<'a>(mapping: Node<'a>) -> impl Iterator<Item = Node<'a>> {
    (0..mapping.child_count())
        .filter_map(move |i| mapping.child(i))
        .filter(|c| c.kind() == "block_mapping_pair")
}

fn key_text<'a>(pair: Node, text: &'a str) -> Option<&'a str> {
    let key = pair.child_by_field_name("key")?;
    let scalar = unwrap_node(key)?;
    let raw = &text[scalar.byte_range()];
    Some(raw.trim_matches(|c| c == '"' || c == '\''))
}

/// Where the container's content ends — the end of its last entry, which is not
/// the same as the container's own end when a comment trails inside it.
/// Inserting *there* rather than after the comment keeps the comment attached to
/// whatever follows it.
///
/// Trailing whitespace is cut off: a `block_sequence_item` runs to the newline
/// after it (measured), and inserting past that newline would take the file's
/// last one with it.
fn last_entry_end(container: Node, text: &str) -> usize {
    let end = (0..container.child_count())
        .filter_map(|i| container.child(i))
        .filter(|c| matches!(c.kind(), "block_mapping_pair" | "block_sequence_item"))
        .filter_map(|c| content_end_of(c))
        .max()
        .unwrap_or_else(|| container.end_byte());
    text[..end].trim_end_matches([' ', '\t', '\r', '\n']).len()
}

/// Where `node`'s text ends once comments are left out, or `None` when it is
/// nothing but one.
///
/// A comment sits inside whichever block is open at its indentation, which can
/// be several levels below the entry it trails — so an entry's own `end_byte`
/// reaches past it and only walking down finds where the content really stops.
fn content_end_of(node: Node) -> Option<usize> {
    if node.kind() == "comment" {
        return None;
    }
    // A node with no comment in it ends where it ends. Descending anyway would
    // answer with its children's extent, which is short of the node's own
    // wherever the text is not all children — a block scalar's body is not.
    if !holds_a_comment(node) {
        return Some(node.end_byte());
    }
    let deepest = children(node).filter_map(content_end_of).max();
    Some(deepest.unwrap_or_else(|| node.end_byte()))
}

fn holds_a_comment(node: Node) -> bool {
    children(node).any(|c| c.kind() == "comment" || holds_a_comment(c))
}

fn children(node: Node<'_>) -> impl Iterator<Item = Node<'_>> {
    (0..node.child_count()).filter_map(move |i| node.child(i))
}

/// Grow `range` to whole lines, trailing newline included — what taking an
/// entry out of a block-style document means.
pub(super) fn line_span(text: &str, range: &Range<usize>) -> Range<usize> {
    let start = text[..range.start].rfind('\n').map_or(0, |ix| ix + 1);
    let end = text[range.end..]
        .find('\n')
        .map_or(text.len(), |ix| range.end + ix + 1);
    start..end
}

/// The indentation step this file is written with, measured rather than assumed:
/// the first place a nested block container sits further right than its parent.
/// Two when the file has no nesting to measure.
pub(super) fn infer_indent(tree: &tree_sitter::Tree) -> usize {
    const DEFAULT: usize = 2;
    let mut cursor = tree.walk();
    let mut stack = vec![tree.root_node()];
    let mut best: Option<usize> = None;
    while let Some(node) = stack.pop() {
        if matches!(node.kind(), "block_mapping" | "block_sequence") {
            for pair in (0..node.child_count()).filter_map(|i| node.child(i)) {
                if let Some(value) = pair.child_by_field_name("value")
                    && let Some(inner) = unwrap_node(value)
                    && matches!(inner.kind(), "block_mapping" | "block_sequence")
                {
                    let step = inner
                        .start_position()
                        .column
                        .saturating_sub(node.start_position().column);
                    if step > 0 {
                        best = Some(best.map_or(step, |b: usize| b.min(step)));
                    }
                }
            }
        }
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    best.unwrap_or(DEFAULT)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
version: 1
# a comment
defaults:
  agent:
    id: claude
nodes:
  - id: design
    output: design.md
    prompt: |
      write a line
  - id: build
    deps: [design]
";

    fn key(k: &str) -> Step {
        Step::Key(k.to_string())
    }

    fn found(text: &str, path: &[Step]) -> Found {
        let tree = parse(text).expect("parses");
        match locate(&tree, text, path).expect("resolves") {
            Site::Found(found) => found,
            Site::Vacant(v) => panic!("expected a value, got a vacancy at {}", v.at),
        }
    }

    #[test]
    fn a_top_level_scalar_resolves_to_its_value_alone() {
        let f = found(SAMPLE, &[key("version")]);
        assert_eq!(&SAMPLE[f.value.clone()], "1");
        assert_eq!(&SAMPLE[f.entry], "version: 1");
        assert_eq!(f.kind, ValueKind::Scalar);
    }

    #[test]
    fn a_nested_key_resolves_through_the_mappings_above_it() {
        let f = found(SAMPLE, &[key("defaults"), key("agent"), key("id")]);
        assert_eq!(&SAMPLE[f.value], "claude");
        assert_eq!(f.column, 4, "the column its continuation lines answer to");
    }

    #[test]
    fn a_sequence_element_resolves_by_index() {
        let f = found(SAMPLE, &[key("nodes"), Step::index(1), key("id")]);
        assert_eq!(&SAMPLE[f.value], "build");
    }

    /// The header and every indented line are one value, and the content column
    /// is read off the body rather than inferred.
    #[test]
    fn a_block_scalar_is_one_value_with_its_own_column() {
        let f = found(SAMPLE, &[key("nodes"), Step::index(0), key("prompt")]);
        assert_eq!(&SAMPLE[f.value], "|\n      write a line");
        assert_eq!(
            f.kind,
            ValueKind::BlockScalar {
                header: BlockHeader::Keep,
                column: 6
            }
        );
    }

    #[test]
    fn a_flow_sequence_is_refused_rather_than_guessed_at() {
        let text = SAMPLE;
        let tree = parse(text).unwrap();
        let err = locate(
            &tree,
            text,
            &[key("nodes"), Step::index(1), key("deps"), Step::index(0)],
        )
        .expect_err("refused");
        assert!(matches!(err, FlowEditError::FlowStyle(_)), "{err:?}");
    }

    #[test]
    fn a_missing_key_reports_where_it_would_go() {
        let tree = parse(SAMPLE).unwrap();
        let site = locate(
            &tree,
            SAMPLE,
            &[key("nodes"), Step::index(0), key("timeout")],
        )
        .unwrap();
        let Site::Vacant(v) = site else {
            panic!("expected a vacancy");
        };
        assert_eq!(v.container, Container::Mapping);
        assert_eq!(v.column, 4);
        assert_eq!(
            &SAMPLE[..v.at],
            &SAMPLE[..SAMPLE.find("\n  - id: build").unwrap()],
            "after the node's last field, not after the sequence"
        );
    }

    /// The trap this guards: without it, a path through a missing `agent:` came
    /// back as a vacancy in the *node*, and the caller would have written
    /// `id: …` there — the right shape one level too shallow. Unreachable from
    /// the differ, which never descends into a key the old side lacks, so the
    /// guard is here rather than there.
    #[test]
    fn a_path_through_a_key_the_file_lacks_is_refused_not_flattened() {
        let tree = parse(SAMPLE).unwrap();
        let err = locate(
            &tree,
            SAMPLE,
            &[key("nodes"), Step::index(0), key("agent"), key("id")],
        )
        .expect_err("refused");
        assert!(matches!(err, FlowEditError::Unaddressable(_)), "{err:?}");
    }

    /// A comment inside the mapping must not push the insertion point past it.
    #[test]
    fn an_insertion_point_stops_at_the_last_entry_not_the_last_comment() {
        let text = "a: 1\nb: 2\n# trailing note\n";
        let tree = parse(text).unwrap();
        let Site::Vacant(v) = locate(&tree, text, &[key("c")]).unwrap() else {
            panic!("expected a vacancy");
        };
        assert_eq!(&text[..v.at], "a: 1\nb: 2");
    }

    #[test]
    fn the_indent_step_is_measured_from_the_file() {
        assert_eq!(infer_indent(&parse(SAMPLE).unwrap()), 2);
        let four = "a:\n    b:\n        c: 1\n";
        assert_eq!(infer_indent(&parse(four).unwrap()), 4);
        assert_eq!(infer_indent(&parse("a: 1\n").unwrap()), 2, "nothing to see");
    }

    /// The rule an append stands on: a container's content ends at its last
    /// entry, even when the node itself runs to the newline after it.
    #[test]
    fn a_containers_content_ends_at_its_last_entry() {
        let f = found(SAMPLE, &[key("nodes")]);
        assert_eq!(f.kind, ValueKind::Sequence);
        assert_eq!(
            &SAMPLE[..f.content_end],
            &SAMPLE[..SAMPLE.len() - 1],
            "the trailing newline is not content"
        );
        assert_eq!(
            SAMPLE[f.content_end..].trim_start_matches('\n'),
            "",
            "and there is nothing after it but that newline"
        );
    }

    /// The gap between a key and a value written under it, which a value
    /// collapsing to one line has to reclaim.
    #[test]
    fn a_pair_reports_where_its_colon_ends() {
        let f = found(SAMPLE, &[key("defaults")]);
        let key_end = f.key_end.expect("a mapping pair has a colon");
        assert_eq!(&SAMPLE[..key_end], "version: 1\n# a comment\ndefaults:");
        assert!(
            SAMPLE[key_end..f.value.start].contains('\n'),
            "and this value is written under its key"
        );
    }

    #[test]
    fn a_line_span_takes_the_whole_line_and_its_newline() {
        let text = "a: 1\nb: 2\nc: 3\n";
        let span = line_span(text, &(5..9));
        assert_eq!(&text[span], "b: 2\n");
    }

    #[test]
    fn a_second_document_is_refused_rather_than_half_edited() {
        let text = "a: 1\n---\nb: 2\n";
        let tree = parse(text).unwrap();
        let err = locate(&tree, text, &[key("a")]).expect_err("refused");
        assert!(matches!(err, FlowEditError::Unaddressable(_)), "{err:?}");
    }
}
