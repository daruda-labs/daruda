/// The test binary never calls `configure_from_env`, so whatever the
/// environment it inherited says, no session it opens writes a capture.
#[test]
fn an_unconfigured_process_has_no_tap() {
    assert_eq!(daruda_acp::wire_log::configured_base(), None);
}
