use crate::surface::strings as s;

/// Why a branch name was rejected, for the inline label under the field.
/// One arm per `daruda_core::git::BranchNameRule` so a new rule is a
/// compile error here rather than a silently unlabelled failure.
pub(crate) fn reason(rule: daruda_core::git::BranchNameRule) -> String {
    use daruda_core::git::BranchNameRule as R;
    match rule {
        // `Empty` never reaches here — the form treats a blank field as
        // "derive from the title", not as an error.
        R::Empty => s::create_lane::err_branch_required(),
        R::DoubleDot => s::branch_rule::double_dot(),
        R::EdgeSlash => s::branch_rule::edge_slash(),
        R::EdgeDot => s::branch_rule::edge_dot(),
        R::ControlChar => s::branch_rule::control_char(),
        R::Space => s::branch_rule::space(),
        R::Reserved(ch) => s::branch_rule::reserved(ch),
    }
}
