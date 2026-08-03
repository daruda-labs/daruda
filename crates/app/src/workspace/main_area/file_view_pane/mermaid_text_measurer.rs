//! Text measurement merman uses for mermaid layout, corrected for Hangul.
//!
//! merman's vendored metrics reproduce Trebuchet MS almost exactly (within
//! 0.01px for Latin), but Trebuchet MS has no Hangul, so the table falls back to
//! an estimate — roughly 0.86em per syllable — while the rasterizer resolves a
//! real Korean face and advances a full em. Layout therefore reserves ~14% too
//! little width for Korean text: a state node's label runs past its box, and an
//! edge label's band comes out narrower than its glyphs.
//!
//! This measurer delegates every measurement to merman's own, then adds the
//! difference for the Hangul syllables in the string. The per-syllable numbers
//! are both *measured*, not assumed:
//!
//! - what merman thinks a syllable advances — asked of the delegate itself,
//! - what it really advances — probed once through the same usvg text stack that
//!   rasterizes, because Korean faces are not all full-width (Apple SD Gothic
//!   Neo advances ~0.82em where Arial Unicode MS advances 1.0em).
//!
//! So the correction is zero for text without Hangul, and collapses to zero on
//! its own if merman ever ships real Hangul metrics.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use merman::render::{
    TextMeasurer, TextMetrics, TextStyle, VendoredFontMetricsTextMeasurer, WrapMode,
};

/// Hangul Syllables. The Jamo blocks are left alone: merman already advances
/// those a full em, matching the rasterizer.
const HANGUL_SYLLABLES: std::ops::RangeInclusive<char> = '\u{AC00}'..='\u{D7A3}';

/// Font size for the advance probe. Advances scale linearly with size in usvg,
/// so one large probe gives a precise em ratio for every size.
const PROBE_FONT_SIZE: f64 = 100.0;

/// Representative syllable for the probe. Any syllable does: a face that covers
/// the block covers it uniformly.
const PROBE_SYLLABLE: &str = "가";

/// The measurer merman should lay diagrams out with.
pub(in crate::workspace) fn host_text_measurer() -> std::sync::Arc<dyn TextMeasurer + Send + Sync> {
    std::sync::Arc::new(HangulCorrectedMeasurer::default())
}

#[derive(Default)]
struct HangulCorrectedMeasurer {
    delegate: VendoredFontMetricsTextMeasurer,
}

impl HangulCorrectedMeasurer {
    /// Width merman under-counts for the Hangul in `text`, in px. Zero for text
    /// without Hangul syllables, so non-Korean diagrams are untouched.
    fn hangul_deficit_px(&self, text: &str, style: &TextStyle) -> f64 {
        let syllables = text
            .chars()
            .filter(|c| HANGUL_SYLLABLES.contains(c))
            .count();
        if syllables == 0 {
            return 0.0;
        }
        let Some(painted_em) = painted_syllable_advance_em(style) else {
            return 0.0;
        };
        let assumed = self.assumed_syllable_advance_px(style);
        syllables as f64 * (painted_em * style.font_size - assumed)
    }

    /// What the delegate itself advances per syllable: the width it adds for a
    /// second copy of the same glyph. Asking beats hardcoding its table.
    fn assumed_syllable_advance_px(&self, style: &TextStyle) -> f64 {
        let one = self.delegate.measure(PROBE_SYLLABLE, style).width;
        let two = self
            .delegate
            .measure(&PROBE_SYLLABLE.repeat(2), style)
            .width;
        two - one
    }

    /// Correct a single-line measurement: the whole string is on one line, so
    /// the deficit applies in full.
    fn corrected(&self, mut metrics: TextMetrics, text: &str, style: &TextStyle) -> TextMetrics {
        metrics.width += self.hangul_deficit_px(text, style);
        metrics
    }

    /// Correct a *wrapped* measurement, whose width is the longest line rather
    /// than the whole string. Adding the whole string's deficit would inflate the
    /// box by the syllables sitting on every other line — half a box too wide for
    /// a six-line label. Scale by the factor the correction applies to the
    /// unwrapped string instead, which is exact when the text fits one line and
    /// tracks the longest line's share of the Hangul otherwise.
    fn scaled(&self, mut metrics: TextMetrics, text: &str, style: &TextStyle) -> TextMetrics {
        let deficit = self.hangul_deficit_px(text, style);
        if deficit == 0.0 {
            return metrics;
        }
        let unwrapped = self.delegate.measure(text, style).width;
        if unwrapped <= 0.0 {
            return metrics;
        }
        metrics.width *= (unwrapped + deficit) / unwrapped;
        metrics
    }
}

