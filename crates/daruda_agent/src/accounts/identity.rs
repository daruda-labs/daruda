//! Reads account identity (email/org) from `<config_dir>/.claude.json`'s
//! `oauthAccount` — the authoritative on-disk source (the credentials blob
//! has no email). Best-effort: any failure yields an empty identity.

use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountIdentity {
    pub email: Option<String>,
    pub organization: Option<String>,
}

pub fn read_account_identity(config_dir: &Path) -> AccountIdentity {
    let raw = match std::fs::read_to_string(config_dir.join(".claude.json")) {
        Ok(s) => s,
        Err(_) => return AccountIdentity::default(),
    };
    let v: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return AccountIdentity::default(),
    };
    let oauth = &v["oauthAccount"];
    AccountIdentity {
        email: oauth["emailAddress"].as_str().map(str::to_string),
        organization: oauth["organizationName"].as_str().map(str::to_string),
    }
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
    #[test]
    fn missing_file_yields_empty_identity() {
        let id = read_account_identity(std::path::Path::new("/nonexistent-xyz"));
        assert!(id.email.is_none() && id.organization.is_none());
    }
}
