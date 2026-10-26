//! Change one thing in a flow file and leave the rest of the bytes alone.
//!
//! The caller mutates a [`FlowFile`] — a typed struct — and gets back the list
//! of text edits that make the file say that. Nothing here enumerates
//! operations ("set this scalar", "add that node"): the old and new values are
//! compared key by key, and only the leaves that actually differ become edits.
//! Node addition and removal fall out of that comparison rather than being
//! separate features, and the form driving it never has to know YAML exists.
//!
//! Comments, key order and block style survive because the file is never
//! re-serialised. Zed's `settings_json` is the precedent for the shape; what
//! does not come from it is YAML itself — indentation is structure, so a removal
//! is a line removal, and a value may be a block scalar or written in flow style.
//! Flow style is refused rather than rewritten.
//!
//! Edits are returned rather than applied (D7 in the S4 design note): a caller
//! today splices them into a file, and a caller later can hand them to an
//! editor buffer as one transaction.

mod diff;
mod locate;
mod render;

use std::ops::Range;

use daruda_flow::parse::FlowFile;
use yaml_serde::Value;

use diff::Change;
use locate::{Container, Site, Step, ValueKind};

/// A replacement of `range` with this text. Ascending by `range.start`; a caller
/// applying them to a string does so in reverse so earlier offsets stay valid.
pub type Edit = (Range<usize>, String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowEditError {
    /// The text is not a flow file to begin with. Nothing can be edited safely
    /// in a file whose shape we cannot read.
    Unparsable(String),
    /// The value tree holds something no flow file can spell — a tagged value, a
    /// non-string key.
    Unrepresentable(String),
    /// The edit lands inside `{a: 1}` or `[a, b]`. Refused: rewriting flow style
    /// as block style, or splicing inside it, is a guess about what the person
    /// meant by writing it that way.
    FlowStyle(String),
    /// A `>` folded scalar. Rewriting one changes where the line breaks land, so
    /// it is refused rather than reflowed.
    FoldedScalar(String),
    /// The path exists in the value tree but not in the text — a multi-document
    /// file, an anchor, a shape this module does not model.
    Unaddressable(String),
}

impl std::fmt::Display for FlowEditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlowEditError::Unparsable(d) => write!(f, "not a flow file: {d}"),
            FlowEditError::Unrepresentable(d) => write!(f, "cannot be written as YAML: {d}"),
            FlowEditError::FlowStyle(d) => write!(f, "written in flow style: {d}"),
            FlowEditError::FoldedScalar(d) => write!(f, "a folded block scalar: {d}"),
            FlowEditError::Unaddressable(d) => write!(f, "cannot be located in the text: {d}"),
        }
    }
}

impl std::error::Error for FlowEditError {}

/// Apply `update` to the flow `text` describes and return the edits that make
/// the text agree with it.
///
/// An update that changes nothing returns no edits — the same rule the graph
/// pane's reload stands on, arrived at from the other side.
pub fn edits_for_update(
    text: &str,
    update: impl FnOnce(&mut FlowFile),
) -> Result<Vec<Edit>, FlowEditError> {
    let before = daruda_flow::parse::parse_flow_file(text)
        .map_err(|e| FlowEditError::Unparsable(e.to_string()))?;
    let mut after = before.clone();
    update(&mut after);
    if after == before {
        return Ok(Vec::new());
    }
    let old = to_value(&before)?;
    let new = to_value(&after)?;

    let changes = diff::changes(&old, &new);
    let edits = edits_for(text, &changes, &new)?;
    Ok(in_the_files_line_endings(text, edits))
}

/// The renderer writes `\n`; a file checked out with CRLF (`core.autocrlf`)
/// would come back holding both. The file's own ending wins, decided by
/// whether it has any CRLF at all.
fn in_the_files_line_endings(text: &str, edits: Vec<Edit>) -> Vec<Edit> {
    if !text.contains("\r\n") {
        return edits;
    }
    edits
        .into_iter()
        .map(|(range, replacement)| {
            (
                range,
                replacement.replace("\r\n", "\n").replace('\n', "\r\n"),
            )
        })
        .collect()
}

fn to_value(file: &FlowFile) -> Result<Value, FlowEditError> {
    yaml_serde::to_value(file).map_err(|e| FlowEditError::Unrepresentable(e.to_string()))
}