impl TextMeasurer for HangulCorrectedMeasurer {
    fn measure(&self, text: &str, style: &TextStyle) -> TextMetrics {
        self.corrected(self.delegate.measure(text, style), text, style)
    }

    fn measure_wrapped(
        &self,
        text: &str,
        style: &TextStyle,
        max_width: Option<f64>,
        wrap_mode: WrapMode,
    ) -> TextMetrics {
        // The delegate still decides the line breaks from its own widths, so a
        // Korean label can wrap up to one word later than it should. The width
        // it reports is corrected, which is what sizes the box.
        self.scaled(
            self.delegate
                .measure_wrapped(text, style, max_width, wrap_mode),
            text,
            style,
        )
    }

    fn measure_wrapped_raw(
        &self,
        text: &str,
        style: &TextStyle,
        max_width: Option<f64>,
        wrap_mode: WrapMode,
    ) -> TextMetrics {
        self.scaled(
            self.delegate
                .measure_wrapped_raw(text, style, max_width, wrap_mode),
            text,
            style,
        )
    }

    fn measure_wrapped_with_raw_width(
        &self,
        text: &str,
        style: &TextStyle,
        max_width: Option<f64>,
        wrap_mode: WrapMode,
    ) -> (TextMetrics, Option<f64>) {
        let (metrics, raw) = self
            .delegate
            .measure_wrapped_with_raw_width(text, style, max_width, wrap_mode);
        // `raw` is the unwrapped width of the whole string, so it takes the full
        // deficit; `metrics.width` is the longest line and is scaled.
        let deficit = self.hangul_deficit_px(text, style);
        (self.scaled(metrics, text, style), raw.map(|w| w + deficit))
    }

    fn measure_svg_text_computed_length_px(&self, text: &str, style: &TextStyle) -> f64 {
        self.delegate
            .measure_svg_text_computed_length_px(text, style)
            + self.hangul_deficit_px(text, style)
    }

    /// Split the correction evenly across both extents. Exact for the centered
    /// anchors these bbox probes are used for; for a left-anchored run it shifts
    /// the reported box by half the deficit, which only feeds viewport sizing.
    fn measure_svg_text_bbox_x(&self, text: &str, style: &TextStyle) -> (f64, f64) {
        let (left, right) = self.delegate.measure_svg_text_bbox_x(text, style);
        let half = self.hangul_deficit_px(text, style) / 2.0;
        (left + half, right + half)
    }

    fn measure_svg_text_bbox_x_with_ascii_overhang(
        &self,
        text: &str,
        style: &TextStyle,
    ) -> (f64, f64) {
        let (left, right) = self
            .delegate
            .measure_svg_text_bbox_x_with_ascii_overhang(text, style);
        let half = self.hangul_deficit_px(text, style) / 2.0;
        (left + half, right + half)
    }

    fn measure_svg_title_bbox_x(&self, text: &str, style: &TextStyle) -> (f64, f64) {
        let (left, right) = self.delegate.measure_svg_title_bbox_x(text, style);
        let half = self.hangul_deficit_px(text, style) / 2.0;
        (left + half, right + half)
    }

    fn measure_svg_simple_text_bbox_width_px(&self, text: &str, style: &TextStyle) -> f64 {
        self.delegate
            .measure_svg_simple_text_bbox_width_px(text, style)
            + self.hangul_deficit_px(text, style)
    }

    fn measure_svg_raw_text_bbox_width_px(&self, text: &str, style: &TextStyle) -> f64 {
        self.delegate
            .measure_svg_raw_text_bbox_width_px(text, style)
            + self.hangul_deficit_px(text, style)
    }

    fn measure_svg_simple_text_bbox_width_for_wrap_px(&self, text: &str, style: &TextStyle) -> f64 {
        self.delegate
            .measure_svg_simple_text_bbox_width_for_wrap_px(text, style)
            + self.hangul_deficit_px(text, style)
    }

