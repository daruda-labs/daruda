//! Font resolution shared by SVG geometry measurement and rasterization.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// Parse options for every SVG daruda reads, so a document is measured and
/// rasterized through exactly one text stack. Anything that inspects geometry
/// before rasterizing (see `mermaid_label_geometry`) must parse with these too,
/// or it measures a different font than the one that gets painted.
pub(crate) fn usvg_options() -> resvg::usvg::Options<'static> {
    let mut options = resvg::usvg::Options {
        fontdb: shared_fontdb(),
        ..Default::default()
    };
    options.font_resolver.select_font = case_insensitive_font_selector();
    options.font_resolver.select_fallback = glyph_fallback_selector();
    options
}

/// Keep a glyph fallback from hijacking the whole text run.
///
/// usvg reshapes the entire run with the fallback face and, when that face
/// matched *every* glyph, keeps that shaping wholesale (`usvg::text::layout`,
/// `shape_text`). macOS's `.LastResort` covers all of Unicode with placeholder
/// boxes and is the first face the default selector finds for an emoji, so one
/// `✅` in a mermaid label turned every character on that line into `□?`.
///
/// Refusing the dot-prefixed system faces — internal, unnameable from CSS, and
/// never what a document asked for — lets the real face for that character win
/// (Apple Color Emoji) and leaves the rest of the run in its own font. A
/// character no ordinary face covers now falls back to the primary font's
/// `.notdef` instead of taking its neighbours with it.
fn glyph_fallback_selector() -> resvg::usvg::FallbackSelectionFn<'static> {
    let default = resvg::usvg::FontResolver::default_fallback_selector();
    Box::new(move |c, exclude_fonts, db| {
        let mut exclude = exclude_fonts.to_vec();
        loop {
            let id = default(c, &exclude, db)?;
            let is_system_internal = db
                .face(id)
                .and_then(|face| face.families.first())
                .is_some_and(|(name, _)| name.starts_with('.'));
            if !is_system_internal {
                return Some(id);
            }
            // Excluded, so the next round cannot return it again.
            exclude.push(id);
        }
    })
}

/// Resolve `font-family` names the way CSS does: case-insensitively.
///
/// `fontdb` compares family names byte-for-byte, and mermaid emits its stack
/// lowercased (`font-family:"trebuchet ms",verdana,arial,sans-serif`), so every
/// named family missed and text fell through to the generic `sans-serif` face.
/// Diagrams rendered in Arial while merman had sized every label from its
/// Trebuchet MS metrics — up to 14px of disagreement per label, which shows up
/// as text overflowing node boxes and label bands.
fn case_insensitive_font_selector() -> resvg::usvg::FontSelectionFn<'static> {
    use resvg::usvg::fontdb;

    Box::new(|font, db| {
        // `fontdb::Family::Name` borrows, so the canonical spellings have to
        // outlive the query built from them.
        let canonical: Vec<String> = font
            .families()
            .iter()
            .filter_map(|family| match family {
                resvg::usvg::FontFamily::Named(name) => Some(
                    canonical_family_names()
                        .get(&name.to_lowercase())
                        .cloned()
                        .unwrap_or_else(|| name.clone()),
                ),
                _ => None,
            })
            .collect();
        let mut named = canonical.iter();
        let mut families: Vec<fontdb::Family> = font
            .families()
            .iter()
            .map(|family| match family {
                resvg::usvg::FontFamily::Serif => fontdb::Family::Serif,
                resvg::usvg::FontFamily::SansSerif => fontdb::Family::SansSerif,
                resvg::usvg::FontFamily::Cursive => fontdb::Family::Cursive,
                resvg::usvg::FontFamily::Fantasy => fontdb::Family::Fantasy,
                resvg::usvg::FontFamily::Monospace => fontdb::Family::Monospace,
                resvg::usvg::FontFamily::Named(_) => {
                    fontdb::Family::Name(named.next().map_or("", std::string::String::as_str))
                }
            })
            .collect();
        // Same last-resort family usvg's own selector appends.
        families.push(fontdb::Family::Serif);

        db.query(&fontdb::Query {
            families: &families,
            weight: fontdb::Weight(font.weight()),
            stretch: match font.stretch() {
                resvg::usvg::FontStretch::UltraCondensed => fontdb::Stretch::UltraCondensed,
                resvg::usvg::FontStretch::ExtraCondensed => fontdb::Stretch::ExtraCondensed,
                resvg::usvg::FontStretch::Condensed => fontdb::Stretch::Condensed,
                resvg::usvg::FontStretch::SemiCondensed => fontdb::Stretch::SemiCondensed,
                resvg::usvg::FontStretch::Normal => fontdb::Stretch::Normal,
                resvg::usvg::FontStretch::SemiExpanded => fontdb::Stretch::SemiExpanded,
                resvg::usvg::FontStretch::Expanded => fontdb::Stretch::Expanded,
                resvg::usvg::FontStretch::ExtraExpanded => fontdb::Stretch::ExtraExpanded,
                resvg::usvg::FontStretch::UltraExpanded => fontdb::Stretch::UltraExpanded,
            },
            style: match font.style() {
                resvg::usvg::FontStyle::Normal => fontdb::Style::Normal,
                resvg::usvg::FontStyle::Italic => fontdb::Style::Italic,
                resvg::usvg::FontStyle::Oblique => fontdb::Style::Oblique,
            },
        })
    })
}

