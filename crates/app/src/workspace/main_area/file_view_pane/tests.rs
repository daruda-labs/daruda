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
        pending_scroll_line: None,
    };
    // No selection → all rows copied.
    let text = fv.selected_text_for_copy();
    assert!(text.contains("-old"));
    assert!(text.contains("+new"));
}

/// Width of the first `<rect>` in a rendered single-node flowchart — the
/// node box the renderer sized from its text estimate.
fn first_node_rect_width(source: &str) -> f64 {
    let svg = merman::render::HeadlessRenderer::new()
        .render_svg_resvg_safe_sync(source)
        .expect("merman should render")
        .expect("diagram should be detected");
    let rect = svg
        .find("<rect")
        .expect("flowchart should emit a node rect");
    let attr = svg[rect..]
        .find("width=\"")
        .expect("rect should have width")
        + rect
        + 7;
    let end = svg[attr..].find('"').expect("unterminated width") + attr;
    svg[attr..end].parse().expect("numeric width")
}

/// East Asian Wide glyphs advance closer to a full em than Latin glyphs, so a
/// Hangul label must measure meaningfully wider than a same-length Latin one
/// — a renderer that regresses to a flat per-character ratio silently clips
/// CJK labels (the previous vendored renderer needed a local patch for
/// exactly this).
#[test]
fn mermaid_renderer_sizes_east_asian_labels_wider_than_latin() {
    let hangul = first_node_rect_width("flowchart TD\n  A[가나다라마바]\n");
    let latin = first_node_rect_width("flowchart TD\n  A[abcdef]\n");
    assert!(
        hangul > latin,
        "6 Hangul glyphs must measure wider than 6 Latin ones \
         (hangul={hangul}, latin={latin}) — East Asian width handling is broken"
    );
}

/// End-to-end guard through the real pipeline (merman host theme → SVG →
/// resvg raster): the diagram canvas must rasterize transparent, so the
/// bitmap composites over the host surface (agent-chat card tint) instead
/// of stamping an opaque rectangle. Node fills stay opaque separately.
/// Swept per diagram type because the original leak was per-type hardcoded
/// root backgrounds that bypass `themeVariables`.
#[test]
fn mermaid_raster_canvas_is_transparent_across_diagram_types() {
    let palette = mermaid_theme::MermaidPalette::default();
    let profile = mermaid_host_theme::mermaid_host_theme_profile(&palette);
    for source in [
        "flowchart TD\n  A[hello]\n",
        "sequenceDiagram\n  A->>B: hi\n",
        "pie\n  \"a\": 1\n  \"b\": 2\n",
        "stateDiagram-v2\n  [*] --> S1\n",
    ] {
        let svg = merman::render::HeadlessRenderer::new()
            .with_host_theme(&profile)
            .render_svg_sync(source)
            .expect("merman should render")
            .expect("diagram should be detected");
        let img = visual::rasterize_svg(&svg).expect("rasterize should succeed");
        // Corner pixel sits on the canvas, outside any node.
        assert_eq!(
            img.rgba[3], 0,
            "canvas corner must be fully transparent for {source:?}"
        );
        // The raster still contains opaque content (node fill / text).
        assert!(
            img.rgba.chunks_exact(4).any(|px| px[3] == 255),
            "diagram content must remain opaque for {source:?}"
        );
    }
}

fn right_edge_is_transparent(img: &visual::RasterImage) -> bool {
    let width = img.width as usize;
    (0..img.height as usize).all(|y| img.rgba[(y * width + width - 1) * 4 + 3] == 0)
}

#[test]
fn wide_mermaid_samples_keep_clear_right_edge_after_rasterize() {
    let palette = mermaid_theme::MermaidPalette::default();
    let profile = mermaid_host_theme::mermaid_host_theme_profile(&palette);
    for (name, source) in [
        (
            "er",
            r#"erDiagram
  WORKSPACE ||--o{ PROJECT : owns
  PROJECT ||--o{ AGENT : contains
  PROJECT ||--o{ RULE : defines
  AGENT ||--o{ HEARTBEAT : emits
  RULE ||--o{ ALERT : triggers
  ALERT ||--o{ NOTIFICATION : sends

  WORKSPACE {
    bigint id PK
    string name
    datetime created_at
  }

  PROJECT {
    bigint id PK
    bigint workspace_id FK
    string code
  }

  AGENT {
    bigint id PK
    bigint project_id FK
    string hostname
    string status
  }

  RULE {
    bigint id PK
    bigint project_id FK
    string expression
  }
"#,
        ),
        (
            "mindmap",
            r#"mindmap
  root((OpsMeta))
    Runtime State
      Redis
        heartbeat
        agent settings
        volatile metadata
    Ledger
      MariaDB
        rules
        history
        reports
        jobs
    Time Series
      S3 or MinIO
        parquet
        hot retention
        warm retention
    Operations
      backup
      purge
      restore
"#,
        ),
    ] {
        let svg = merman::render::HeadlessRenderer::new()
            .with_svg_options(mermaid_host_theme::mermaid_svg_render_options())
            .with_host_theme(&profile)
            .render_svg_sync(source)
            .expect("merman should render")
            .unwrap_or_else(|| panic!("{name} diagram should be detected"));
        let img = visual::rasterize_svg(&svg).expect("rasterize should succeed");
        assert!(
            right_edge_is_transparent(&img),
            "{name} diagram should leave transparent padding at the right edge"
        );
    }
}

#[test]
fn make_plain_row_helper() {
    let r = make_plain_row("foo", 1);
    assert_eq!(r.content, "foo");
    assert_eq!(r.line_no_left, "1");
    assert!(r.spans.is_empty());
    assert!(r.word_changes.is_empty());
}