/// Turn every change into a text edit, in ascending order, with insertions into
/// the same slot merged.
fn edits_for(text: &str, changes: &[Change], new: &Value) -> Result<Vec<Edit>, FlowEditError> {
    let tree = locate::parse(text)?;
    let indent = locate::infer_indent(&tree);
    let mut edits: Vec<Edit> = Vec::new();
    for change in changes {
        let (range, replacement) = edit_for(text, &tree, indent, change, new)?;
        if range.is_empty() && replacement.is_empty() {
            continue;
        }
        // Two changes inside one flow-style list both resolve to replacing that
        // whole list, with the same replacement — the same edit twice, which
        // would read as an overlap.
        if edits.contains(&(range.clone(), replacement.clone())) {
            continue;
        }
        match edits.iter_mut().find(|(r, _)| *r == range && r.is_empty()) {
            // Two new fields in one mapping land on the same byte; one edit
            // holding both keeps the ranges non-overlapping.
            Some((_, existing)) => existing.push_str(&replacement),
            None => edits.push((range, replacement)),
        }
    }
    edits.sort_by_key(|(range, _)| range.start);
    fold_insertions_into_removals(&mut edits);
    if let Some(overlap) = first_overlap(&edits) {
        return Err(FlowEditError::Unaddressable(format!(
            "two edits overlap at {}..{}",
            overlap.start, overlap.end
        )));
    }
    Ok(edits)
}

/// An insertion anchored inside a line that is being removed **takes that
/// line's place**.
///
/// The anchor for a new key is the end of the mapping's last entry — and when
/// that entry is the one going away (a prompt becoming a `prompt_file`, say),
/// the two edits overlap and the whole change is refused. Folding them says what
/// was meant: the new key is written where the old one was.
fn fold_insertions_into_removals(edits: &mut Vec<Edit>) {
    let removals: Vec<Range<usize>> = edits
        .iter()
        .filter(|(range, text)| !range.is_empty() && text.is_empty())
        .map(|(range, _)| range.clone())
        .collect();
    let mut folded: Vec<(Range<usize>, String)> = Vec::new();
    edits.retain(|(range, text)| {
        if !range.is_empty() || text.is_empty() {
            return true;
        }
        let Some(removal) = removals
            .iter()
            .find(|removal| removal.contains(&range.start))
        else {
            return true;
        };
        // The insertion is written as "\n<line>"; taking a removed line's place
        // means dropping that leading newline and keeping the line's own.
        let line = text.strip_prefix('\n').unwrap_or(text);
        folded.push((removal.clone(), format!("{line}\n")));
        false
    });
    for (range, text) in folded {
        if let Some(slot) = edits.iter_mut().find(|(r, t)| *r == range && t.is_empty()) {
            slot.1 = text;
        }
    }
}

fn first_overlap(edits: &[Edit]) -> Option<Range<usize>> {
    edits.windows(2).find_map(|pair| {
        let (a, b) = (&pair[0].0, &pair[1].0);
        (b.start < a.end).then(|| a.clone())
    })
}

/// Refuse when `range` holds anything but whitespace.
///
/// An edit that replaces more than the value itself takes whatever is in
/// between with it, and what lives there is a comment. Deleting one is the
/// single thing this module promises never to do, so it stops instead.
fn prose_free(text: &str, range: std::ops::Range<usize>) -> Result<(), FlowEditError> {
    let between = &text[range];
    if between.trim().is_empty() {
        return Ok(());
    }
    Err(FlowEditError::Unaddressable(format!(
        "a comment inside the range this edit would replace:{between}"
    )))
}

/// Where a replacement may end: the value's content, not the node.
///
/// tree-sitter hangs a trailing comment at the container's own indentation off
/// the container, so the node ends past it while [`locate::Found::content_end`]
/// stops at the last real entry. Anything between the two is refused rather
/// than overwritten.
fn prose_free_end(text: &str, found: &locate::Found) -> Result<usize, FlowEditError> {
    prose_free(text, found.content_end..found.value.end)?;
    Ok(found.content_end)
}

