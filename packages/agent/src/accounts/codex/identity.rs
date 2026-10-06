//! Reads a managed Codex account's identity from `<CODEX_HOME>/auth.json`.
//! Credentials live in that plaintext file (no Keychain), and the identity
//! sits in the `tokens.id_token` JWT payload. Best-effort: any failure
//! yields an empty identity.

use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;

use crate::accounts::identity::AccountIdentity;

/// Plaintext credential file codex writes inside its `CODEX_HOME`.
const AUTH_FILE: &str = "auth.json";

/// Namespaced OIDC claim carrying the ChatGPT account + organization data.
const OPENAI_AUTH_CLAIM: &str = "https://api.openai.com/auth";

pub fn read_codex_identity(config_dir: &Path) -> AccountIdentity {
    read_id_token_claims(config_dir)
        .as_ref()
        .map(identity_from_claims)
        .unwrap_or_default()
}

/// `true` when `auth.json` parses into decodable identity claims.
pub fn has_codex_credentials(config_dir: &Path) -> bool {
    read_id_token_claims(config_dir).is_some()
}

fn read_id_token_claims(config_dir: &Path) -> Option<Value> {
    let raw = std::fs::read_to_string(config_dir.join(AUTH_FILE)).ok()?;
    let auth: Value = serde_json::from_str(&raw).ok()?;
    decode_jwt_payload(auth["tokens"]["id_token"].as_str()?)
}

/// Decode a JWT's payload segment. The signature is deliberately not
/// verified: this reads identity out of a local file the user already owns,
/// it is not an authorization decision.
fn decode_jwt_payload(token: &str) -> Option<Value> {
    let mut segments = token.split('.');
    let (_header, payload) = (segments.next()?, segments.next()?);
    segments.next()?;
    if segments.next().is_some() {
        return None;
    }
    // JWT segments are unpadded base64url; tolerate a padding producer.
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn identity_from_claims(claims: &Value) -> AccountIdentity {
    AccountIdentity {
        email: claims["email"].as_str().map(str::to_string),
        organization: default_organization_title(&claims[OPENAI_AUTH_CLAIM]["organizations"]),
    }
}

/// Title of the `is_default` organization, falling back to the first entry.
fn default_organization_title(organizations: &Value) -> Option<String> {
    let orgs = organizations.as_array()?;
    orgs.iter()
        .find(|org| org["is_default"].as_bool() == Some(true))
        .or_else(|| orgs.first())
        .and_then(|org| org["title"].as_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a three-segment JWT whose payload is `payload`.
    fn jwt(payload: Value) -> String {
        format!(
            "header.{}.signature",
            URL_SAFE_NO_PAD.encode(payload.to_string())
        )
    }

    /// Mirrors the real file's shape, `tokens.account_id` included: the usage
    /// source reads that key out of this same file for its
    /// `ChatGPT-Account-Id` header, so a fixture without it would let a change
    /// here silently break that.
    fn write_auth(dir: &Path, id_token: &str) {
        let auth = serde_json::json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": Value::Null,
            "tokens": {
                "id_token": id_token,
                "access_token": "a",
                "refresh_token": "r",
                "account_id": "ce62a47c-2346-4c67-a552-e94a60e0a946",
            },
            "last_refresh": "2026-07-28T00:00:00Z",
        });
        std::fs::write(dir.join(AUTH_FILE), auth.to_string()).expect("write auth.json");
    }

    fn full_payload() -> Value {
        serde_json::json!({
            "email": "alice@acme.com",
            "name": "Alice",
            "sub": "user-1",
            OPENAI_AUTH_CLAIM: {
                "chatgpt_account_id": "acct-1",
                "chatgpt_plan_type": "pro",
                "organizations": [
                    { "id": "o1", "is_default": false, "role": "member", "title": "Other" },
                    { "id": "o2", "is_default": true, "role": "owner", "title": "Acme Inc" },
                ],
            },
        })
    }

    #[test]
    fn extracts_email_and_default_organization_title() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_auth(dir.path(), &jwt(full_payload()));
        let id = read_codex_identity(dir.path());
        assert_eq!(id.email.as_deref(), Some("alice@acme.com"));
        assert_eq!(id.organization.as_deref(), Some("Acme Inc"));
    }

    #[test]
    fn falls_back_to_first_organization_when_none_is_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let payload = serde_json::json!({
            "email": "bob@acme.com",
            OPENAI_AUTH_CLAIM: { "organizations": [ { "title": "First" }, { "title": "Second" } ] },
        });
        write_auth(dir.path(), &jwt(payload));
        let id = read_codex_identity(dir.path());
        assert_eq!(id.organization.as_deref(), Some("First"));
    }

    #[test]
    fn organization_is_none_when_claim_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_auth(dir.path(), &jwt(serde_json::json!({ "email": "c@x.com" })));
        let id = read_codex_identity(dir.path());
        assert_eq!(id.email.as_deref(), Some("c@x.com"));
        assert!(id.organization.is_none());
    }

    #[test]
    fn missing_file_yields_empty_identity() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(read_codex_identity(dir.path()), AccountIdentity::default());
    }

    #[test]
    fn malformed_json_yields_empty_identity() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(AUTH_FILE), "{not json").expect("write");
        assert_eq!(read_codex_identity(dir.path()), AccountIdentity::default());
    }

    #[test]
    fn wrong_jwt_segment_count_yields_empty_identity() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Decodable payloads, so only the segment count can reject these.
        let payload = URL_SAFE_NO_PAD.encode(full_payload().to_string());
        for token in [
            format!("header.{payload}"),
            format!("header.{payload}.signature.extra"),
        ] {
            write_auth(dir.path(), &token);
            assert_eq!(
                read_codex_identity(dir.path()),
                AccountIdentity::default(),
                "rejected token: {token}"
            );
        }
    }

    #[test]
    fn non_base64_payload_yields_empty_identity() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_auth(dir.path(), "header.!!!not-base64!!!.signature");
        assert_eq!(read_codex_identity(dir.path()), AccountIdentity::default());
    }

    #[test]
    fn non_json_payload_yields_empty_identity() {
        let dir = tempfile::tempdir().expect("tempdir");
        let payload = URL_SAFE_NO_PAD.encode("plain text");
        write_auth(dir.path(), &format!("header.{payload}.signature"));
        assert_eq!(read_codex_identity(dir.path()), AccountIdentity::default());
    }

    /// The identity reader and the usage source both parse this one file. This
    /// pins the keys each needs, so a fixture drifting from the real shape
    /// can't hide a break in the other reader.
    #[test]
    fn the_auth_file_carries_what_both_readers_need() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_auth(dir.path(), &jwt(full_payload()));
        let raw = std::fs::read_to_string(dir.path().join(AUTH_FILE)).expect("read");
        let auth: Value = serde_json::from_str(&raw).expect("parse");

        // Identity reads the JWT claims; usage reads the bearer token and the
        // account id beside it.
        assert!(auth["tokens"]["id_token"].is_string());
        assert!(auth["tokens"]["access_token"].is_string());
        assert!(
            auth["tokens"]["account_id"].is_string(),
            "usage scopes its request with this key"
        );
    }

    #[test]
    fn has_credentials_is_false_for_an_empty_dir_and_true_for_a_fixture() {
        let empty = tempfile::tempdir().expect("tempdir");
        assert!(!has_codex_credentials(empty.path()));

        let filled = tempfile::tempdir().expect("tempdir");
        write_auth(filled.path(), &jwt(full_payload()));
        assert!(has_codex_credentials(filled.path()));
    }
}
