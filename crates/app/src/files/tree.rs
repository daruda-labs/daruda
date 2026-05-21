//! Per-lane file tree data structure.
//!
//! Each `Entry` is a node in a flat HashMap keyed by `EntryId`, with a
//! parallel `path → id` index and a `parent_id → child_ids` index. The
//! lazy state machine `UnloadedDir → PendingDir → Dir` mirrors Zed's
//! `EntryKind`: an unloaded directory shows up as a row but its
//! children are read on demand. `expanded` is a sorted `Vec<EntryId>`
//! so toggling is O(log n) and stale ids are easy to drop after
//! `remove_subtree`.
//!
//! The module is GPUI-free; callers feed `LoadedEntry` records (one
//! per direct child) produced by the blocking `crate::files::load`
//! helper into `apply_dir_load` to materialise the next layer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Per-lane entry id. Monotonically allocated; never reused after
/// removal so stale references in `expanded` or external caches can be
/// dropped without dangling.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Debug)]
pub struct EntryId(pub u32);

#[derive(Clone, Copy, Eq, PartialEq, Debug)]
pub enum EntryKind {
    /// Directory whose children have not been read yet. Click expands.
    UnloadedDir,
    /// Directory whose load task is in flight. Renders a spinner.
    PendingDir,
    /// Directory with children loaded.
    Dir,
    File,
}

impl EntryKind {
    pub fn is_dir(&self) -> bool {
        matches!(self, Self::UnloadedDir | Self::PendingDir | Self::Dir)
    }
    pub fn is_loaded(&self) -> bool {
        matches!(self, Self::Dir | Self::File)
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: EntryId,
    pub kind: EntryKind,
    /// Lane-root-relative path. The root entry has an empty path.
    pub path: PathBuf,
    /// `path.file_name()` cached. Empty for the root.
    pub name: String,
    /// `name.to_lowercase()` cached so sort comparisons skip per-call
    /// allocation. Recomputed on rename.
    pub name_lower: String,
    pub is_symlink: bool,
    /// `name.starts_with('.')`. Drives the "dotfile last" sort group.
    pub is_hidden: bool,
    /// Filled by gitignore evaluation (W-7j). Default false.
    pub is_ignored: bool,
}

#[derive(Debug)]
pub enum FileTreeError {
    Io(std::io::Error),
    NotADir,
    NotFound,
    PermissionDenied,
}

impl std::fmt::Display for FileTreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::NotADir => write!(f, "not a directory"),
            Self::NotFound => write!(f, "not found"),
            Self::PermissionDenied => write!(f, "permission denied"),
        }
    }
}

impl std::error::Error for FileTreeError {}

/// One entry per direct child returned by a directory read. The tree
/// converts these into `Entry` values with fresh ids inside
/// `apply_dir_load`.
#[derive(Clone, Debug)]
pub struct LoadedEntry {
    pub name: String,
    /// Always `UnloadedDir` or `File` at this stage.
    pub kind: EntryKind,
    pub is_symlink: bool,
}

