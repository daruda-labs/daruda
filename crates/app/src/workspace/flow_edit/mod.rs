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
//! Edits are returned rather than applied ([D7]): a caller today splices them
//! into a file, and a caller later can hand them to an editor buffer as one
//! transaction.
//!
//! [D7]: the S4 design note

mod locate;
mod render;

use std::ops::Range;

use daruda_flow::parse::FlowFile;
use yaml_serde::Value;

use locate::{Container, Site, Step, ValueKind};

/// A replacement of `range` with this text. Ascending by `range.start`; a caller
/// applying them to a string does so in reverse so earlier offsets stay valid.
pub(in crate::workspace) type Edit = (Range<usize>, String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum FlowEditError {
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
pub(in crate::workspace) fn edits_for_update(
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

    let mut changes = Vec::new();
    diff(&old, &new, &mut Vec::new(), &mut changes);
    edits_for(text, &changes, &new)
}

fn to_value(file: &FlowFile) -> Result<Value, FlowEditError> {
    yaml_serde::to_value(file).map_err(|e| FlowEditError::Unrepresentable(e.to_string()))
}

/// One thing that differs, and what to do about it. Three variants rather than
/// an `Option<Value>` plus a flag: "append to the sequence this path names" and
/// "replace the value this path names" are different instructions that would
/// otherwise be told apart by inspecting the value, which is a guess.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Change {
    /// Replace the value at `path` — inserting the key when it is absent.
    Set { path: Vec<Step>, value: Value },
    /// Take the entry at `path` out.
    Remove { path: Vec<Step> },
    /// Add an element to the sequence at `path`.
    Append { path: Vec<Step>, value: Value },
}

impl Change {
    fn path(&self) -> &[Step] {
        match self {
            Change::Set { path, .. } | Change::Remove { path } | Change::Append { path, .. } => {
                path
            }
        }
    }
}

/// Walk both trees together, recording the narrowest thing that differs.
///
/// Narrowest matters: rewriting a whole mapping because one of its fields
/// changed would take every comment inside it with it.
fn diff(old: &Value, new: &Value, path: &mut Vec<Step>, out: &mut Vec<Change>) {
    if old == new {
        return;
    }
    match (old, new) {
        (Value::Mapping(a), Value::Mapping(b)) => {
            for (key, new_value) in b {
                let Some(key) = key.as_str() else { continue };
                path.push(Step::Key(key.to_string()));
                match a.get(key) {
                    Some(old_value) => diff(old_value, new_value, path, out),
                    None => out.push(Change::Set {
                        path: path.clone(),
                        value: new_value.clone(),
                    }),
                }
                path.pop();
            }
            for (key, _) in a {
                let Some(key) = key.as_str() else { continue };
                if b.get(key).is_none() {
                    path.push(Step::Key(key.to_string()));
                    out.push(Change::Remove { path: path.clone() });
                    path.pop();
                }
            }
        }
        (Value::Sequence(a), Value::Sequence(b)) => diff_sequence(a, b, path, out),
        _ => out.push(Change::Set {
            path: path.clone(),
            value: new.clone(),
        }),
    }
}

/// Sequences whose elements carry an `id` are matched **by that id**.
///
/// Equality is not enough to pair them: deleting a node also takes it out of
/// what pointed at it, so a surviving element changes in the same breath — and
/// then neither the prefix nor the suffix matches, and the whole sequence gets
/// rewritten. Which is how a deletion came to reformat every node in the file,
/// comments and one-line lists included.
///
/// An `id` is what a flow file identifies a node by (`deps` and `rerun` name it),
/// so this is not the differ guessing — it is the same key the data uses.
/// Sequences without one fall back to matching what did not move.
fn diff_sequence(a: &[Value], b: &[Value], path: &mut Vec<Step>, out: &mut Vec<Change>) {
    // Only when the length changed. At equal length nothing moved, and pairing by
    // id would read a **rename** as one element removed and another added — which
    // would take the node's lines out and add them back at the end of the file.
    if a.len() != b.len()
        && let (Some(old_ids), Some(new_ids)) = (element_ids(a), element_ids(b))
    {
        diff_keyed_sequence(a, b, &old_ids, &new_ids, path, out);
        return;
    }
    diff_positional_sequence(a, b, path, out)
}

/// Every element's `id`, when every element is a mapping with a distinct one.
fn element_ids(seq: &[Value]) -> Option<Vec<String>> {
    let ids: Vec<String> = seq
        .iter()
        .map(|item| item.get("id")?.as_str().map(str::to_string))
        .collect::<Option<Vec<_>>>()?;
    let distinct: std::collections::HashSet<&String> = ids.iter().collect();
    (distinct.len() == ids.len()).then_some(ids)
}

/// Pair by id: what stayed is compared in place, what went away is removed by
/// the index it had, and what is new is appended.
fn diff_keyed_sequence(
    a: &[Value],
    b: &[Value],
    old_ids: &[String],
    new_ids: &[String],
    path: &mut Vec<Step>,
    out: &mut Vec<Change>,
) {
    for (old_ix, id) in old_ids.iter().enumerate() {
        if let Some(new_ix) = new_ids.iter().position(|new| new == id) {
            path.push(Step::Index {
                text: old_ix,
                value: Some(new_ix),
            });
            diff(&a[old_ix], &b[new_ix], path, out);
            path.pop();
        }
    }
    // Removals highest-index first: every edit is against the original text, and
    // descending keeps the ranges from crossing.
    for (old_ix, id) in old_ids.iter().enumerate().rev() {
        if !new_ids.iter().any(|new| new == id) {
            // Nowhere in the new tree: this is the element being removed.
            path.push(Step::Index {
                text: old_ix,
                value: None,
            });
            out.push(Change::Remove { path: path.clone() });
            path.pop();
        }
    }
    for (new_ix, id) in new_ids.iter().enumerate() {
        if !old_ids.iter().any(|old| old == id) {
            out.push(Change::Append {
                path: path.clone(),
                value: b[new_ix].clone(),
            });
        }
    }
}

fn diff_positional_sequence(a: &[Value], b: &[Value], path: &mut Vec<Step>, out: &mut Vec<Change>) {
    let prefix = a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count();
    let max_suffix = a.len().min(b.len()) - prefix;
    let suffix = (0..max_suffix)
        .take_while(|i| a[a.len() - 1 - i] == b[b.len() - 1 - i])
        .count();

    let old_middle = prefix..a.len() - suffix;
    let new_middle = prefix..b.len() - suffix;

    if old_middle.len() == new_middle.len() {
        for (old_ix, new_ix) in old_middle.zip(new_middle) {
            path.push(Step::index(old_ix));
            diff(&a[old_ix], &b[new_ix], path, out);
            path.pop();
        }
        return;
    }
    if old_middle.is_empty() {
        // Pure addition. Appended to the end of the sequence whatever the
        // person's intended position was: placing an element between two
        // existing ones means splicing lines in the middle of a block, and
        // execution order comes from `deps`, not from the file's order.
        for ix in new_middle {
            out.push(Change::Append {
                path: path.clone(),
                value: b[ix].clone(),
            });
        }
        return;
    }
    if new_middle.is_empty() {
        // Pure removal. Highest index first: every edit is against the original
        // text, so the order only has to be one the caller can apply, and
        // descending keeps the ranges from crossing.
        for ix in old_middle.rev() {
            // `value: None` for the same reason the keyed path uses it: the
            // element is the one going away, so it has no index in the new tree
            // — and claiming it has one is how a removal turns into a write of
            // whatever now sits at that index.
            path.push(Step::Index {
                text: ix,
                value: None,
            });
            out.push(Change::Remove { path: path.clone() });
            path.pop();
        }
        return;
    }
    // Both sides changed length *and* content — a rewrite of the whole
    // sequence is the only honest answer, and it is one edit rather than a
    // guess at which element became which.
    out.push(Change::Set {
        path: path.clone(),
        value: Value::Sequence(b.to_vec()),
    });
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
pub(in crate::workspace) fn apply(text: &str, edits: &[Edit]) -> String {
    let mut out = text.to_string();
    for (range, replacement) in edits.iter().rev() {
        out.replace_range(range.clone(), replacement);
    }
    out
}

#[cfg(test)]
mod tests;
