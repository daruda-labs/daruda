//! Contrast-safety preprocessor for sequence `rect`/`box` mermaid source
//! text.
//!
//! A sequence `rect`/`box` background is a bare color argument with no
//! `classDef`-shaped escape hatch to pair a text color to at all, so it is
//! fixed by capping its fill to a translucent tint instead: a tint can't
//! out-contrast the text sitting on it, because it never stops being mostly
//! the surface behind it — the same reasoning behind daruda's own
//! diff-add-bg / diff-del-bg rows. `rect`/`box` genuinely is meant as a
//! subtle background wash grouping related messages (not a bold status
//! indicator the way a flowchart/class/ER node is), so a translucent tint is
//! also the *intended* look here, not just a workaround.
//!
//! `classDef`/`style` fills (flowchart, class diagram, ER diagram, …) are a
//! different problem with a different fix: those are meant to read as bold,
//! clearly-colored status indicators, so washing them out to a tint would
//! defeat the point even though it would technically keep the text legible.
//! See `mermaid_node_contrast.rs` — it forces label-text contrast by
//! rewriting the *rendered* SVG instead, so the fill stays exactly as the
//! diagram author declared it.
//!
//! Pure text rewriting — no mermaid grammar parser, just the same
//! lightweight line-level scan `mermaid_sources`
//! (`agent_chat_pane/agent_chat_helpers.rs`) already uses for fence
//! extraction. Runs once, before the source reaches merman, inside the single
//! `render_mermaid_svg` funnel — so every mermaid surface (agent chat *and*
//! file viewer) picks it up for free.
//!
//! Only ever *lowers* an already-opaque fill's alpha — never raises one the
//! author already set low, and a named CSS color (`rect Aqua`) is left
//! untouched rather than guessed at. So it's safe to run unconditionally,
//! including over a diagram that carries its own `%%{init}%%` directive.

use std::borrow::Cow;

/// Alpha every capped fill lands at — matches
/// [`crate::workspace::main_area::file_view_pane::mermaid_theme`]'s
/// `DIAGRAM_SURFACE_ALT_ALPHA` sibling tier (daruda's own diff-row tint
/// strength), so a highlighted box reads the same "subtle tint, not a
/// competing fill" as the rest of the app's translucent surfaces.
pub(in crate::workspace) const MAX_ALPHA: f64 = 0.12;

/// Rewrite `source` so every sequence `rect`/`box` background is capped to
/// [`MAX_ALPHA`]. Zero-copy (borrows `source` unchanged) when no line needs
/// rewriting.
pub(in crate::workspace) fn ensure_text_contrast(source: &str) -> Cow<'_, str> {
    let mut changed = false;
    let mut out = String::with_capacity(source.len());
    for (i, line) in source.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        match rewrite_line(line) {
            Some(rewritten) => {
                changed = true;
                out.push_str(&rewritten);
            }
            None => out.push_str(line),
        }
    }
    if changed {
        Cow::Owned(out)
    } else {
        Cow::Borrowed(source)
    }
}

/// `Some(rewritten)` when `line` is a `rect`/`box` declaration this pass can
/// improve; `None` leaves the caller to keep the line verbatim (not a match,
/// or its fill is already translucent enough).
fn rewrite_line(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    if let Some(rest) = trimmed.strip_prefix("rect ") {
        return rewrite_rect(indent, "rect", rest);
    }
    if let Some(rest) = trimmed.strip_prefix("box ") {
        return rewrite_rect(indent, "box", rest);
    }
    None
}

/// Cap a `rect`/`box` color expression's alpha to [`MAX_ALPHA`]. `keyword` is
/// echoed back verbatim so the caller doesn't need a second copy. `None` when
/// the expression isn't recognized or is already at/below the cap.
fn rewrite_rect(indent: &str, keyword: &str, rest: &str) -> Option<String> {
    // `box` carries a trailing participant-group name after the color
    // (`box rgb(0,255,0) Team1`); `rect` never has a trailing token. Only the
    // leading color token is ever a fill this pass touches. Split on the
    // *matching* boundary, not the first space — `rgb(212, 248, 212)`'s own
    // comma-space separators would otherwise cut the color token in half.
    let (color, trailing) = split_color_token(rest.trim_end());
    let capped = cap_alpha(color)?;
    let trailing = if trailing.is_empty() {
        String::new()
    } else {
        format!(" {trailing}")
    };
    Some(format!("{indent}{keyword} {capped}{trailing}"))
}

/// Split `rest` into its leading color token and whatever follows (`box`'s
/// optional participant-group name), honoring `rgb(...)`/`rgba(...)`'s own
/// internal `, `-separated arguments so they aren't mistaken for the
/// boundary. A `#hex` or bare named-color token ends at the first space, same
/// as before.
fn split_color_token(rest: &str) -> (&str, &str) {
    if (rest.starts_with("rgb(") || rest.starts_with("rgba("))
        && let Some(close) = rest.find(')')
    {
        let (color, after) = rest.split_at(close + 1);
        return (color, after.trim_start());
    }
    rest.split_once(' ').unwrap_or((rest, ""))
}

