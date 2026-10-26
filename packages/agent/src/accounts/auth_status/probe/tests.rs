use super::*;

#[test]
fn excessive_output_is_not_retained_as_a_partial_payload() {
    let bytes = vec![b'x'; MAX_CAPTURED_OUTPUT + 1];
    let output = drain(std::io::Cursor::new(bytes)).unwrap();
    assert_eq!(output.recv_timeout(Duration::from_secs(5)).unwrap(), None);
}