    fn measure_svg_simple_text_bbox_height_px(&self, text: &str, style: &TextStyle) -> f64 {
        // Line height comes from the font size, not the glyph widths.
        self.delegate
            .measure_svg_simple_text_bbox_height_px(text, style)
    }
}

/// Advance of one Hangul syllable, in em, as the rasterizer's font stack paints
/// it. Probed once per font family; `None` when the probe can't be laid out.
///
/// Keyed by family alone: Hangul faces advance their syllables uniformly across
/// weights, and mermaid only varies weight for emphasis inside a label.
fn painted_syllable_advance_em(style: &TextStyle) -> Option<f64> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<f64>>>> = OnceLock::new();
    let family = style.font_family.clone().unwrap_or_default();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    // A poisoned lock only means another thread panicked mid-probe; the map is
    // still a valid cache, so keep using it rather than failing the diagram.
    let mut cache = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(cached) = cache.get(&family) {
        return *cached;
    }
    let probed = probe_syllable_advance_em(&family);
    cache.insert(family, probed);
    probed
}

/// Difference between the ink of two syllables and one, which is exactly one
/// advance — side bearings cancel out.
fn probe_syllable_advance_em(family: &str) -> Option<f64> {
    let one = probe_ink_width(PROBE_SYLLABLE, family)?;
    let two = probe_ink_width(&PROBE_SYLLABLE.repeat(2), family)?;
    Some(f64::from(two - one) / PROBE_FONT_SIZE)
}

