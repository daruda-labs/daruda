//! Public API tests for update.

use daruda_config::update::*;

#[test]
fn default_auto_check_is_true() {
    assert!(UpdateConfig::default().auto_check);
}
