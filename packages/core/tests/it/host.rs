//! Public API tests for host.

use daruda_core::host::*;

#[test]
fn this_machine_has_a_name() {
    let name = name().expect("the OS names this machine");
    assert!(!name.contains('\0'));
}