fn probe_ink_width(text: &str, family: &str) -> Option<f32> {
    let family = if family.trim().is_empty() {
        "sans-serif".to_owned()
    } else {
        family.to_owned()
    };
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="4000" height="400"><text x="0" y="200" font-family="{}" font-size="{PROBE_FONT_SIZE}">{text}</text></svg>"#,
        family.replace('"', "&quot;")
    );
    let tree = resvg::usvg::Tree::from_str(&svg, &super::visual::usvg_options()).ok()?;
    fn first_text(group: &resvg::usvg::Group) -> Option<f32> {
        for node in group.children() {
            match node {
                resvg::usvg::Node::Text(text) => return Some(text.abs_bounding_box().width()),
                resvg::usvg::Node::Group(inner) => {
                    if let Some(width) = first_text(inner) {
                        return Some(width);
                    }
                }
                _ => {}
            }
        }
        None
    }
    first_text(tree.root())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> TextStyle {
        TextStyle {
            font_family: Some("\"trebuchet ms\", verdana, arial, sans-serif".to_owned()),
            font_size: 16.0,
            ..Default::default()
        }
    }

    #[test]
    fn latin_text_is_measured_exactly_as_merman_would() {
        let corrected = HangulCorrectedMeasurer::default();
        let plain = VendoredFontMetricsTextMeasurer::default();
        for text in ["schedule tick", "payload received", "Ingest pipeline", ""] {
            assert_eq!(
                corrected.measure(text, &style()).width,
                plain.measure(text, &style()).width,
                "{text:?} must not be touched"
            );
            assert_eq!(
                corrected
                    .measure_wrapped(text, &style(), Some(200.0), WrapMode::SvgLike)
                    .width,
                plain
                    .measure_wrapped(text, &style(), Some(200.0), WrapMode::SvgLike)
                    .width
            );
        }
    }

    #[test]
    fn han_and_kana_are_not_touched_because_merman_already_agrees() {
        let corrected = HangulCorrectedMeasurer::default();
        let plain = VendoredFontMetricsTextMeasurer::default();
        for text in ["漢字", "ひらがな", "カタカナ", "。！"] {
            assert_eq!(
                corrected.measure(text, &style()).width,
                plain.measure(text, &style()).width,
                "{text:?} is already measured at a full em"
            );
        }
    }

    #[test]
    fn hangul_is_widened_towards_what_the_rasterizer_paints() {
        let corrected = HangulCorrectedMeasurer::default();
        let plain = VendoredFontMetricsTextMeasurer::default();
        let label = "스케줄 틱 발생";
        let before = plain.measure(label, &style()).width;
        let after = corrected.measure(label, &style()).width;
        let painted = f64::from(
            probe_ink_width(label, style().font_family.as_deref().unwrap_or_default())
                .expect("probe lays out"),
        ) / PROBE_FONT_SIZE
            * style().font_size;
        // The corrected width is a sum of advances while `painted` is the ink
        // box, so the corrected value sits a side bearing above it — closing the
        // gap most of the way is the goal, not landing exactly on it.
        let was_off = (before - painted).abs();
        let now_off = (after - painted).abs();
        assert!(
            now_off < was_off / 4.0 && now_off < 2.0,
            "correction should close the gap to the painted {painted}px: \
             was off by {was_off}px ({before}), now off by {now_off}px ({after})"
        );
    }

    /// A wrapped label's reported width is its longest line, so the correction
    /// has to be the longest line's share — not every syllable in the string.
    /// The bound below is derived from how many syllables can even fit on that
    /// line, so it holds regardless of how the correction is computed.
    #[test]
    fn a_wrapped_label_is_not_widened_by_the_syllables_on_its_other_lines() {
        let corrected = HangulCorrectedMeasurer::default();
        let plain = VendoredFontMetricsTextMeasurer::default();
        let text = "백그라운드 수집기가 큐를 폴링하여 스키마를 검증하고 저장합니다";
        let narrow = Some(100.0);
        let wrapped = plain.measure_wrapped(text, &style(), narrow, WrapMode::SvgLike);
        assert!(
            wrapped.line_count > 1,
            "fixture must actually wrap (got {} line)",
            wrapped.line_count
        );

        let got = corrected
            .measure_wrapped(text, &style(), narrow, WrapMode::SvgLike)
            .width;
        assert!(got > wrapped.width, "Hangul must still be widened");

        let per_syllable = corrected.hangul_deficit_px("가", &style());
        let advance = corrected.assumed_syllable_advance_px(&style());
        // One syllable of slack for the trailing partial glyph.
        let fits_on_longest_line = (wrapped.width / advance).ceil() + 1.0;
        assert!(
            got - wrapped.width <= fits_on_longest_line * per_syllable,
            "widened by {:.2}px, but at most {:.0} syllables fit the longest line \
             ({:.2}px) — the other lines' syllables are leaking into the box width",
            got - wrapped.width,
            fits_on_longest_line,
            fits_on_longest_line * per_syllable
        );
    }

    #[test]
    fn a_wrapped_label_is_never_wider_than_the_same_label_unwrapped() {
        let corrected = HangulCorrectedMeasurer::default();
        let text = "데이터 보관 정책 검증 진행 중";
        let wrapped = corrected
            .measure_wrapped(text, &style(), Some(100.0), WrapMode::SvgLike)
            .width;
        let unwrapped = corrected
            .measure_wrapped(text, &style(), Some(10_000.0), WrapMode::SvgLike)
            .width;
        assert!(
            wrapped <= unwrapped,
            "wrapped {wrapped} must not exceed unwrapped {unwrapped}"
        );
    }

    #[test]
    fn the_correction_scales_with_the_number_of_syllables() {
        let corrected = HangulCorrectedMeasurer::default();
        let one = corrected.hangul_deficit_px("가", &style());
        let four = corrected.hangul_deficit_px("가가가가", &style());
        assert!(one > 0.0, "expected merman to under-measure a syllable");
        assert!(
            (four - 4.0 * one).abs() < 0.01,
            "deficit must be additive ({one} per syllable, but four gave {four})"
        );
    }

    #[test]
    fn jamo_are_left_alone() {
        let corrected = HangulCorrectedMeasurer::default();
        assert_eq!(corrected.hangul_deficit_px("ㄱㄴㄷ", &style()), 0.0);
    }

    #[test]
    fn a_syllable_advance_is_probed_from_the_real_font_stack() {
        let em = painted_syllable_advance_em(&style()).expect("probe resolves");
        assert!(
            (0.5..=1.2).contains(&em),
            "a Hangul advance of {em}em is not plausible"
        );
    }

    #[test]
    fn height_is_never_corrected() {
        let corrected = HangulCorrectedMeasurer::default();
        let plain = VendoredFontMetricsTextMeasurer::default();
        assert_eq!(
            corrected.measure("스케줄", &style()).height,
            plain.measure("스케줄", &style()).height
        );
    }
}
