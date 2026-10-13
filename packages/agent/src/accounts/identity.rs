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
