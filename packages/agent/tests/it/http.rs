//! Public API tests for HTTP errors.

use daruda_agent::FetchError;

#[test]
fn fetch_error_display_includes_inner() {
    let e = FetchError::Http("connection refused".to_string());
    assert!(e.to_string().contains("connection refused"));
    let e = FetchError::Parse("missing field".to_string());
    assert!(e.to_string().contains("missing field"));
    let e = FetchError::NoToken;
    assert!(e.to_string().contains("Keychain"));
}
