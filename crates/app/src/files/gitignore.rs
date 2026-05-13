//! Per-worktree gitignore matcher for the sidebar Files view.
//!
//! Supports nested `.gitignore` files: each subdirectory that contains a
//! `.gitignore` gets its own `Gitignore` object anchored to that directory.
//! When checking a path, only the matchers whose directory is an ancestor of
//! the path are consulted, so a rule in `packages/ui/.gitignore` never leaks
//! out of `packages/ui/`.
//!
//! The discovery walk skips any subtree that is already ignored by the
//! root-level rules (`.gitignore` + `.git/info/exclude`) **or** by any
//! intermediate nested matcher collected so far, so a `dist/` rule in
//! `packages/.gitignore` prevents descending into `packages/dist/`.
//!
//! Evaluation order in `is_ignored` goes from most-specific (deepest
//! subdirectory) to least-specific (root), so a `!negation` pattern in a
//! subdirectory's `.gitignore` can un-ignore a path that a parent rule would
//! otherwise ignore — matching real Git semantics.

use std::path::{Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// Maximum directory depth `collect_nested` will descend into.
/// Prevents stack overflow on pathological trees (deep symlink cycles,
/// unusually nested vendor directories, etc.).
const MAX_COLLECT_DEPTH: usize = 50;

/// Compiled gitignore rules for one worktree.
///
/// Cheap to evaluate per row; rebuilt only when any `.gitignore` file in
/// the tree changes.
pub struct GitignoreSet {
    /// Rules from the worktree root (`.gitignore` + `.git/info/exclude`).
    root: Gitignore,
    /// Per-subdirectory matchers: `(dir, matcher)`.
    /// Each matcher is anchored to its own `dir` and is only applied when
    /// the query path starts with `dir`. Stored outer → inner (DFS order).
    nested: Vec<(PathBuf, Gitignore)>,
}

impl GitignoreSet {
    /// Build the matcher for `root`. Discovers nested `.gitignore` files
    /// while skipping subtrees already ignored at the root level or by any
    /// intermediate nested matcher.
    pub fn build(root: &Path) -> Self {
        let mut rb = GitignoreBuilder::new(root);
        let _ = rb.add(root.join(".gitignore"));
        let _ = rb.add(root.join(".git").join("info").join("exclude"));
        let root_gi = rb
            .build()
            .unwrap_or_else(|_| GitignoreBuilder::new(root).build().expect("empty"));

        let mut nested = Vec::new();
        collect_nested(root, &root_gi, &mut nested, 0);

        Self {
            root: root_gi,
            nested,
        }
    }

    /// `true` when `abs_path` is matched by an ignore rule and not
    /// whitelisted by a negation (`!pattern`).
    ///
    /// Evaluates from the most-specific (deepest subdirectory) matcher to
    /// the least-specific (root). The first matcher that returns `Ignore`
    /// or `Whitelist` wins, so a subdirectory negation overrides a parent
    /// ignore rule — matching real Git semantics.
    pub fn is_ignored(&self, abs_path: &Path, is_dir: bool) -> bool {
        // Build the list of applicable matchers ordered outer → inner
        // (root first, then nested by DFS insertion order = outer first).
        let applicable: Vec<&Gitignore> = std::iter::once(&self.root)
            .chain(
                self.nested
                    .iter()
                    .filter(|(dir, _)| abs_path.starts_with(dir))
                    .map(|(_, gi)| gi),
            )
            .collect();

        // Evaluate most-specific first so that a `!negation` in a
        // subdirectory can un-ignore a path an ancestor rule would ignore.
        for gi in applicable.iter().rev() {
            match gi.matched_path_or_any_parents(abs_path, is_dir) {
                ignore::Match::Ignore(_) => return true,
                ignore::Match::Whitelist(_) => return false,
                ignore::Match::None => {}
            }
        }
        false
    }
}

/// Recursively visit `dir`, building a per-directory `Gitignore` for every
/// subdirectory that has a `.gitignore` file, and appending it to `out`.
///
/// Subtrees already ignored by `root_gi` **or** by any ancestor nested
/// matcher already in `out` are skipped, preventing descent into directories
/// like `packages/dist/` when `packages/.gitignore` contains `dist/`.
///
/// **Ordering invariant**: a directory's own `.gitignore` is pushed into
/// `out` before `collect_nested` recurses into it. This guarantees that the
/// ancestor-pruning check for any child directory can always find the
/// parent's matcher in `out`. The check relies on the ancestor `starts_with`
/// guard: sibling directories are never in each other's `starts_with`
/// relationship, so their relative insertion order does not matter.
///
/// `depth` guards against stack overflow on pathological trees.
fn collect_nested(
    dir: &Path,
    root_gi: &Gitignore,
    out: &mut Vec<(PathBuf, Gitignore)>,
    depth: usize,
) {
    if depth >= MAX_COLLECT_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_dir() {
            continue;
        }
        // Skip subtrees the root-level gitignore already ignores.
        if matches!(
            root_gi.matched_path_or_any_parents(&path, true),
            ignore::Match::Ignore(_)
        ) {
            continue;
        }
        // Skip subtrees ignored by any ancestor nested matcher already
        // collected. This prevents loading a `.gitignore` that lives inside
        // a subtree that an outer rule has already marked as ignored.
        let pruned_by_nested = out
            .iter()
            .filter(|(ancestor_dir, _)| path.starts_with(ancestor_dir))
            .any(|(_, gi)| {
                matches!(
                    gi.matched_path_or_any_parents(&path, true),
                    ignore::Match::Ignore(_)
                )
            });
        if pruned_by_nested {
            continue;
        }
        let gi_path = path.join(".gitignore");
        if gi_path.is_file() {
            let mut b = GitignoreBuilder::new(&path);
            let _ = b.add(&gi_path);
            if let Ok(gi) = b.build() {
                out.push((path.clone(), gi));
            }
        }
        collect_nested(&path, root_gi, out, depth + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn root_gitignore_matches_target_dir() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join(".gitignore"), "target/\n").unwrap();
        let set = GitignoreSet::build(root);
        assert!(set.is_ignored(&root.join("target"), true));
        assert!(set.is_ignored(&root.join("target/debug/foo"), false));
        assert!(!set.is_ignored(&root.join("src/lib.rs"), false));
    }

    #[test]
    fn root_gitignore_respects_negation() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join(".gitignore"), "*.log\n!keep.log\n").unwrap();
        let set = GitignoreSet::build(root);
        assert!(set.is_ignored(&root.join("debug.log"), false));
        assert!(!set.is_ignored(&root.join("keep.log"), false));
    }

    #[test]
    fn missing_gitignore_returns_unignored() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let set = GitignoreSet::build(root);
        assert!(!set.is_ignored(&root.join("anything"), false));
        assert!(!set.is_ignored(&root.join("any/dir"), true));
    }

    #[test]
    fn nested_gitignore_applies_inside_subdir() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        fs::create_dir_all(root.join("packages/ui")).unwrap();
        fs::write(root.join("packages/ui/.gitignore"), "*.generated.ts\n").unwrap();

        let set = GitignoreSet::build(root);

        // Root rule fires everywhere.
        assert!(set.is_ignored(&root.join("debug.log"), false));
        // Nested rule fires only inside its directory.
        assert!(set.is_ignored(&root.join("packages/ui/api.generated.ts"), false));
        // Nested rule does NOT fire at the root level.
        assert!(!set.is_ignored(&root.join("api.generated.ts"), false));
        // Nested rule does NOT fire in a sibling directory.
        assert!(!set.is_ignored(&root.join("packages/core/api.generated.ts"), false));
    }

    #[test]
    fn does_not_load_gitignore_inside_ignored_dir() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join(".gitignore"), "target/\n").unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        // Negation inside the already-ignored directory must be ignored.
        fs::write(root.join("target/.gitignore"), "!important\n").unwrap();
        fs::write(root.join("target/debug/binary"), "").unwrap();

        let set = GitignoreSet::build(root);
        assert!(set.is_ignored(&root.join("target"), true));
        assert!(set.is_ignored(&root.join("target/debug/binary"), false));
    }

    #[test]
    fn nested_negation_overrides_root_rule() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        // Root ignores all .log files.
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        fs::create_dir_all(root.join("packages/ui")).unwrap();
        // packages/ui un-ignores debug.log specifically.
        fs::write(root.join("packages/ui/.gitignore"), "!debug.log\n").unwrap();

        let set = GitignoreSet::build(root);

        // Root rule still fires outside packages/ui.
        assert!(set.is_ignored(&root.join("app.log"), false));
        assert!(set.is_ignored(&root.join("packages/debug.log"), false));
        // Nested negation overrides root rule inside packages/ui.
        assert!(!set.is_ignored(&root.join("packages/ui/debug.log"), false));
        // Other .log files inside packages/ui are still ignored.
        assert!(set.is_ignored(&root.join("packages/ui/error.log"), false));
    }

    #[test]
    fn nested_gitignore_inside_nested_ignored_subtree_is_not_loaded() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        // packages/.gitignore ignores the dist/ subdirectory.
        fs::create_dir_all(root.join("packages/dist")).unwrap();
        fs::write(root.join("packages/.gitignore"), "dist/\n").unwrap();
        // packages/dist/.gitignore tries to un-ignore something — must not be loaded.
        fs::write(root.join("packages/dist/.gitignore"), "!app.js\n").unwrap();

        let set = GitignoreSet::build(root);

        // dist/ itself is ignored.
        assert!(set.is_ignored(&root.join("packages/dist"), true));
        // app.js inside dist/ remains ignored despite the inner negation
        // (the inner .gitignore must not have been loaded).
        assert!(set.is_ignored(&root.join("packages/dist/app.js"), false));
    }
}
