use unicode_width::UnicodeWidthStr as _;

use super::text_metrics::byte_index_for_column_in_line;

/// Column range (inclusive, 1-indexed) of the URL under `col`, together
/// with the URL itself. Returns `None` if the cell is not inside an
/// http(s) URL.
///
/// Mirrors Alacritty's `highlighted_at()` (display/hint.rs:389) —
/// the hint module returns a grid-coordinate range so the renderer can
/// inject `Flags::UNDERLINE` on the exact cells.
pub(super) fn url_range_at_column_in_line(line: &str, col: u16) -> Option<(u16, u16, String)> {
    let url = url_at_column_in_line(line, col)?;

    // Locate the URL inside the line. Multiple occurrences are rare in
    // practice; when they occur, prefer the one covering `col`.
    let byte_at_col = byte_index_for_column_in_line(line, col);
    let candidates: Vec<usize> = line
        .match_indices(url.as_str())
        .map(|(start, _)| start)
        .collect();
    let start_byte = candidates
        .iter()
        .copied()
        .find(|&start| (start..start + url.len()).contains(&byte_at_col))
        .or_else(|| candidates.first().copied())?;
    let end_byte = start_byte + url.len();

    let start_col = column_at_byte_index(line, start_byte);
    let end_col = column_at_byte_index(line, end_byte).saturating_sub(1);
    Some((start_col, end_col, url))
}

/// Inverse of `byte_index_for_column_in_line`: given a byte offset,
/// return the 1-indexed display column immediately after it.
pub(super) fn column_at_byte_index(line: &str, byte_index: usize) -> u16 {
    if byte_index == 0 {
        return 1;
    }
    let clamped = byte_index.min(line.len());
    let slice = line.get(..clamped).unwrap_or("");
    let width = slice.width() as u16;
    width.saturating_add(1)
}

fn is_url_byte(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9')
        || matches!(
            b,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b':'
                | b'/'
                | b'?'
                | b'#'
                | b'['
                | b']'
                | b'@'
                | b'!'
                | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
                | b'%'
        )
}

pub(super) fn url_at_byte_index(text: &str, index: usize) -> Option<String> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    let mut idx = index.min(bytes.len().saturating_sub(1));

    if !is_url_byte(bytes[idx]) && idx > 0 && is_url_byte(bytes[idx - 1]) {
        idx -= 1;
    }

    if !is_url_byte(bytes[idx]) {
        return None;
    }

    let mut start = idx;
    while start > 0 && is_url_byte(bytes[start - 1]) {
        start -= 1;
    }

    let mut end = idx + 1;
    while end < bytes.len() && is_url_byte(bytes[end]) {
        end += 1;
    }

    while end > start
        && matches!(
            bytes[end - 1],
            b'.' | b',' | b')' | b']' | b'}' | b';' | b':' | b'!' | b'?'
        )
    {
        end -= 1;
    }

    let candidate = std::str::from_utf8(&bytes[start..end]).ok()?;
    if is_openable_url(candidate) {
        Some(candidate.to_string())
    } else {
        None
    }
}

pub(super) fn is_openable_url(candidate: &str) -> bool {
    candidate.starts_with("https://") || candidate.starts_with("http://")
}

pub(super) fn url_at_column_in_line(line: &str, col: u16) -> Option<String> {
    if line.is_empty() {
        return None;
    }

    let local = byte_index_for_column_in_line(line, col).min(line.len().saturating_sub(1));
    url_at_byte_index(line, local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_https_links() {
        let text = "Visit https://google.com for search";
        let idx = text.find("google").unwrap();
        assert_eq!(
            url_at_byte_index(text, idx).as_deref(),
            Some("https://google.com")
        );
    }

    #[test]
    fn finds_https_links_by_cell_column() {
        let line = "https://google.com";
        assert_eq!(
            url_at_column_in_line(line, 1).as_deref(),
            Some("https://google.com")
        );
        assert_eq!(
            url_at_column_in_line(line, 10).as_deref(),
            Some("https://google.com")
        );
    }

    #[test]
    fn range_spans_whole_url_from_bare_line() {
        let line = "https://example.org/foo";
        let (start, end, url) = url_range_at_column_in_line(line, 1).unwrap();
        assert_eq!(start, 1);
        assert_eq!(end, line.len() as u16);
        assert_eq!(url, "https://example.org/foo");
    }

    #[test]
    fn range_spans_only_url_portion_with_surrounding_text() {
        let line = "see https://a.b/c for docs";
        let (start, end, url) = url_range_at_column_in_line(line, 6).unwrap();
        assert_eq!(url, "https://a.b/c");
        // "see " = 4 chars, URL starts at column 5 (1-indexed).
        assert_eq!(start, 5);
        assert_eq!(end, 5 + url.len() as u16 - 1);
    }

    #[test]
    fn range_none_on_non_url_text() {
        assert!(url_range_at_column_in_line("plain text", 3).is_none());
        assert!(url_range_at_column_in_line("", 1).is_none());
    }

    #[test]
    fn range_trailing_punctuation_is_excluded() {
        let line = "see https://a.b/c.";
        let (_, end, url) = url_range_at_column_in_line(line, 6).unwrap();
        assert_eq!(url, "https://a.b/c");
        // Trailing '.' (byte 18, col 18) must not be part of the range.
        assert!(end < line.len() as u16);
    }

    #[test]
    fn range_with_multibyte_prefix() {
        // Korean leading text — each Hangul char is width 2.
        let line = "방문 https://x.y/";
        let col_after_korean = 1 + "방문 ".width() as u16; // 1-indexed start of 'h'
        let (start, _, url) = url_range_at_column_in_line(line, col_after_korean).unwrap();
        assert_eq!(url, "https://x.y/");
        assert_eq!(start, col_after_korean);
    }

    #[test]
    fn column_at_byte_index_handles_boundaries() {
        assert_eq!(column_at_byte_index("abc", 0), 1);
        assert_eq!(column_at_byte_index("abc", 1), 2);
        assert_eq!(column_at_byte_index("abc", 3), 4);
        assert_eq!(column_at_byte_index("abc", 99), 4); // clamps
    }

    #[test]
    fn openable_url_filter_allows_only_http_schemes() {
        assert!(is_openable_url("https://example.com"));
        assert!(is_openable_url("http://example.com"));
        assert!(!is_openable_url("javascript:alert(1)"));
        assert!(!is_openable_url("file:///etc/passwd"));
        assert!(!is_openable_url("daruda://open"));
    }
}
