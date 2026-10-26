//! Semantic changes between the old and updated flow value trees.

use yaml_serde::Value;

use super::locate::Step;

pub(super) fn changes(old: &Value, new: &Value) -> Vec<Change> {
    let mut changes = Vec::new();
    diff(old, new, &mut Vec::new(), &mut changes);
    changes
}

/// One thing that differs, and what to do about it. Three variants rather than
/// an `Option<Value>` plus a flag: "append to the sequence this path names" and
/// "replace the value this path names" are different instructions that would
/// otherwise be told apart by inspecting the value, which is a guess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Change {
    /// Replace the value at `path` — inserting the key when it is absent.
    Set { path: Vec<Step>, value: Value },
    /// Take the entry at `path` out.
    Remove { path: Vec<Step> },
    /// Add an element to the sequence at `path`.
    Append { path: Vec<Step>, value: Value },
}

impl Change {
    pub(super) fn path(&self) -> &[Step] {
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

#[cfg(test)]
mod tests {}
