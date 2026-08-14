//! Reads account identity (email/org) from `<config_dir>/.claude.json`'s
//! `oauthAccount` — the authoritative on-disk source (the credentials blob
//! has no email). Best-effort: any failure yields an empty identity.

use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountIdentity {
    pub email: Option<String>,
    pub organization: Option<String>,
}

/// Config files that may carry the `oauthAccount` block, in the order the CLI
/// has been observed to write them.
const CONFIG_FILES: &[&str] = &[".claude.json", ".config.json"];

/// Field names the email has appeared under, most specific first.
const EMAIL_FIELDS: &[&str] = &["emailAddress", "email"];

pub fn read_account_identity(config_dir: &Path) -> AccountIdentity {
    let Some(oauth) = read_oauth_account(config_dir) else {
        return AccountIdentity::default();
    };
    AccountIdentity {
        email: first_string(&oauth, EMAIL_FIELDS),
        organization: first_string(&oauth, &["organizationName"]),
    }
}

/// The `oauthAccount` block from whichever config file carries one.
///
/// Two files rather than one, and neither is guaranteed: the CLI has written
/// this block under both names, and reading only the first leaves an account
/// with no captured identity at all — which surfaces as a nameless row the
/// user cannot tell from their other accounts.
fn read_oauth_account(config_dir: &Path) -> Option<serde_json::Value> {
    for name in CONFIG_FILES {
        let Ok(raw) = std::fs::read_to_string(config_dir.join(name)) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let oauth = &v["oauthAccount"];
        if oauth.is_object() {
            return Some(oauth.clone());
        }
    }
    None
}

/// First of `fields` present as a non-empty string.
fn first_string(v: &serde_json::Value, fields: &[&str]) -> Option<String> {
    fields
        .iter()
        .filter_map(|f| v[*f].as_str())
        .find(|s| !s.trim().is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_email_and_org_from_oauth_account() {
        let dir = std::env::temp_dir().join(format!("daruda-id-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"alice@acme.com","organizationName":"Acme"}}"#,
        )
        .unwrap();
        let id = read_account_identity(&dir);
        assert_eq!(id.email.as_deref(), Some("alice@acme.com"));
        assert_eq!(id.organization.as_deref(), Some("Acme"));
        std::fs::remove_dir_all(&dir).ok();
    }
    /// The block has been written under a second file name too; reading only
    /// the first leaves the account with no identity at all.
    #[test]
    fn reads_the_oauth_account_from_the_alternate_config_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(".config.json"),
            r#"{"oauthAccount":{"emailAddress":"b@x.com","organizationName":"Org"}}"#,
        )
        .unwrap();
        let id = read_account_identity(dir.path());
        assert_eq!(id.email.as_deref(), Some("b@x.com"));
    }

    /// `.claude.json` wins when both carry a block.
    #[test]
    fn the_primary_config_file_wins() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"primary@x.com"}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".config.json"),
            r#"{"oauthAccount":{"emailAddress":"alternate@x.com"}}"#,
        )
        .unwrap();
        assert_eq!(
            read_account_identity(dir.path()).email.as_deref(),
            Some("primary@x.com")
        );
    }

    /// A file present but carrying no block must not stop the search.
    #[test]
    fn a_blockless_config_file_falls_through_to_the_next() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(".claude.json"), r#"{"other":1}"#).unwrap();
        std::fs::write(
            dir.path().join(".config.json"),
            r#"{"oauthAccount":{"email":"fallback@x.com"}}"#,
        )
        .unwrap();
        assert_eq!(
            read_account_identity(dir.path()).email.as_deref(),
            Some("fallback@x.com"),
            "the alternate file and the alternate field name both have to be tried"
        );
    }

    /// An empty string is not an identity — it renders as a blank row.
    #[test]
    fn a_blank_email_is_not_captured() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"  ","email":"real@x.com"}}"#,
        )
        .unwrap();
        assert_eq!(
            read_account_identity(dir.path()).email.as_deref(),
            Some("real@x.com")
        );
    }

    #[test]
    fn missing_file_yields_empty_identity() {
        let id = read_account_identity(std::path::Path::new("/nonexistent-xyz"));
        assert!(id.email.is_none() && id.organization.is_none());
    }
}
