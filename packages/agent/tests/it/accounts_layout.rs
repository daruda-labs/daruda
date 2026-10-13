//! Tests for the public `daruda_agent::accounts::layout` API.

use daruda_agent::accounts::layout::*;
use std::path::Path;

#[test]
fn scoped_service_name_is_stable_and_suffixed() {
    let a = scoped_keychain_service(Path::new("/tmp/acc/alice"));
    let a2 = scoped_keychain_service(Path::new("/tmp/acc/alice"));
    let b = scoped_keychain_service(Path::new("/tmp/acc/bob"));
    assert_eq!(a, a2, "same dir → same service name");
    assert_ne!(a, b, "different dir → different service name");
    assert!(a.starts_with("Claude Code-credentials-"));
    // First 8 hex chars of the sha256.
    assert_eq!(a.rsplit('-').next().unwrap().len(), 8);
}

#[test]
fn config_dir_is_under_data_dir_and_per_account() {
    let data = Path::new("/data");
    let id1 = daruda_store::accounts::AccountId::new();
    let id2 = daruda_store::accounts::AccountId::new();
    let d1 = account_config_dir(data, id1);
    let d2 = account_config_dir(data, id2);
    assert!(d1.starts_with("/data"));
    assert_ne!(d1, d2);
}
