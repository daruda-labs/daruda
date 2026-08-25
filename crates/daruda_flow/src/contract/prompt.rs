//! The contract as the agent is told it, in the terms `contract::file`
//! judges it by. Without this the rule is unwritten: a node that does the
//! work and never writes the file is refused for breaking a contract
//! nobody stated.

use crate::parse::SchemaSubset;
use crate::runner::ContractBreach;
use std::path::Path;

/// Says `machine-validated` deliberately: an agent that knows a check runs
/// keeps the contract, and whoever reads the transcript afterwards can see
/// what refused the node.
const HEADER: &str = "OUTPUT CONTRACT (machine-validated):";

/// The block appended to the prompt of a node that owes a file. `output` is
/// the absolute path the scheduler resolved — the agent cannot be told a
/// relative one, because its working directory is not the run directory.
pub(crate) fn block(output: &Path, schema: Option<&SchemaSubset>) -> String {
    let mut block = String::from(HEADER);
    block.push_str(&format!(
        "\nWhen you are done, write your result to {}.",
        output.display()
    ));
    block.push_str("\nThe file must exist and be non-empty; a symlink is refused.");
    // Appended rather than folded into the line above: a node that declares no
    // shape must read exactly as it did before this existed.
    if let Some(schema) = schema {
        block.push_str(
            "\nIts contents must be a single JSON value matching this schema, with no prose \
             and no code fence around it. Properties the schema does not name are ignored:\n",
        );
        block.push_str(&schema_json(schema));
    }
    block
}

/// The schema as the agent is shown it. JSON rather than the file's YAML: it is
/// a JSON Schema, and the value being asked for is JSON.
fn schema_json(schema: &SchemaSubset) -> String {
    // Only a map with non-string keys can fail here, and `SchemaSubset` has
    // none — but a prompt is not worth a panic.
    serde_json::to_string_pretty(schema).unwrap_or_default()
}

/// What the agent is told when its turn ended without meeting the contract,
/// sent on the session that turn ran in.
///
/// **The contract itself is deliberately not repeated.** It is already in
/// this session's context, a few thousand tokens up, and re-pasting it
/// invites the model to rebuild its answer from scratch — dropping fields
/// that were right the first time. What is new is the reasons, carried
/// verbatim from the check so the agent is told the same thing the run will
/// report.
pub(crate) fn correction(breach: &ContractBreach) -> String {
    let mut text =
        String::from("Your previous turn ended without satisfying the OUTPUT CONTRACT above.");
    for reason in std::iter::once(&breach.first).chain(&breach.rest) {
        text.push_str(&format!("\n- {reason}"));
    }
    text.push_str("\nSatisfy the contract now. Change nothing else.");
    text
}

#[cfg(test)]
mod tests;
