use super::*;

fn make_plain_row(content: &str, line_no: usize) -> VisualRow {
    VisualRow {
        kind: VisualRowKind::Plain,
        line_no_left: line_no.to_string(),
        line_no_right: String::new(),
        content: content.to_owned(),
        header_context: String::new(),
        spans: Vec::new(),
        word_changes: Vec::new(),
    }
}

#[test]
fn parse_diff_hunks_basic() {
    let diff = "\
diff --git a/foo.rs b/foo.rs
index abc..def 100644
--- a/foo.rs
+++ b/foo.rs
@@ -1,3 +1,4 @@
 fn main() {
-    println!(\"old\");
+    println!(\"new\");
+    println!(\"extra\");
 }
";
    let hunks = parse_diff_hunks(diff);
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].old_start, 1);
    assert_eq!(hunks[0].header, "@@ -1,3 +1,4 @@");
    assert_eq!(hunks[0].header_context, "");
    let added = hunks[0]
        .lines
        .iter()
        .filter(|l| matches!(l, DiffLine::Added { .. }))
        .count();
    let removed = hunks[0]
        .lines
        .iter()
        .filter(|l| matches!(l, DiffLine::Removed { .. }))
        .count();
    assert_eq!(added, 2);
    assert_eq!(removed, 1);
}

#[test]
fn parse_diff_hunks_header_context() {
    let diff = "@@ -5,3 +5,3 @@ fn bar() {\n-old\n+new\n";
    let hunks = parse_diff_hunks(diff);
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].header, "@@ -5,3 +5,3 @@");
    assert_eq!(hunks[0].header_context, "fn bar() {");
}

#[test]
fn parse_diff_hunks_multiple() {
    let diff = "\
diff --git a/bar.rs b/bar.rs
--- a/bar.rs
+++ b/bar.rs
@@ -1,3 +1,3 @@
 a
-b
+B
 c
@@ -10,3 +10,3 @@
 x
-y
+Y
 z
";
    let hunks = parse_diff_hunks(diff);
    assert_eq!(hunks.len(), 2);
    assert_eq!(hunks[0].old_start, 1);
    assert_eq!(hunks[1].old_start, 10);
}

#[test]
fn parse_diff_empty_diff() {
    let hunks = parse_diff_hunks("");
    assert!(hunks.is_empty());
}

#[test]
fn build_raw_rows_line_numbers() {
    let lines: Vec<String> = vec!["alpha".into(), "beta".into(), "gamma".into()];
    let rows = build_raw_rows(&lines);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].line_no_left, "1");
    assert_eq!(rows[2].line_no_left, "3");
    assert!(matches!(rows[0].kind, VisualRowKind::Plain));
    assert!(rows[0].header_context.is_empty());
    assert!(rows[0].spans.is_empty());
}

#[test]
fn build_diff_rows_hide_ctx() {
    let hunks = parse_diff_hunks("@@ -1,3 +1,3 @@\n context\n-old\n+new\n");
    let all = build_diff_rows(&hunks, false);
    let no_ctx = build_diff_rows(&hunks, true);
    // all: header + context + removed + added = 4
    assert_eq!(all.len(), 4);
    // no_ctx: header + removed + added = 3 (context skipped)
    assert_eq!(no_ctx.len(), 3);
    assert!(
        no_ctx
            .iter()
            .all(|r| !matches!(r.kind, VisualRowKind::Context))
    );
}

#[test]
fn build_diff_rows_header_context_propagated() {
    let hunks = parse_diff_hunks("@@ -1,2 +1,2 @@ fn foo() {\n-old\n+new\n");
    let rows = build_diff_rows(&hunks, false);
    assert!(matches!(rows[0].kind, VisualRowKind::HunkHeader));
    assert_eq!(rows[0].content, "@@ -1,2 +1,2 @@");
    assert_eq!(rows[0].header_context, "fn foo() {");
}

#[test]
fn count_diff_stats_basic() {
    let hunks = parse_diff_hunks("@@ -1,3 +1,4 @@\n ctx\n-old\n+new1\n+new2\n");
    let (added, removed) = count_diff_stats(&hunks);
    assert_eq!(added, 2);
    assert_eq!(removed, 1);
}

#[test]
fn copy_text_markers() {
    let added = VisualRow {
        kind: VisualRowKind::Added,
        line_no_left: String::new(),
        line_no_right: "1".into(),
        content: "hello".into(),
        header_context: String::new(),
        spans: Vec::new(),
        word_changes: Vec::new(),
    };
    assert_eq!(added.copy_text(), "+hello");

    let removed = VisualRow {
        kind: VisualRowKind::Removed,
        line_no_left: "1".into(),
        line_no_right: String::new(),
        content: "world".into(),
        header_context: String::new(),
        spans: Vec::new(),
        word_changes: Vec::new(),
    };
    assert_eq!(removed.copy_text(), "-world");

    let ctx = VisualRow {
        kind: VisualRowKind::Context,
        line_no_left: "1".into(),
        line_no_right: "1".into(),
        content: "ctx".into(),
        header_context: String::new(),
        spans: Vec::new(),
        word_changes: Vec::new(),
    };
    assert_eq!(ctx.copy_text(), " ctx");
}

#[test]
fn selected_text_for_copy_no_selection() {
    let hunks = parse_diff_hunks("@@ -1,2 +1,2 @@\n-old\n+new\n");
    let rows_all = build_diff_rows(&hunks, false);
    let rows_no_ctx = build_diff_rows(&hunks, true);
    let (added, removed) = count_diff_stats(&hunks);
    let fv = PaneFileView {
        lane_id: 0,
        path: "test.rs".into(),
        staged: false,
        file_status: None,
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
    };
    // No selection → all rows copied.
    let text = fv.selected_text_for_copy();
    assert!(text.contains("-old"));
    assert!(text.contains("+new"));
}

#[test]
fn make_plain_row_helper() {
    let r = make_plain_row("foo", 1);
    assert_eq!(r.content, "foo");
    assert_eq!(r.line_no_left, "1");
    assert!(r.spans.is_empty());
    assert!(r.word_changes.is_empty());
}
