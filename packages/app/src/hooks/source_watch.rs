//! Watch source trees without recursively subscribing to their parent homes.

use std::{collections::BTreeSet, path::PathBuf, sync::mpsc::Receiver};

use super::watch_target::WatchTarget;

pub(crate) struct SourceWatch {
    subscriptions: Vec<(Receiver<()>, crate::dir_watch::DirWatcher)>,
}

impl SourceWatch {
    pub(crate) fn new(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut recursive = BTreeSet::new();
        let mut parents = BTreeSet::new();
        let mut targets = Vec::new();
        let mut pending: Vec<_> = paths.into_iter().map(|path| (path, 0)).collect();
        let mut seen = BTreeSet::new();
        while let Some((path, depth)) = pending.pop() {
            if !seen.insert(path.clone()) {
                continue;
            }
            let anchor = path.ancestors().find(|p| p.exists());
            targets.push(WatchTarget::new(&path, anchor));
            if path.is_dir() {
                recursive.insert(daruda_core::path::canonicalize_or_self(&path));
            }
            // Keep lexical parents for replacement and physical parents for
            // edits through a file symlink's external destination.
            for spelling in [path.clone(), daruda_core::path::canonicalize_or_self(&path)] {
                if let Some(parent) = spelling
                    .parent()
                    .and_then(|p| p.ancestors().find(|a| a.is_dir()))
                {
                    parents.insert(daruda_core::path::canonicalize_or_self(parent));
                }
            }
            for ancestor in path.ancestors() {
                if std::fs::symlink_metadata(ancestor).is_ok_and(|m| m.is_symlink())
                    && let Some(parent) = ancestor.parent()
                {
                    parents.insert(daruda_core::path::canonicalize_or_self(parent));
                    // A dangling link still has a destination whose nearest
                    // existing parent must detect the first file creation.
                    if depth < 40
                        && let Ok(destination) = std::fs::read_link(ancestor)
                    {
                        let destination = if destination.is_absolute() {
                            destination
                        } else {
                            parent.join(destination)
                        };
                        if let Ok(tail) = path.strip_prefix(ancestor) {
                            pending.push((destination.join(tail), depth + 1));
                        }
                    }
                }
            }
        }
        let subscriptions = [
            (recursive, notify::RecursiveMode::Recursive),
            (parents, notify::RecursiveMode::NonRecursive),
        ]
        .into_iter()
        .filter(|(anchors, _)| !anchors.is_empty())
        .map(|(anchors, mode)| {
            let targets = targets.clone();
            crate::dir_watch::spawn_dir_watcher(
                &anchors.into_iter().collect::<Vec<_>>(),
                mode,
                move |event| {
                    if invalidates(event, &targets) {
                        vec![()]
                    } else {
                        Vec::new()
                    }
                },
                || vec![()],
            )
        })
        .collect();
        Self { subscriptions }
    }

    pub(crate) fn changed(&self) -> bool {
        self.subscriptions
            .iter()
            .fold(false, |changed, (events, _)| {
                events.try_iter().count() > 0 || changed
            })
    }
}

fn invalidates(event: &notify::Event, targets: &[WatchTarget]) -> bool {
    !matches!(event.kind, notify::EventKind::Access(_))
        && event
            .paths
            .iter()
            .any(|p| targets.iter().any(|t| t.affected_by(p)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_source_subscriptions_disconnects_their_receivers() {
        let _serial = crate::hooks::tests_common::fsevent_serial();
        let temp = tempfile::tempdir().unwrap();
        let SourceWatch { subscriptions } = SourceWatch::new([temp.path().to_path_buf()]);
        assert_eq!(subscriptions.len(), 2);
        for (events, handle) in subscriptions {
            drop(handle);
            while events.try_recv().is_ok() {}
            assert!(matches!(
                events.recv_timeout(std::time::Duration::from_secs(1)),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
            ));
        }
    }

    #[test]
    fn reading_a_source_does_not_invalidate_it() {
        let path = PathBuf::from("source.json");
        let targets = [WatchTarget::new(&path, None)];
        let event = notify::Event::new(notify::EventKind::Access(notify::event::AccessKind::Any))
            .add_path(path.clone());
        assert!(!invalidates(&event, &targets));
        let event = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(path);
        assert!(invalidates(&event, &targets));
    }

    #[cfg(unix)]
    #[test]
    fn file_symlink_targets_and_replacements_are_observed() {
        let _serial = crate::hooks::tests_common::fsevent_serial();
        let home = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let target = external.path().join("actual.json");
        let link = home.path().join("config.json");
        std::fs::write(&target, "{}").unwrap();
        daruda_core::path::symlink(&target, &link).unwrap();
        let watch = SourceWatch::new([link.clone()]);
        std::thread::sleep(std::time::Duration::from_millis(250));
        std::fs::write(&target, "changed").unwrap();
        wait_for_change(&watch, "external target edit");
        std::fs::remove_file(&target).unwrap();
        wait_for_change(&watch, "external target deletion");
        drop(watch);
        let watch = SourceWatch::new([link.clone()]);
        std::thread::sleep(std::time::Duration::from_millis(250));
        watch.changed();
        std::fs::write(&target, "recreated").unwrap();
        wait_for_change(&watch, "dangling link target recreation");
        std::thread::sleep(std::time::Duration::from_millis(250));
        std::fs::remove_file(&link).unwrap();
        std::fs::write(link, "replacement").unwrap();
        wait_for_change(&watch, "link replacement");
    }

    #[cfg(unix)]
    fn wait_for_change(watch: &SourceWatch, phase: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !watch.changed() {
            assert!(
                std::time::Instant::now() < deadline,
                "source change was not observed: {phase}"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