/// `Some("rgba(r, g, b, MAX_ALPHA)")` when `expr` parses to a color whose
/// alpha exceeds [`MAX_ALPHA`]; `None` when it's already at/below the cap, or
/// isn't a recognized shape (a named CSS color — out of scope, left as the
/// diagram author wrote it).
pub(in crate::workspace) fn cap_alpha(expr: &str) -> Option<String> {
    let (r, g, b, a) = parse_color_with_alpha(expr)?;
    (a > MAX_ALPHA).then(|| format!("rgba({r}, {g}, {b}, {MAX_ALPHA})"))
}

/// Parse `rgb(r,g,b)`, `rgba(r,g,b,a)`, `#RRGGBB`, `#RGB`, or `#RRGGBBAA` into
/// `(r, g, b, alpha)`. A form with no alpha channel is opaque (`alpha = 1.0`).
/// `None` for a named CSS color or malformed input.
pub(in crate::workspace) fn parse_color_with_alpha(expr: &str) -> Option<(u8, u8, u8, f64)> {
    let expr = expr.trim();
    if let Some(inner) = expr.strip_prefix("rgba(").and_then(|s| s.strip_suffix(')')) {
        let mut parts = inner.split(',').map(str::trim);
        let r = parts.next()?.parse().ok()?;
        let g = parts.next()?.parse().ok()?;
        let b = parts.next()?.parse().ok()?;
        let a = parts.next()?.parse().ok()?;
        return (parts.next().is_none()).then_some((r, g, b, a));
    }
    if let Some(inner) = expr.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let mut parts = inner.split(',').map(str::trim);
        let r = parts.next()?.parse().ok()?;
        let g = parts.next()?.parse().ok()?;
        let b = parts.next()?.parse().ok()?;
        return (parts.next().is_none()).then_some((r, g, b, 1.0));
    }
    parse_hex_color(expr)
}

/// Parse a `#RRGGBB`, `#RGB`, or `#RRGGBBAA` hex color into `(r, g, b, alpha)`
/// (`alpha = 1.0` for the two forms with no alpha byte). `None` for anything
/// else.
pub(in crate::workspace) fn parse_hex_color(value: &str) -> Option<(u8, u8, u8, f64)> {
    let hex = value.strip_prefix('#')?;
    let byte = |i: usize| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok();
    match hex.len() {
        6 => Some((byte(0)?, byte(1)?, byte(2)?, 1.0)),
        8 => Some((byte(0)?, byte(1)?, byte(2)?, f64::from(byte(3)?) / 255.0)),
        3 => {
            let mut chars = hex.chars();
            let expand = |c: char| -> Option<u8> {
                let v = c.to_digit(16)? as u8;
                Some(v * 16 + v)
            };
            let r = expand(chars.next()?)?;
            let g = expand(chars.next()?)?;
            let b = expand(chars.next()?)?;
            Some((r, g, b, 1.0))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_rect_rgb_is_capped_to_translucent() {
        let source = "sequenceDiagram\n  rect rgb(212, 248, 212)\n  A->>B: Hi\n  end\n";
        assert_eq!(
            ensure_text_contrast(source),
            "sequenceDiagram\n  rect rgba(212, 248, 212, 0.12)\n  A->>B: Hi\n  end\n"
        );
    }

    #[test]
    fn rect_hex_is_capped_to_translucent() {
        let source = "sequenceDiagram\n  rect #d4f8d4\n  A->>B: Hi\n  end\n";
        assert_eq!(
            ensure_text_contrast(source),
            "sequenceDiagram\n  rect rgba(212, 248, 212, 0.12)\n  A->>B: Hi\n  end\n"
        );
    }

    #[test]
    fn rect_already_translucent_is_untouched() {
        let source = "sequenceDiagram\n  rect rgba(212, 248, 212, 0.1)\n  A->>B: Hi\n  end\n";
        assert!(matches!(
            ensure_text_contrast(source),
            Cow::Borrowed(s) if s == source
        ));
    }

    #[test]
    fn rect_alpha_is_never_raised() {
        // An author who deliberately chose a very faint 2% tint keeps it —
        // this pass only ever lowers an over-opaque alpha, never raises one.
        let source = "sequenceDiagram\n  rect rgba(212, 248, 212, 0.02)\n  end\n";
        assert!(matches!(
            ensure_text_contrast(source),
            Cow::Borrowed(s) if s == source
        ));
    }

    #[test]
    fn rect_named_color_is_left_alone() {
        let source = "sequenceDiagram\n  rect Aqua\n  A->>B: Hi\n  end\n";
        assert!(matches!(
            ensure_text_contrast(source),
            Cow::Borrowed(s) if s == source
        ));
    }

    #[test]
    fn box_trailing_participant_group_name_is_preserved() {
        let source = "sequenceDiagram\n  box rgb(212, 248, 212) Team1\n  participant A\n  end\n";
        assert_eq!(
            ensure_text_contrast(source),
            "sequenceDiagram\n  box rgba(212, 248, 212, 0.12) Team1\n  participant A\n  end\n"
        );
    }

    #[test]
    fn box_named_color_with_trailing_name_is_left_alone() {
        let source = "sequenceDiagram\n  box Aqua Team1\n  end\n";
        assert!(matches!(
            ensure_text_contrast(source),
            Cow::Borrowed(s) if s == source
        ));
    }

    #[test]
    fn unrelated_lines_pass_through_untouched() {
        let source = "flowchart TD\n  A --> B\n  B --> C\n  classDef hl fill:#d4f8d4\n";
        assert!(matches!(
            ensure_text_contrast(source),
            Cow::Borrowed(s) if s == source
        ));
    }
}