/// Lowercased family name → the spelling the installed face declares. Built
/// once from the shared database; first declaration wins so a later face can't
/// shadow the canonical name.
fn canonical_family_names() -> &'static HashMap<String, String> {
    static NAMES: OnceLock<HashMap<String, String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut names = HashMap::new();
        for face in shared_fontdb().faces() {
            for (family, _) in &face.families {
                names
                    .entry(family.to_lowercase())
                    .or_insert_with(|| family.clone());
            }
        }
        names
    })
}

/// System font database, loaded once and shared across all `rasterize_svg`
/// calls. `load_system_fonts` scans and parses every installed face, so doing
/// it per render would dominate the cost of drawing a diagram.
fn shared_fontdb() -> Arc<resvg::usvg::fontdb::Database> {
    static FONTDB: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    FONTDB
        .get_or_init(|| {
            let mut db = resvg::usvg::fontdb::Database::new();
            db.load_system_fonts();
            adopt_installed_generic_families(&mut db);
            Arc::new(db)
        })
        .clone()
}

/// Candidate faces behind each CSS generic family, most-preferred first. The
/// Microsoft core names lead because a document asking for `sans-serif` on
/// macOS or Windows should keep getting the face it has always been painted
/// with; the rest are what a Linux install actually ships.
const GENERIC_FAMILY_CANDIDATES: [(GenericFamily, &[&str]); 5] = [
    (
        GenericFamily::SansSerif,
        &[
            "Arial",
            "Helvetica",
            "Liberation Sans",
            "DejaVu Sans",
            "Noto Sans",
            "FreeSans",
        ],
    ),
    (
        GenericFamily::Serif,
        &[
            "Times New Roman",
            "Times",
            "Liberation Serif",
            "DejaVu Serif",
            "Noto Serif",
            "FreeSerif",
        ],
    ),
    (
        GenericFamily::Monospace,
        &[
            "Courier New",
            "Menlo",
            "Liberation Mono",
            "DejaVu Sans Mono",
            "Noto Sans Mono",
            "FreeMono",
        ],
    ),
    (
        GenericFamily::Cursive,
        &[
            "Comic Sans MS",
            "Comic Neue",
            "URW Chancery L",
            "DejaVu Sans",
        ],
    ),
    (
        GenericFamily::Fantasy,
        &["Impact", "Papyrus", "Ultra", "DejaVu Sans"],
    ),
];

/// Which generic a candidate list stands for. `fontdb` exposes the five as
/// setters rather than a value, so this is what lets one loop drive them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GenericFamily {
    SansSerif,
    Serif,
    Monospace,
    Cursive,
    Fantasy,
}

/// Point every generic family at a face the database actually holds.
///
/// `fontdb` defaults them to Arial / Times New Roman / Courier New, which no
/// ordinary Linux install has — so every mermaid stack, all of which end in
/// `sans-serif`, missed and usvg dropped the `<text>`. One already naming an
/// installed face is left alone: the platform's own or a fontconfig alias.
fn adopt_installed_generic_families(db: &mut resvg::usvg::fontdb::Database) {
    for (generic, candidates) in GENERIC_FAMILY_CANDIDATES {
        let current = current_generic_family(db, generic);
        if is_installed(db, &current) {
            continue;
        }
        let Some(replacement) = first_installed(db, candidates) else {
            continue;
        };
        let replacement = replacement.to_owned();
        match generic {
            GenericFamily::SansSerif => db.set_sans_serif_family(replacement),
            GenericFamily::Serif => db.set_serif_family(replacement),
            GenericFamily::Monospace => db.set_monospace_family(replacement),
            GenericFamily::Cursive => db.set_cursive_family(replacement),
            GenericFamily::Fantasy => db.set_fantasy_family(replacement),
        }
    }
}

fn current_generic_family(db: &resvg::usvg::fontdb::Database, generic: GenericFamily) -> String {
    use resvg::usvg::fontdb::Family;
    let family = match generic {
        GenericFamily::SansSerif => Family::SansSerif,
        GenericFamily::Serif => Family::Serif,
        GenericFamily::Monospace => Family::Monospace,
        GenericFamily::Cursive => Family::Cursive,
        GenericFamily::Fantasy => Family::Fantasy,
    };
    db.family_name(&family).to_owned()
}

/// The first candidate naming a face in `db`, or `None` when it holds none of
/// them — a database with no fonts at all included.
fn first_installed<'a>(
    db: &resvg::usvg::fontdb::Database,
    candidates: &[&'a str],
) -> Option<&'a str> {
    candidates
        .iter()
        .copied()
        .find(|candidate| is_installed(db, candidate))
}

/// Case-insensitively, matching how CSS compares family names — see
/// [`case_insensitive_font_selector`].
fn is_installed(db: &resvg::usvg::fontdb::Database, family: &str) -> bool {
    db.faces().any(|face| {
        face.families
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(family))
    })
}

#[cfg(test)]
mod tests;
