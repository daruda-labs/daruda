//! ACP failure classification and the remedy each class implies.
//!
//! A failed ACP exchange arrives as a JSON-RPC error whose *code* and *data*
//! say what went wrong; the host needs that to decide what to offer the user.
//! [`classify`] reads both and never pattern-matches the human-readable
//! message — the adapter documents `data.errorKind` as "a convention for ACP
//! clients to dispatch on without having to pattern-match the human-readable
//! message text", and message text is not stable across adapter releases.
//!
//! Two protocol shapes carry authentication failures, and **their remedies are
//! opposite**:
//!
//! - `-32000` ([`ErrorCode::AuthRequired`]) — the agent CLI is asking to be
//!   logged in again. Re-login is exactly the fix.
//! - `-32603` with `data.errorKind = "oauth_org_not_allowed"` — the
//!   organization blocks subscription access. Re-login changes nothing; the
//!   user needs an API key or an administrator.
//!
//! So "auth failed → offer re-login" is wrong as a single rule. Callers read
//! [`AcpFailure::remedy`] instead of inspecting the class directly.
//!
//! The `errorKind` strings are wire constants of the Claude Agent SDK
//! (`SDKAssistantMessageError`); this module is their designated home. The set
//! is closed *per SDK version* but grows across versions, so
//! [`FailureKind::Other`] keeps an unknown value renderable instead of
//! swallowing it.

use agent_client_protocol::{Error as AcpProtocolError, ErrorCode};

use crate::node::NodeError;

/// What the host can offer the user for a given failure.
///
/// `Retry`'s *action* is deliberately not encoded here: retrying a terminal
/// session error means reconnecting, while retrying a turn error means
/// re-sending the prompt. The event that carried the failure decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remedy {
    /// Signing in again fixes this.
    Reauthenticate,
    /// Transient — the same request may succeed.
    Retry,
    /// Fixable inside the app's own settings.
    Configure,
    /// Only actionable outside the app (administrator, billing, manual
    /// install). Show the guidance; offer no button that pretends otherwise.
    ExternalAction,
    /// Nothing to offer — render the message and stop.
    NoneAvailable,
}

/// Mirror of the Claude Agent SDK's `SDKAssistantMessageError`, which the
/// adapter forwards verbatim as `data.errorKind`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureKind {
    AuthenticationFailed,
    OauthOrgNotAllowed,
    Billing,
    RateLimit,
    Overloaded,
    ServerError,
    InvalidRequest,
    ModelNotFound,
    MaxOutputTokens,
    Unknown,
    /// A value this build does not know — a newer SDK than the one this
    /// mapping was written against. Kept whole so the host can still show it.
    Other(String),
}

impl FailureKind {
    /// Parse a wire `errorKind`. Unknown values become [`Self::Other`] rather
    /// than an error: the SDK adds variants between releases, and dropping one
    /// would silently downgrade a classifiable failure to an opaque one.
    #[must_use]
    pub fn from_wire(value: &str) -> Self {
        match value {
            "authentication_failed" => Self::AuthenticationFailed,
            "oauth_org_not_allowed" => Self::OauthOrgNotAllowed,
            "billing_error" => Self::Billing,
            "rate_limit" => Self::RateLimit,
            "overloaded" => Self::Overloaded,
            "server_error" => Self::ServerError,
            "invalid_request" => Self::InvalidRequest,
            "model_not_found" => Self::ModelNotFound,
            "max_output_tokens" => Self::MaxOutputTokens,
            "unknown" => Self::Unknown,
            other => Self::Other(other.to_owned()),
        }
    }

    #[must_use]
    pub fn remedy(&self) -> Remedy {
        match self {
            Self::AuthenticationFailed => Remedy::Reauthenticate,
            // Re-login cannot lift an organization policy or settle a bill.
            Self::OauthOrgNotAllowed | Self::Billing => Remedy::ExternalAction,
            Self::RateLimit | Self::Overloaded | Self::ServerError => Remedy::Retry,
            Self::InvalidRequest | Self::ModelNotFound => Remedy::Configure,
            Self::MaxOutputTokens | Self::Unknown | Self::Other(_) => Remedy::NoneAvailable,
        }
    }
}

/// Which node-provisioning step failed.
///
/// [`NodeError`] itself is not carried: it derives neither `Clone` nor
/// `PartialEq`, and this type ends up inside the chat render model, which
/// needs both. The variant plus [`NodeError`]'s own `Display` text (already
/// written to name the remedy) is everything the host needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeKind {
    UnsupportedPlatform,
    Download,
    Checksum,
    Extract,
}

impl RuntimeKind {
    #[must_use]
    pub fn remedy(self) -> Remedy {
        match self {
            // The user has to install Node.js themselves.
            Self::UnsupportedPlatform => Remedy::ExternalAction,
            Self::Download | Self::Extract => Remedy::Retry,
            // A checksum mismatch is an integrity signal, not a hiccup.
            // Offering "try again" would train the user to click past it.
            Self::Checksum => Remedy::NoneAvailable,
        }
    }
}