fn edit_for(
    text: &str,
    tree: &tree_sitter::Tree,
    indent: usize,
    change: &Change,
    new: &Value,
) -> Result<Edit, FlowEditError> {
    // Asked first, because it changes what the edit *is*: a path that touches
    // something the file wrote as `[a, b]` cannot be spliced into, so the whole
    // value is replaced instead — in the style it was written in.
    if let Some(edit) = flow_value_edit(text, tree, change.path(), new)? {
        return Ok(edit);
    }
    match change {
        Change::Append { path, value } => {
            let Site::Found(sequence) = locate::locate(tree, text, path)? else {
                return Err(FlowEditError::Unaddressable(
                    "cannot append to a sequence that is not in the text".into(),
                ));
            };
            if sequence.kind != ValueKind::Sequence {
                return Err(FlowEditError::Unaddressable(
                    "cannot append to something that is not a sequence".into(),
                ));
            }
            let column = element_column(tree, text, path)?;
            let at = sequence.content_end;
            Ok((at..at, render::entry(None, value, column, indent)?))
        }
        Change::Set { path, value } => match locate::locate(tree, text, path)? {
            Site::Found(found) => {
                let rendered = render::value_at(value, found.kind, found.column, indent)?;
                // A value that was a block and is now one line: its own range
                // starts on the next line, so replacing only that would leave the
                // key and the value on separate lines. Take the gap between them.
                if let Some(key_end) = found.key_end
                    && !rendered.contains('\n')
                    && text[key_end..found.value.start].contains('\n')
                {
                    prose_free(text, key_end..found.value.start)?;
                    let end = prose_free_end(text, &found)?;
                    return Ok((key_end..end, format!(" {rendered}")));
                }
                // A block container's range starts at its first entry, *after*
                // that line's indentation — and what `render` produces carries
                // its own. Replacing from the line's start instead of doubling it.
                if matches!(found.kind, ValueKind::Mapping | ValueKind::Sequence) {
                    let from = text[..found.value.start].rfind('\n').map_or(0, |ix| ix + 1);
                    if text[from..found.value.start].trim().is_empty() {
                        let end = prose_free_end(text, &found)?;
                        return Ok((from..end, rendered));
                    }
                }
                Ok((found.value.clone(), rendered))
            }
            Site::Vacant(vacancy) => {
                let key = match (vacancy.container, path.last()) {
                    (Container::Mapping, Some(Step::Key(key))) => Some(key.as_str()),
                    (Container::Sequence, _) => None,
                    (Container::Mapping, _) => {
                        return Err(FlowEditError::Unaddressable(
                            "a mapping needs a key to insert under".into(),
                        ));
                    }
                };
                Ok((
                    vacancy.at..vacancy.at,
                    render::entry(key, value, vacancy.column, indent)?,
                ))
            }
        },
        Change::Remove { path } => match locate::locate(tree, text, path)? {
            Site::Found(found) => Ok((locate::line_span(text, &found.entry), String::new())),
            // Removing what is not written is not an error, and not an edit.
            Site::Vacant(vacancy) => Ok((vacancy.at..vacancy.at, String::new())),
        },
    }
}

/// The shortest prefix of `path` the text writes in flow style, replaced whole
/// from the new tree — or `None` when the path touches no such value.
///
/// Shortest first: the outermost flow-style value is the one that can still be
/// addressed, and anything below it is inside a value this module refuses to
/// step into. A prefix the new tree has nothing at is a removal of the value
/// itself, which is a line removal and not this.
fn flow_value_edit(
    text: &str,
    tree: &tree_sitter::Tree,
    path: &[Step],
    new: &Value,
) -> Result<Option<Edit>, FlowEditError> {
    for len in 1..=path.len() {
        let prefix = &path[..len];
        match locate::locate(tree, text, prefix)? {
            Site::Found(found) if found.kind == ValueKind::FlowSequence => {
                let Some(value) = value_at_path(new, prefix) else {
                    return Ok(None);
                };
                return Ok(Some((found.value.clone(), render::flow_sequence(value)?)));
            }
            Site::Found(_) => continue,
            // Nothing deeper is written yet, so nothing deeper is flow style.
            Site::Vacant(_) => return Ok(None),
        }
    }
    Ok(None)
}

/// Follow `path` into a value tree.
fn value_at_path<'a>(value: &'a Value, path: &[Step]) -> Option<&'a Value> {
    let mut current = value;
    for step in path {
        current = match step {
            Step::Key(key) => current.get(key.as_str())?,
            Step::Index { value, .. } => current.as_sequence()?.get((*value)?)?,
        };
    }
    Some(current)
}

/// The column a sequence's elements start their `-` in — read off the first one
/// so a new element lines up with its siblings whatever the file's style is.
fn element_column(
    tree: &tree_sitter::Tree,
    text: &str,
    path: &[Step],
) -> Result<usize, FlowEditError> {
    let mut probe = path.to_vec();
    probe.push(Step::index(0));
    Ok(match locate::locate(tree, text, &probe)? {
        Site::Found(found) => found.column,
        Site::Vacant(vacancy) => vacancy.column,
    })
}

/// Apply `edits` to `text`. The one place that knows they come back ascending
/// and have to be applied the other way round.
pub fn apply(text: &str, edits: &[Edit]) -> String {
    let mut out = text.to_string();
    for (range, replacement) in edits.iter().rev() {
        out.replace_range(range.clone(), replacement);
    }
    out
}

#[cfg(test)]
mod tests;
