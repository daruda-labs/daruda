use super::*;

#[test]
fn selected_text_for_copy_no_selection() {
    let hunks = parse_diff_hunks("@@ -1,2 +1,2 @@\n-old\n+new\n");
    let rows_all = build_diff_rows(&hunks, false);
    let rows_no_ctx = build_diff_rows(&hunks, true);
    let (added, removed) = count_diff_stats(&hunks);
    let fv = PaneFileView {
        origin: FileOrigin::Lane,
        path: "test.rs".into(),
        source: super::DiffSource::WorkingTree,
        live_status: None,
        content: PaneFileContent::LoadedDiff {
            rows_all,
            rows_no_ctx,
            added,
            removed,
        },
        view_mode: FileViewMode::Changes,
        hide_unchanged: false,
        selection_drag: SelectionDrag::None,
        search: None,
        pending_scroll_line: None,
    };
    // No selection → all rows copied.
    let text = fv.selected_text_for_copy();
    assert!(text.contains("-old"));
    assert!(text.contains("+new"));
}

/// The toolbar offers Preview by this and the loader parses by it, so one
/// answer covers both — `README.MD` used to get the button but plain text.
#[test]
fn a_markdown_extension_is_recognised_in_any_case() {
    for name in ["README.md", "README.MD", "notes.Markdown"] {
        assert!(
            super::is_markdown_path(std::path::Path::new(name)),
            "{name}"
        );
    }
    assert!(!super::is_markdown_path(std::path::Path::new("README.txt")));
}

/// A range pane carries its letter with its commits, so a pane restored
/// before any git read — `live_status` still `None` — still offers its diff.
#[test]
fn a_range_pane_reads_its_pinned_status_and_a_live_pane_its_live_one() {
    let range = super::DiffSource::Range {
        from: "m".into(),
        to: "h".into(),
        old_path: None,
        status: 'A',
    };
    let mut fv = PaneFileView::loading(
        FileOrigin::Lane,
        "a.rs".into(),
        range,
        None,
        FileViewMode::Changes,
    );
    assert_eq!(fv.status(), Some('A'));
    fv.source = super::DiffSource::WorkingTree;
    assert_eq!(fv.status(), None);
    fv.live_status = Some('M');
    assert_eq!(fv.status(), Some('M'));
}

/// The toolbar colours a pane's status by this answer and the against-base
/// row colours the same file as committed; a range pane must agree with it.
#[test]
fn staged_and_range_panes_read_as_committed_and_the_working_tree_does_not() {
    use super::DiffSource;
    assert!(!DiffSource::WorkingTree.reads_as_committed());
    assert!(DiffSource::Index.reads_as_committed());
    assert!(
        DiffSource::Range {
            from: "m".into(),
            to: "h".into(),
            old_path: None,
            status: 'M',
        }
        .reads_as_committed()
    );
}
