//! `defaults` is the layer a profile covers, so it cannot also name it.

use super::*;
use crate::parse::parse_flow_file;

/// The file spells the base layer `defaults:`, so a profile of that
/// name means two things at once — and a host listing the base beside
/// the declared ones would show two rows nobody can tell apart, with
/// different settings behind them.
///
/// Refused whether or not it is the one chosen: the ambiguity is in
/// the file, not in the pick.
#[test]
fn a_profile_cannot_take_the_name_of_the_layer_it_covers() {
    let text = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
profiles:
  defaults:
    timeout: 1m
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
";
    for chosen in [None, Some("defaults")] {
        let issues = resolve(parse_flow_file(text).expect("parses"), chosen)
            .expect_err("a reserved profile name is not a flow");
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::ReservedProfileName)),
            "chosen {chosen:?}: {issues:?}"
        );
    }
}
