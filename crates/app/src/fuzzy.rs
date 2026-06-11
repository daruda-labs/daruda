//! GPUI-free fuzzy matching over candidate strings, backed by
//! `nucleo-matcher`. Ranks candidates by subsequence-match quality so
//! pickers (the Lane switcher today, more later) surface the best
//! matches first.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// Rank `candidates` against `query`, returning their indices with the
/// best match first. An empty (or whitespace-only) query returns every
/// index in its original order — no filtering. Matching is smart-case
/// (a lowercase query is case-insensitive; any uppercase letter makes
/// it case-sensitive) with smart Unicode normalization.
pub fn fuzzy_match<S: AsRef<str>>(query: &str, candidates: &[S]) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..candidates.len()).collect();
    }
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut buf = Vec::new();
    let mut scored: Vec<(usize, u32)> = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let haystack = Utf32Str::new(candidate.as_ref(), &mut buf);
        if let Some(score) = pattern.score(haystack, &mut matcher) {
            scored.push((index, score));
        }
    }
    // Higher score first; ties fall back to original order for a stable,
    // predictable ordering.
    scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.into_iter().map(|(index, _)| index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_returns_all_in_original_order() {
        let candidates = ["alpha", "beta", "gamma"];
        assert_eq!(fuzzy_match("", &candidates), vec![0, 1, 2]);
        assert_eq!(fuzzy_match("   ", &candidates), vec![0, 1, 2]);
    }

    #[test]
    fn non_matching_query_returns_empty() {
        let candidates = ["alpha", "beta"];
        assert!(fuzzy_match("zzz", &candidates).is_empty());
    }

    #[test]
    fn ranks_and_filters_subsequence_matches() {
        let candidates = ["main", "feature/login", "fix/login-bug"];
        let ranked = fuzzy_match("login", &candidates);
        // "main" has no `login` subsequence, so it is filtered out.
        assert!(!ranked.contains(&0));
        assert_eq!(ranked.len(), 2);
        assert!(ranked.contains(&1) && ranked.contains(&2));
    }

    #[test]
    fn smart_case_lowercase_query_is_case_insensitive() {
        let candidates = ["Daruda", "daruda-feat"];
        assert_eq!(fuzzy_match("dar", &candidates).len(), 2);
    }
}
