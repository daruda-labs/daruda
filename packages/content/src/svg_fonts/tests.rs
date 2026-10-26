use super::*;

/// Size [`ink_width`] lays its probe out at.
const PROBE_FONT_SIZE: f32 = 16.0;

/// Ink width of `text` laid out in `family`, through the app's own options.
fn ink_width(text: &str, family: &str) -> f32 {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="4000" height="200"><style>text{{font-family:{family};font-size:{PROBE_FONT_SIZE}px}}</style><text x="0" y="100">{text}</text></svg>"#
    );
    let tree = resvg::usvg::Tree::from_str(&svg, &usvg_options()).expect("parses");
    fn find(group: &resvg::usvg::Group) -> Option<f32> {
        for node in group.children() {
            match node {
                resvg::usvg::Node::Text(text) => return Some(text.abs_bounding_box().width()),
                resvg::usvg::Node::Group(inner) => {
                    if let Some(width) = find(inner) {
                        return Some(width);
                    }
                }
                _ => {}
            }
        }
        None
    }
    find(tree.root()).expect("laid out text")
}

/// The case the fix exists for: mermaid's own stack, verbatim. Without
/// case-insensitive resolution every name misses and text lands on the
/// generic `sans-serif` face, disagreeing with merman's Trebuchet MS
/// metrics by up to 14px per label.
#[test]
fn the_mermaid_font_stack_resolves_to_trebuchet_ms() {
    if !canonical_family_names().contains_key("trebuchet ms") {
        // Trebuchet MS ships with macOS (the verified target) but not with
        // every Linux image; nothing to assert without it.
        return;
    }
    const PROBE: &str = "schedule tick";
    assert!(
        (ink_width(PROBE, r#""trebuchet ms",verdana,arial,sans-serif"#)
            - ink_width(PROBE, r#""Trebuchet MS""#))
        .abs()
            < 0.05,
        "mermaid's lowercased stack must land on Trebuchet MS"
    );
}

/// One character the primary font lacks must not drag its neighbours into
/// the fallback face. Before `select_fallback` refused `.LastResort` —
/// which matches all of Unicode with placeholder boxes — usvg kept that
/// whole-run reshaping and a single emoji turned its entire mermaid label
/// into `□?`.
#[test]
fn one_unsupported_char_does_not_reshape_the_whole_run() {
    if !canonical_family_names().contains_key("trebuchet ms") {
        // Same guard as the test above: no Trebuchet MS, nothing to assert.
        return;
    }
    const STACK: &str = r#""trebuchet ms",verdana,arial,sans-serif"#;
    let latin = ink_width("run.yaml", STACK);
    let with_emoji = ink_width("run.yaml ✅", STACK);
    assert!(
        with_emoji < latin + 2.0 * PROBE_FONT_SIZE,
        "a trailing emoji may add about one em, not reshape the line \
         (latin={latin}, with_emoji={with_emoji})"
    );
}

/// The defect this guards: `fontdb` defaults every generic family to a
/// Microsoft core font, so on a host without them `sans-serif` resolved to
/// nothing and usvg dropped the text. Fails on such a host before the fix
/// and passes on every host after it — it cannot fail on macOS, where Arial
/// is installed either way.
#[test]
fn every_generic_family_names_a_face_that_is_installed() {
    let db = shared_fontdb();

    for (generic, _) in GENERIC_FAMILY_CANDIDATES {
        let name = current_generic_family(&db, generic);
        assert!(
            is_installed(&db, &name),
            "{generic:?} resolves to {name:?}, which this host does not have"
        );
    }
}

/// The repair path itself, driven on any host: point the generics at a name
/// nothing declares — which is the state a Linux box is already in, since
/// it has no Arial — and the mermaid stack must resolve again afterwards.
#[test]
fn a_generic_naming_nothing_is_repaired_and_text_lays_out_again() {
    use resvg::usvg::fontdb;

    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    for set in [
        fontdb::Database::set_sans_serif_family as fn(&mut _, String),
        fontdb::Database::set_serif_family,
        fontdb::Database::set_monospace_family,
    ] {
        set(&mut db, "No Such Face At All".to_owned());
    }
    let broken = db
        .query(&fontdb::Query {
            families: &[fontdb::Family::SansSerif, fontdb::Family::Serif],
            ..Default::default()
        })
        .is_none();
    assert!(broken, "the fixture must reproduce the miss first");

    adopt_installed_generic_families(&mut db);

    assert!(
        db.query(&fontdb::Query {
            families: &[fontdb::Family::SansSerif],
            ..Default::default()
        })
        .is_some(),
        "`sans-serif` — where every mermaid label's stack ends — still misses"
    );
}

/// What picks the replacement, asserted without needing a host that is
/// missing anything: an absent candidate is skipped for a present one.
#[test]
fn the_generic_picker_skips_candidates_the_host_lacks() {
    let db = shared_fontdb();
    let installed = db
        .faces()
        .find_map(|face| face.families.first().map(|(name, _)| name.clone()))
        .expect("the host has at least one font");

    assert_eq!(
        first_installed(&db, &["No Such Face At All", &installed]),
        Some(installed.as_str()),
        "the absent candidate should have been skipped"
    );
    assert_eq!(first_installed(&db, &["No Such Face At All"]), None);
}

/// Case-insensitively, because the lookup the fix feeds is.
#[test]
fn an_installed_face_is_matched_whatever_its_case() {
    let db = shared_fontdb();
    let installed = db
        .faces()
        .find_map(|face| face.families.first().map(|(name, _)| name.clone()))
        .expect("the host has at least one font");

    assert!(is_installed(&db, &installed.to_uppercase()));
    assert!(is_installed(&db, &installed.to_lowercase()));
}

#[test]
fn shared_fontdb_is_loaded_once() {
    let a = shared_fontdb();
    let b = shared_fontdb();
    assert!(
        Arc::ptr_eq(&a, &b),
        "repeated calls must return the same cached database"
    );
}