/// A classified ACP failure. Every variant carries the message the user sees,
/// so the host never has to reach back into protocol types to render one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcpFailure {
    /// `-32000` — the agent asked to be authenticated again.
    AuthRequired { message: String },
    /// `data.errorKind` named a category.
    Categorized { kind: FailureKind, message: String },
    /// Provisioning the Node.js runtime failed before the agent ever ran.
    Runtime { kind: RuntimeKind, message: String },
    /// Neither a known code nor an `errorKind` — including host-synthesized
    /// failures that never came from the wire at all.
    Unclassified { message: String },
}

impl AcpFailure {
    /// Classify a protocol error by code and `data.errorKind` only.
    ///
    /// The message is taken from [`AcpProtocolError::message`] rather than its
    /// `Display`, which appends a pretty-printed copy of `data` — that is how
    /// a raw `{"errorKind": …}` blob ends up in front of users today. The
    /// structure now lives in the returned value instead.
    #[must_use]
    pub fn classify(error: &AcpProtocolError) -> Self {
        let message = error.message.clone();
        if matches!(error.code, ErrorCode::AuthRequired) {
            return Self::AuthRequired { message };
        }
        match error_kind(error) {
            Some(kind) => Self::Categorized { kind, message },
            None => Self::Unclassified { message },
        }
    }

    /// Classify a runtime-provisioning failure. `NodeError`'s `Display` is
    /// already user-facing, so it becomes the message verbatim.
    #[must_use]
    pub fn from_node_error(error: &NodeError) -> Self {
        let kind = match error {
            NodeError::UnsupportedPlatform(_) => RuntimeKind::UnsupportedPlatform,
            NodeError::Download(_) => RuntimeKind::Download,
            NodeError::Checksum { .. } => RuntimeKind::Checksum,
            NodeError::Extract(_) => RuntimeKind::Extract,
        };
        Self::Runtime {
            kind,
            message: error.to_string(),
        }
    }

    /// A failure the host raised itself, with no protocol error behind it.
    #[must_use]
    pub fn unclassified(message: impl Into<String>) -> Self {
        Self::Unclassified {
            message: message.into(),
        }
    }

    /// What the host can offer for this failure.
    #[must_use]
    pub fn remedy(&self) -> Remedy {
        match self {
            Self::AuthRequired { .. } => Remedy::Reauthenticate,
            Self::Categorized { kind, .. } => kind.remedy(),
            Self::Runtime { kind, .. } => kind.remedy(),
            Self::Unclassified { .. } => Remedy::NoneAvailable,
        }
    }

    /// The user-facing text for this failure.
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::AuthRequired { message }
            | Self::Categorized { message, .. }
            | Self::Runtime { message, .. }
            | Self::Unclassified { message } => message,
        }
    }
}

