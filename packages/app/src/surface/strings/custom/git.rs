/// The subject an empty commit message commits as. Repository content rather
/// than interface copy, so it stays English in every locale — a history mixes
/// contributors and should read one way.
pub(crate) fn default_commit_message(files: &[&str]) -> String {
    match files {
        [one] => format!("Update {one}"),
        many => format!("Update {} files", many.len()),
    }
}
