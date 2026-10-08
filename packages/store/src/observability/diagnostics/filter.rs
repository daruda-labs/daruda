//! Project allowlisted metadata and retain the newest bounded set of events.

use super::{super::error_report::ErrorReport, EVENT_LIMIT, LOOKBACK_DAYS};
use chrono::{DateTime, Utc};
use serde_json::json;
use std::{cmp::Reverse, collections::BinaryHeap};

pub(super) fn recent_events(
    files: &[super::selection::LogFile],
    now: DateTime<Utc>,
) -> std::io::Result<Vec<String>> {
    let cutoff = now - chrono::Duration::days(LOOKBACK_DAYS);
    // Retain only projected events: raw payloads never accumulate in memory.
    let mut events = BinaryHeap::new();
    let mut sequence = 0_u64;
    for file in files {
        for line in file.lines()? {
            let Ok(report) = serde_json::from_str::<ErrorReport>(&line) else {
                continue;
            };
            if report.timestamp < cutoff || report.timestamp > now {
                continue;
            }
            sequence += 1;
            events.push(Reverse((
                report.timestamp,
                sequence,
                public_event(&report).to_string(),
            )));
            if events.len() > EVENT_LIMIT {
                events.pop();
            }
        }
    }
    Ok(events
        .into_sorted_vec()
        .into_iter()
        .map(|Reverse((_, _, event))| event)
        .collect())
}

pub(super) fn public_event(report: &ErrorReport) -> serde_json::Value {
    let location = report.location.as_deref().filter(|location| {
        (location.starts_with("packages/") || location.starts_with("packages\\"))
            && location.len() < 512
            && !location.contains("..")
            && location.chars().all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, '/' | '\\' | '_' | '-' | '.' | ':')
            })
    });
    let known: Vec<_> = [
        "ENOENT",
        "EACCES",
        "EPERM",
        "ETIMEDOUT",
        "ECONNRESET",
        "os error 5",
        "os error 32",
        "os error 33",
        "os error 206",
    ]
    .into_iter()
    .filter(|code| {
        contains_error_code(&report.message, code)
            || report
                .source_chain
                .iter()
                .any(|source| contains_error_code(source, code))
            || report
                .context
                .get("detail")
                .is_some_and(|detail| contains_error_code(detail, code))
    })
    .collect();
    json!({"timestamp": report.timestamp, "severity": report.severity, "location": location, "error_codes": known})
}

/// Match complete codes, never a prefix of a number or part of an identifier.
fn contains_error_code(text: &str, code: &str) -> bool {
    let boundary = |c: char| !c.is_alphanumeric() && c != '_';
    text.match_indices(code).any(|(start, _)| {
        text[..start].chars().next_back().is_none_or(boundary)
            && text[start + code.len()..]
                .chars()
                .next()
                .is_none_or(boundary)
    })
}