impl std::fmt::Display for AcpFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// Read `data.errorKind`, if the error carries one.
fn error_kind(error: &AcpProtocolError) -> Option<FailureKind> {
    let kind = error.data.as_ref()?.get("errorKind")?.as_str()?;
    Some(FailureKind::from_wire(kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(code: i32, message: &str, error_kind: Option<&str>) -> AcpProtocolError {
        let mut e = AcpProtocolError::new(code, message);
        if let Some(kind) = error_kind {
            e = e.data(serde_json::json!({ "errorKind": kind }));
        }
        e
    }

    /// The wire capture that motivated this module: a `session/prompt` that
    /// came back `-32000` with no `data` at all.
    #[test]
    fn auth_required_code_classifies_without_any_data() {
        let failure = AcpFailure::classify(&err(-32000, "Authentication required", None));
        assert_eq!(
            failure,
            AcpFailure::AuthRequired {
                message: "Authentication required".to_owned()
            }
        );
        assert_eq!(failure.remedy(), Remedy::Reauthenticate);
    }

    /// The second wire capture: an auth failure that arrives as a *generic*
    /// internal error and is only identifiable through `data.errorKind`.
    #[test]
    fn org_not_allowed_arrives_as_internal_error_and_is_not_reauthenticable() {
        let failure = AcpFailure::classify(&err(
            -32603,
            "Internal error: Your organization has disabled Claude subscription access",
            Some("oauth_org_not_allowed"),
        ));
        assert_eq!(
            failure.remedy(),
            Remedy::ExternalAction,
            "re-login cannot lift an org policy; offering it would send the user \
             through a browser flow that lands them right back here"
        );
    }

    /// Both auth shapes must reach the same remedy, so losing either detection
    /// path still leaves the user with a working button.
    #[test]
    fn both_authentication_shapes_agree_on_the_remedy() {
        let by_code = AcpFailure::classify(&err(-32000, "Authentication required", None));
        let by_kind =
            AcpFailure::classify(&err(-32603, "auth failed", Some("authentication_failed")));
        assert_eq!(by_code.remedy(), Remedy::Reauthenticate);
        assert_eq!(by_kind.remedy(), Remedy::Reauthenticate);
    }

    #[test]
    fn every_known_error_kind_maps_to_its_remedy() {
        let cases = [
            ("authentication_failed", Remedy::Reauthenticate),
            ("oauth_org_not_allowed", Remedy::ExternalAction),
            ("billing_error", Remedy::ExternalAction),
            ("rate_limit", Remedy::Retry),
            ("overloaded", Remedy::Retry),
            ("server_error", Remedy::Retry),
            ("invalid_request", Remedy::Configure),
            ("model_not_found", Remedy::Configure),
            ("max_output_tokens", Remedy::NoneAvailable),
            ("unknown", Remedy::NoneAvailable),
        ];
        for (wire, expected) in cases {
            assert_eq!(
                FailureKind::from_wire(wire).remedy(),
                expected,
                "errorKind {wire}"
            );
        }
    }

    /// The SDK adds variants between releases. An unrecognised one must stay
    /// renderable rather than collapsing into the same bucket as "no data".
    #[test]
    fn unknown_error_kind_is_kept_whole() {
        let failure = AcpFailure::classify(&err(-32603, "something new", Some("teapot_error")));
        assert_eq!(
            failure,
            AcpFailure::Categorized {
                kind: FailureKind::Other("teapot_error".to_owned()),
                message: "something new".to_owned(),
            }
        );
        assert_eq!(failure.remedy(), Remedy::NoneAvailable);
        assert_eq!(failure.message(), "something new");
    }

    #[test]
    fn internal_error_without_error_kind_is_unclassified() {
        let failure = AcpFailure::classify(&err(-32603, "Internal error: boom", None));
        assert_eq!(
            failure,
            AcpFailure::Unclassified {
                message: "Internal error: boom".to_owned()
            }
        );
        assert_eq!(failure.remedy(), Remedy::NoneAvailable);
    }

    /// `data` that is not an object, or carries no `errorKind`, must not panic
    /// or be mistaken for a category.
    #[test]
    fn data_without_a_string_error_kind_is_unclassified() {
        let shapes = [
            serde_json::json!({ "other": "field" }),
            serde_json::json!({ "errorKind": 7 }),
            serde_json::json!("bare string"),
            serde_json::json!([1, 2, 3]),
        ];
        for data in shapes {
            let error = AcpProtocolError::new(-32603, "boom").data(data.clone());
            assert!(
                matches!(
                    AcpFailure::classify(&error),
                    AcpFailure::Unclassified { .. }
                ),
                "data {data}"
            );
        }
    }

    /// The message must not carry a pretty-printed `data` blob — that is the
    /// JSON fragment users see on screen today.
    #[test]
    fn classified_message_drops_the_raw_data_blob() {
        let error = err(
            -32603,
            "Internal error: nope",
            Some("oauth_org_not_allowed"),
        );
        let failure = AcpFailure::classify(&error);
        assert_eq!(failure.message(), "Internal error: nope");
        assert!(
            !failure.message().contains("errorKind"),
            "structure belongs in the value, not in the text"
        );
        assert!(
            error.to_string().contains("errorKind"),
            "guards the premise: the protocol Display is what leaks it"
        );
    }

    #[test]
    fn node_errors_map_to_their_kind_and_keep_their_guidance() {
        let cases = [
            (
                NodeError::UnsupportedPlatform("plan9".to_owned()),
                RuntimeKind::UnsupportedPlatform,
                Remedy::ExternalAction,
            ),
            (
                NodeError::Download("timed out".to_owned()),
                RuntimeKind::Download,
                Remedy::Retry,
            ),
            (
                NodeError::Checksum {
                    expected: "aaa".to_owned(),
                    actual: "bbb".to_owned(),
                },
                RuntimeKind::Checksum,
                Remedy::NoneAvailable,
            ),
            (
                NodeError::Extract("disk full".to_owned()),
                RuntimeKind::Extract,
                Remedy::Retry,
            ),
        ];
        for (error, expected_kind, expected_remedy) in cases {
            let expected_message = error.to_string();
            let failure = AcpFailure::from_node_error(&error);
            assert_eq!(
                failure,
                AcpFailure::Runtime {
                    kind: expected_kind,
                    message: expected_message,
                }
            );
            assert_eq!(failure.remedy(), expected_remedy);
        }
    }

    /// A checksum mismatch must never be offered as "try again".
    #[test]
    fn checksum_failure_is_not_retryable() {
        assert_eq!(RuntimeKind::Checksum.remedy(), Remedy::NoneAvailable);
    }

    /// The host synthesises a failure when a connection task dies without
    /// emitting anything; it has no protocol error behind it.
    #[test]
    fn host_synthesised_failure_round_trips_its_message() {
        let failure = AcpFailure::unclassified("the agent stopped responding");
        assert_eq!(failure.message(), "the agent stopped responding");
        assert_eq!(failure.remedy(), Remedy::NoneAvailable);
        assert_eq!(failure.to_string(), "the agent stopped responding");
    }

    /// `daruda_flow` keeps a string-shaped failure; `Display` is that bridge.
    #[test]
    fn display_is_the_message() {
        let failure = AcpFailure::classify(&err(-32000, "Authentication required", None));
        assert_eq!(failure.to_string(), "Authentication required");
    }
}
