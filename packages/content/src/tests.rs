//! The diagram pipeline end to end: merman layout → host theme → SVG → resvg
//! raster.

/// Area reserved for the first node, including wrapped label lines.
fn first_node_rect_area(source: &str) -> f64 {
    let request = merman::SvgRequest {
        pipeline: Some(merman::svg::SvgPipeline::resvg_safe()),
        ..Default::default()
    };
    let engine = merman::Engine::new().with_site_config(merman::MermaidConfig::from_value(
        serde_json::json!({"htmlLabels": false}),
    ));
    let output = merman::Renderer::new()
        .with_engine(engine)
        .render(merman::RenderRequest::svg(
            source,
            merman::OperationControl::default(),
            request,
        ))
        .expect("merman should render");
    let merman::RenderOutput::Svg(Some(output)) = output else {
        panic!("diagram should be detected")
    };
    let svg = output.svg();
    let rect = svg
        .find("<rect")
        .expect("flowchart should emit a node rect");
    let rect = &svg[rect..svg[rect..].find('>').unwrap() + rect];
    let dimension = |name: &str| -> f64 {
        let start = rect.find(&format!("{name}=\"")).expect("node dimension") + name.len() + 2;
        let end = rect[start..].find('"').expect("dimension end") + start;
        rect[start..end].parse().expect("numeric dimension")
    };
    dimension("width") * dimension("height")
}

/// Wide glyphs require more space even when the renderer caps node width
/// and wraps the longer label onto extra lines.
#[test]
fn mermaid_renderer_reserves_more_space_for_east_asian_labels() {
    let hangul = first_node_rect_area(
        "flowchart TD\n  A[가나다라마바가나다라마바가나다라마바가나다라마바]\n",
    );
    let latin = first_node_rect_area("flowchart TD\n  A[abcdefabcdefabcdefabcdef]\n");
    assert!(
        hangul > latin,
        "wide glyphs must reserve more space (hangul={hangul}, latin={latin})"
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
    let palette = crate::test_palette();
    for source in [
        "flowchart TD\n  A[hello]\n",
        "sequenceDiagram\n  A->>B: hi\n",
        "pie\n  \"a\": 1\n  \"b\": 2\n",
        "stateDiagram-v2\n  [*] --> S1\n",
    ] {
        let svg =
            crate::visual::render_mermaid_svg(source, &palette).expect("diagram should render");
        let img = crate::visual::rasterize_svg(&svg).expect("rasterize should succeed");
        // Corner pixel sits on the canvas, outside any node.
        assert_eq!(
            img.bgra[3], 0,
            "canvas corner must be fully transparent for {source:?}"
        );
        // The raster still contains opaque content (node fill / text).
        assert!(
            img.bgra.as_chunks::<4>().0.iter().any(|px| px[3] == 255),
            "diagram content must remain opaque for {source:?}"
        );
    }
}

fn right_edge_is_transparent(img: &crate::visual::RasterImage) -> bool {
    let width = img.width as usize;
    (0..img.height as usize).all(|y| img.bgra[(y * width + width - 1) * 4 + 3] == 0)
}

#[test]
fn wide_mermaid_samples_keep_clear_right_edge_after_rasterize() {
    let palette = crate::test_palette();
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
        let svg = crate::visual::render_mermaid_svg(source, &palette)
            .unwrap_or_else(|| panic!("{name} diagram should render"));
        let img = crate::visual::rasterize_svg(&svg).expect("rasterize should succeed");
        assert!(
            right_edge_is_transparent(&img),
            "{name} diagram should leave transparent padding at the right edge"
        );
    }
}
