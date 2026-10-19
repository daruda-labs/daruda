//! A node failure as the log keeps it.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

use super::millis;
use crate::runner::NodeFailure;

/// [`NodeFailure`], field for field, so a replayed `NodeFailed` is the one
/// the run reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FailureLine {
    Timeout {
        elapsed_ms: u64,
    },
    NoOutput {
        expected: PathBuf,
    },
    OutputNotAFile {
        expected: PathBuf,
    },
    OutputEscapes {
        expected: PathBuf,
        resolved: PathBuf,
    },
    OutputSchema {
        expected: PathBuf,
        problem: String,
        more: usize,
    },
    ContextExhausted,
    TurnLimit,
    Refused,
    TurnFailed {
        message: String,
    },
    SessionError {
        message: String,
    },
    UnsupportedSetting {
        field: String,
        value: String,
        available: Vec<String>,
    },
    PermissionDenied {
        tool: String,
    },
    SettingRejected {
        config_id: String,
        reason: String,
    },
    Exit {
        code: Option<i32>,
    },
    Interrupted,
    Recorded {
        message: String,
    },
}

/// The settings a node can pin, so a replayed failure names the same
/// `&'static` field the live one did.
const SETTING_FIELDS: [&str; 3] = ["model", "effort", "mode"];
const UNKNOWN_SETTING: &str = "setting";

impl From<&NodeFailure> for FailureLine {
    fn from(failure: &NodeFailure) -> Self {
        match failure.clone() {
            NodeFailure::Timeout { elapsed } => Self::Timeout {
                elapsed_ms: millis(elapsed),
            },
            NodeFailure::NoOutput { expected } => Self::NoOutput { expected },
            NodeFailure::OutputNotAFile { expected } => Self::OutputNotAFile { expected },
            NodeFailure::OutputEscapes { expected, resolved } => {
                Self::OutputEscapes { expected, resolved }
            }
            NodeFailure::OutputSchema {
                expected,
                problem,
                more,
            } => Self::OutputSchema {
                expected,
                problem,
                more,
            },
            NodeFailure::ContextExhausted => Self::ContextExhausted,
            NodeFailure::TurnLimit => Self::TurnLimit,
            NodeFailure::Refused => Self::Refused,
            NodeFailure::TurnFailed(message) => Self::TurnFailed { message },
            NodeFailure::SessionError(message) => Self::SessionError { message },
            NodeFailure::UnsupportedSetting {
                field,
                value,
                available,
            } => Self::UnsupportedSetting {
                field: field.to_string(),
                value,
                available,
            },
            NodeFailure::PermissionDenied { tool } => Self::PermissionDenied { tool },
            NodeFailure::SettingRejected { config_id, reason } => {
                Self::SettingRejected { config_id, reason }
            }
            NodeFailure::Exit { code } => Self::Exit { code },
            NodeFailure::Interrupted => Self::Interrupted,
            NodeFailure::Recorded(message) => Self::Recorded { message },
        }
    }
}

impl From<&FailureLine> for NodeFailure {
    fn from(line: &FailureLine) -> Self {
        match line.clone() {
            FailureLine::Timeout { elapsed_ms } => Self::Timeout {
                elapsed: Duration::from_millis(elapsed_ms),
            },
            FailureLine::NoOutput { expected } => Self::NoOutput { expected },
            FailureLine::OutputNotAFile { expected } => Self::OutputNotAFile { expected },
            FailureLine::OutputEscapes { expected, resolved } => {
                Self::OutputEscapes { expected, resolved }
            }
            FailureLine::OutputSchema {
                expected,
                problem,
                more,
            } => Self::OutputSchema {
                expected,
                problem,
                more,
            },
            FailureLine::ContextExhausted => Self::ContextExhausted,
            FailureLine::TurnLimit => Self::TurnLimit,
            FailureLine::Refused => Self::Refused,
            FailureLine::TurnFailed { message } => Self::TurnFailed(message),
            FailureLine::SessionError { message } => Self::SessionError(message),
            FailureLine::UnsupportedSetting {
                field,
                value,
                available,
            } => Self::UnsupportedSetting {
                field: SETTING_FIELDS
                    .into_iter()
                    .find(|known| *known == field)
                    .unwrap_or(UNKNOWN_SETTING),
                value,
                available,
            },
            FailureLine::PermissionDenied { tool } => Self::PermissionDenied { tool },
            FailureLine::SettingRejected { config_id, reason } => {
                Self::SettingRejected { config_id, reason }
            }
            FailureLine::Exit { code } => Self::Exit { code },
            FailureLine::Interrupted => Self::Interrupted,
            FailureLine::Recorded { message } => Self::Recorded(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A replayed failure is the failure the run reported, variant for
    /// variant — `NodeFailed` from the log must match the live one.
    #[test]
    fn every_failure_reads_back_as_itself() {
        let path = PathBuf::from("/run/out.md");
        let failures = [
            NodeFailure::Timeout {
                elapsed: Duration::from_millis(1500),
            },
            NodeFailure::NoOutput {
                expected: path.clone(),
            },
            NodeFailure::OutputNotAFile {
                expected: path.clone(),
            },
            NodeFailure::OutputEscapes {
                expected: path.clone(),
                resolved: PathBuf::from("/elsewhere"),
            },
            NodeFailure::OutputSchema {
                expected: path,
                problem: "missing `a`".to_string(),
                more: 2,
            },
            NodeFailure::ContextExhausted,
            NodeFailure::TurnLimit,
            NodeFailure::Refused,
            NodeFailure::TurnFailed("x".to_string()),
            NodeFailure::SessionError("y".to_string()),
            NodeFailure::UnsupportedSetting {
                field: "effort",
                value: "max".to_string(),
                available: vec!["low".to_string()],
            },
            NodeFailure::PermissionDenied {
                tool: "write".to_string(),
            },
            NodeFailure::SettingRejected {
                config_id: "model".to_string(),
                reason: "no".to_string(),
            },
            NodeFailure::Exit { code: Some(2) },
            NodeFailure::Exit { code: None },
            NodeFailure::Interrupted,
            NodeFailure::Recorded("exited with status 2".to_string()),
        ];
        for failure in failures {
            let line = FailureLine::from(&failure);
            let text = serde_json::to_string(&line).expect("serialize");
            let back: FailureLine = serde_json::from_str(&text).expect("deserialize");
            assert_eq!(NodeFailure::from(&back), failure);
        }
    }
}