pub struct FileTree {
    /// Absolute path of the lane root.
    pub root: PathBuf,
    pub root_id: EntryId,
    entries: HashMap<EntryId, Entry>,
    path_to_id: HashMap<PathBuf, EntryId>,
    /// parent id → ordered child ids (sorted by `sort_children`).
    children: HashMap<EntryId, Vec<EntryId>>,
    /// Sorted vec of expanded directory ids. binary_search for toggle.
    expanded: Vec<EntryId>,
    next_id: u32,
    /// Set when an inactive lane receives a watcher event. Cleared
    /// on activation; activation triggers a Bulk reload if true.
    pub dirty: bool,
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        let root_id = EntryId(0);
        let mut entries = HashMap::new();
        let mut path_to_id = HashMap::new();
        let mut children = HashMap::new();
        let root_entry = Entry {
            id: root_id,
            kind: EntryKind::UnloadedDir,
            path: PathBuf::new(),
            name: String::new(),
            name_lower: String::new(),
            is_symlink: false,
            is_hidden: false,
            is_ignored: false,
        };
        entries.insert(root_id, root_entry);
        path_to_id.insert(PathBuf::new(), root_id);
        children.insert(root_id, Vec::new());
        Self {
            root,
            root_id,
            entries,
            path_to_id,
            children,
            expanded: Vec::new(),
            next_id: 1,
            dirty: false,
        }
    }

    pub fn entry(&self, id: EntryId) -> Option<&Entry> {
        self.entries.get(&id)
    }

    pub fn entry_mut(&mut self, id: EntryId) -> Option<&mut Entry> {
        self.entries.get_mut(&id)
    }

    pub fn entry_for_path(&self, path: &Path) -> Option<&Entry> {
        self.path_to_id
            .get(path)
            .and_then(|id| self.entries.get(id))
    }

    pub fn id_for_path(&self, path: &Path) -> Option<EntryId> {
        self.path_to_id.get(path).copied()
    }

    pub fn child_ids(&self, parent: EntryId) -> &[EntryId] {
        self.children
            .get(&parent)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn child_entries(&self, parent: EntryId) -> impl Iterator<Item = &Entry> {
        self.child_ids(parent)
            .iter()
            .filter_map(|id| self.entries.get(id))
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Toggle expanded state and return the new state. `expanded` stays
    /// sorted so subsequent binary_search keeps O(log n).
    pub fn toggle_expand(&mut self, id: EntryId) -> bool {
        match self.expanded.binary_search(&id) {
            Ok(pos) => {
                self.expanded.remove(pos);
                false
            }
            Err(pos) => {
                self.expanded.insert(pos, id);
                true
            }
        }
    }

    pub fn is_expanded(&self, id: EntryId) -> bool {
        self.expanded.binary_search(&id).is_ok()
    }

    pub fn expanded_ids(&self) -> &[EntryId] {
        &self.expanded
    }

    /// Walk the subtree rooted at `id`, excluding `id` itself. Order is
    /// pre-order over the `children` index (not sorted).
    pub fn descendants(&self, id: EntryId) -> Vec<EntryId> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            if let Some(children) = self.children.get(&cur) {
                for &child in children {
                    out.push(child);
                    stack.push(child);
                }
            }
        }
        out
    }

    /// Allocate a fresh id and insert one new child under `parent_id`.
    /// Caller is expected to call `sort_children(parent_id)` once after
    /// a batch of inserts. Panics if `parent_id` does not exist or
    /// `parent_id` is a non-directory.
    pub fn insert_child(&mut self, parent_id: EntryId, loaded: LoadedEntry) -> EntryId {
        let parent = self
            .entries
            .get(&parent_id)
            .expect("insert_child: parent missing");
        debug_assert!(parent.kind.is_dir(), "insert_child: parent is not a dir");

        let id = EntryId(self.next_id);
        self.next_id = self.next_id.checked_add(1).expect("EntryId overflow (u32)");
        let path = parent.path.join(&loaded.name);
        let name_lower = loaded.name.to_lowercase();
        let is_hidden = loaded.name.starts_with('.');
        let is_dir = loaded.kind.is_dir();

        let entry = Entry {
            id,
            kind: loaded.kind,
            path: path.clone(),
            name: loaded.name,
            name_lower,
            is_symlink: loaded.is_symlink,
            is_hidden,
            is_ignored: false,
        };
        self.entries.insert(id, entry);
        self.path_to_id.insert(path, id);
        if is_dir {
            self.children.insert(id, Vec::new());
        }
        self.children
            .get_mut(&parent_id)
            .expect("parent children list missing")
            .push(id);
        id
    }

    /// Re-sort the children of `parent_id` per the global rule:
    /// directories first, then dotfiles last within each group, then
    /// case-insensitive alpha (using the cached `name_lower`), with
    /// original-case as tie-break.
    pub fn sort_children(&mut self, parent_id: EntryId) {
        let mut child_ids = match self.children.get(&parent_id) {
            Some(c) => c.clone(),
            None => return,
        };
        let entries = &self.entries;
        child_ids.sort_by(|a, b| {
            let ea = &entries[a];
            let eb = &entries[b];
            // Directories first.
            eb.kind
                .is_dir()
                .cmp(&ea.kind.is_dir())
                // Hidden last within group.
                .then(ea.is_hidden.cmp(&eb.is_hidden))
                // Case-insensitive (cached).
                .then_with(|| ea.name_lower.cmp(&eb.name_lower))
                // Tie-break: original case, stable.
                .then_with(|| ea.name.cmp(&eb.name))
        });
        self.children.insert(parent_id, child_ids);
    }

    /// Remove the subtree rooted at `path` and clean up the path index,
    /// children index, expanded set, and the parent's child list. The
    /// root path is never removed via this API.
    pub fn remove_subtree(&mut self, path: &Path) {
        let Some(&id) = self.path_to_id.get(path) else {
            return;
        };
        if id == self.root_id {
            return;
        }

        let mut victims = vec![id];
        victims.extend(self.descendants(id));

        for &rm_id in &victims {
            if let Some(entry) = self.entries.remove(&rm_id) {
                self.path_to_id.remove(&entry.path);
            }
            self.children.remove(&rm_id);
            if let Ok(pos) = self.expanded.binary_search(&rm_id) {
                self.expanded.remove(pos);
            }
        }

        let parent_path = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        if let Some(&parent_id) = self.path_to_id.get(&parent_path)
            && let Some(siblings) = self.children.get_mut(&parent_id)
        {
            let victims_set: std::collections::HashSet<EntryId> = victims.iter().copied().collect();
            siblings.retain(|sib| !victims_set.contains(sib));
        }
    }

    /// Apply a fresh directory listing to `parent_id`, diffing against
    /// the existing children: new names are inserted, missing names are
    /// removed (subtree included). The parent's `kind` flips from
    /// `PendingDir` → `Dir`. The children list is re-sorted at the end.
    ///
    /// Returns `(added, removed)` so callers can wire cache
    /// invalidations or watcher follow-up.
    pub fn apply_dir_load(
        &mut self,
        parent_id: EntryId,
        loaded: Vec<LoadedEntry>,
    ) -> (Vec<EntryId>, Vec<PathBuf>) {
        // Existing names under this parent.
        let existing: HashMap<String, EntryId> = self
            .child_ids(parent_id)
            .iter()
            .filter_map(|&id| self.entries.get(&id).map(|e| (e.name.clone(), id)))
            .collect();
        let new_names: std::collections::HashSet<&str> =
            loaded.iter().map(|l| l.name.as_str()).collect();

        // Remove children whose name is no longer present.
        let mut removed_paths = Vec::new();
        let to_remove: Vec<PathBuf> = existing
            .iter()
            .filter(|(name, _)| !new_names.contains(name.as_str()))
            .filter_map(|(_, id)| self.entries.get(id).map(|e| e.path.clone()))
            .collect();
        for p in to_remove {
            removed_paths.push(p.clone());
            self.remove_subtree(&p);
        }

        // Insert names that didn't exist before.
        let mut added_ids = Vec::new();
        for l in loaded {
            if !existing.contains_key(&l.name) {
                let new_id = self.insert_child(parent_id, l);
                added_ids.push(new_id);
            }
        }

        // Flip PendingDir/UnloadedDir → Dir on the parent itself.
        if let Some(parent) = self.entries.get_mut(&parent_id)
            && matches!(parent.kind, EntryKind::PendingDir | EntryKind::UnloadedDir)
        {
            parent.kind = EntryKind::Dir;
        }

        self.sort_children(parent_id);
        (added_ids, removed_paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded(name: &str, kind: EntryKind) -> LoadedEntry {
        LoadedEntry {
            name: name.to_string(),
            kind,
            is_symlink: false,
        }
    }

    fn fresh_tree() -> FileTree {
        FileTree::new(PathBuf::from("/tmp/wt"))
    }

    #[test]
    fn entry_id_assignment_monotonic() {
        let mut t = fresh_tree();
        let root = t.root_id;
        let a = t.insert_child(root, loaded("a", EntryKind::File));
        let b = t.insert_child(root, loaded("b", EntryKind::File));
        let c = t.insert_child(root, loaded("c", EntryKind::File));
        assert_eq!(a, EntryId(1));
        assert_eq!(b, EntryId(2));
        assert_eq!(c, EntryId(3));
        // Removing one and adding a new entry must not reuse the id.
        t.remove_subtree(&PathBuf::from("b"));
        let d = t.insert_child(root, loaded("d", EntryKind::File));
        assert_eq!(d, EntryId(4), "ids must be monotonic, never reused");
    }

    #[test]
    fn toggle_expand_inserts_sorted() {
        let mut t = fresh_tree();
        assert!(t.toggle_expand(EntryId(5)));
        assert!(t.toggle_expand(EntryId(2)));
        assert!(t.toggle_expand(EntryId(8)));
        // expanded must stay sorted for binary_search to work.
        assert_eq!(t.expanded_ids(), &[EntryId(2), EntryId(5), EntryId(8)]);
    }

    #[test]
    fn toggle_expand_removes_idempotent() {
        let mut t = fresh_tree();
        assert!(t.toggle_expand(EntryId(5)));
        assert!(!t.toggle_expand(EntryId(5)));
        assert!(t.expanded_ids().is_empty());
        // Toggling an id that was never set is also a no-op insertion.
        assert!(t.toggle_expand(EntryId(7)));
        assert!(!t.toggle_expand(EntryId(7)));
        assert!(t.expanded_ids().is_empty());
    }

    #[test]
    fn child_entries_returns_direct_only() {
        let mut t = fresh_tree();
        let root = t.root_id;
        let a = t.insert_child(root, loaded("a", EntryKind::UnloadedDir));
        let b = t.insert_child(root, loaded("b", EntryKind::File));
        let _aa = t.insert_child(a, loaded("aa", EntryKind::File));
        let names: Vec<&str> = t.child_entries(root).map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
        let _ = b;
    }

    #[test]
    fn child_entries_excludes_grandchildren() {
        let mut t = fresh_tree();
        let root = t.root_id;
        let a = t.insert_child(root, loaded("a", EntryKind::UnloadedDir));
        let aa = t.insert_child(a, loaded("aa", EntryKind::UnloadedDir));
        let _aaa = t.insert_child(aa, loaded("aaa", EntryKind::File));
        let direct: Vec<&str> = t.child_entries(a).map(|e| e.name.as_str()).collect();
        assert_eq!(direct, vec!["aa"]);
        // aaa is a grandchild of `a`; must not appear.
        assert!(!direct.contains(&"aaa"));
    }

    #[test]
    fn remove_subtree_cleans_descendants_and_expanded() {
        let mut t = fresh_tree();
        let root = t.root_id;
        let a = t.insert_child(root, loaded("a", EntryKind::UnloadedDir));
        let aa = t.insert_child(a, loaded("aa", EntryKind::UnloadedDir));
        let aaa = t.insert_child(aa, loaded("aaa", EntryKind::File));
        // Mark some ids as expanded to verify cleanup.
        t.toggle_expand(a);
        t.toggle_expand(aa);

        t.remove_subtree(&PathBuf::from("a"));

        // All descendants gone from every index.
        assert!(t.entry(a).is_none());
        assert!(t.entry(aa).is_none());
        assert!(t.entry(aaa).is_none());
        assert!(t.id_for_path(&PathBuf::from("a")).is_none());
        assert!(t.id_for_path(&PathBuf::from("a/aa")).is_none());
        assert!(t.id_for_path(&PathBuf::from("a/aa/aaa")).is_none());
        // expanded set fully cleared (only descendants of `a` were in it).
        assert!(t.expanded_ids().is_empty());
        // Parent's child list no longer references `a`.
        assert!(t.child_ids(root).is_empty());
    }

    #[test]
    fn sort_dirs_first() {
        let mut t = fresh_tree();
        let root = t.root_id;
        // Insert a mix; `insert_child` doesn't sort, the explicit
        // `sort_children` call models what `apply_dir_load` does.
        t.insert_child(root, loaded("zfile", EntryKind::File));
        t.insert_child(root, loaded("adir", EntryKind::UnloadedDir));
        t.insert_child(root, loaded("bfile", EntryKind::File));
        t.insert_child(root, loaded("cdir", EntryKind::UnloadedDir));
        t.sort_children(root);
        let names: Vec<&str> = t.child_entries(root).map(|e| e.name.as_str()).collect();
        // Directories before files; alpha within each group.
        assert_eq!(names, vec!["adir", "cdir", "bfile", "zfile"]);
    }

    #[test]
    fn sort_dotfiles_last() {
        let mut t = fresh_tree();
        let root = t.root_id;
        t.insert_child(root, loaded(".hidden", EntryKind::File));
        t.insert_child(root, loaded("a", EntryKind::File));
        t.insert_child(root, loaded(".env", EntryKind::File));
        t.insert_child(root, loaded("b", EntryKind::File));
        t.sort_children(root);
        let names: Vec<&str> = t.child_entries(root).map(|e| e.name.as_str()).collect();
        // Visible first (alpha), then dotfiles (alpha among themselves).
        assert_eq!(names, vec!["a", "b", ".env", ".hidden"]);
    }

    #[test]
    fn sort_case_insensitive() {
        let mut t = fresh_tree();
        let root = t.root_id;
        t.insert_child(root, loaded("Zoo", EntryKind::File));
        t.insert_child(root, loaded("apple", EntryKind::File));
        t.insert_child(root, loaded("Banana", EntryKind::File));
        t.sort_children(root);
        let names: Vec<&str> = t.child_entries(root).map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["apple", "Banana", "Zoo"]);
    }

    #[test]
    fn sort_stable_for_same_lowercase() {
        let mut t = fresh_tree();
        let root = t.root_id;
        // Two entries that lower-case to the same string. Sort tie-break
        // uses original-case, so capital letters sort before lower-case
        // (per Rust string `Ord`: 'F' = 0x46 < 'f' = 0x66).
        t.insert_child(root, loaded("foo", EntryKind::File));
        t.insert_child(root, loaded("Foo", EntryKind::File));
        t.sort_children(root);
        let names: Vec<&str> = t.child_entries(root).map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["Foo", "foo"]);
    }

    #[test]
    fn apply_dir_load_diffs_added_and_removed() {
        let mut t = fresh_tree();
        let root = t.root_id;
        // First load: a, b.
        let (added1, removed1) = t.apply_dir_load(
            root,
            vec![loaded("a", EntryKind::File), loaded("b", EntryKind::File)],
        );
        assert_eq!(added1.len(), 2);
        assert!(removed1.is_empty());
        // Root flipped to Dir.
        assert_eq!(t.entry(root).unwrap().kind, EntryKind::Dir);

        // Second load: a deleted, c added, b unchanged.
        let (added2, removed2) = t.apply_dir_load(
            root,
            vec![loaded("b", EntryKind::File), loaded("c", EntryKind::File)],
        );
        assert_eq!(added2.len(), 1);
        assert_eq!(removed2, vec![PathBuf::from("a")]);
        let names: Vec<&str> = t.child_entries(root).map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["b", "c"]);
    }
}
