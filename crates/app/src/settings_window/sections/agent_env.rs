//! The agent catalog row's Environment field: the `KEY=value` text a user
//! types, and the three-state [`daruda_config::AgentDefinition::env`] it
//! stands for.
//!
//! The text form itself is [`crate::agent::mcp`]'s — an MCP server's env is
//! edited as the same one-`KEY=value`-per-line block, so both fields share
//! one parser rather than one each. Only the mapping below is specific to the
//! catalog, and it is the whole reason this file exists: a field the user
//! emptied has to say *which* empty it means.

use crate::agent::mcp::{format_env_lines, parse_env_lines};
use std::collections::BTreeMap;

/// `env` as the field's text — one `KEY=value` per line. Empty for a
/// definition that states no environment *and* for one stating an empty one:
/// the field cannot show the difference, which is why [`stated_env`] resolves
/// it from the preset instead of from the text alone.
pub(in crate::settings_window) fn env_field_text(env: Option<&[(String, String)]>) -> String {
    format_env_lines(&env.unwrap_or_default().iter().cloned().collect())
}

/// The environment a row states, given the text it holds and `base` — the
/// environment its preset ships (`None` for a custom row, or a preset that
/// ships none).
///
/// Empty text *clears* rather than un-states whenever there is something to
/// clear: the row loaded the preset's environment into the field, so an
/// emptied field is the user opting out, and only `Some(vec![])` says that
/// (see [`daruda_config::AgentDefinition::env`]). With nothing to opt out of,
/// it writes no key at all, leaving the row free to pick up a preset default
/// added later.
///
/// `Err` names what the user has to fix first, which the caller turns into the
/// section's inline diagnostic.
pub(in crate::settings_window) fn stated_env(
    text: &str,
    base: Option<&[(String, String)]>,
) -> Result<Option<Vec<(String, String)>>, EnvFieldError> {
    let lines = parse_env_lines(text).map_err(|err| EnvFieldError::MalformedLine(err.line))?;
    if let Some((name, _)) = lines
        .iter()
        .find(|(name, _)| !daruda_config::is_valid_env_name(name))
    {
        return Err(EnvFieldError::UnusableName(name.clone()));
    }
    // Through the same funnel the `[[agents]]` load path uses, so a value
    // typed here and one read from disk are ordered identically — which is
    // what lets the preset diff decide on content. A no-op after the check
    // above; calling it is what keeps the invariant one named rule rather
    // than an incidental property of `parse_env_lines`' `BTreeMap`.
    let parsed = daruda_config::canonical_env(lines);
    if parsed.is_empty() && base.is_none_or(<[(String, String)]>::is_empty) {
        return Ok(None);
    }
    Ok(Some(parsed))
}

/// Why a row's Environment text cannot become a value.
///
/// Two variants because they are two different mistakes with two different
/// fixes, and the inline diagnostic has to say which one: a line that is not
/// `KEY=value` at all, versus a well-formed line whose name daruda cannot put
/// into a launch command.
///
/// The name check lives here rather than in the shared
/// [`parse_env_lines`] because that parser also backs the MCP server editor,
/// whose env lands in a JSON `env` map and never reaches a shell — the
/// charset restriction is this field's, not the text form's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::settings_window) enum EnvFieldError {
    /// The line carries no `=`, or nothing before it.
    MalformedLine(String),
    /// The name is outside `[A-Za-z_][A-Za-z0-9_]*` — see
    /// [`daruda_config::is_valid_env_name`] for why nothing downstream can
    /// carry it, and `daruda_config::assemble_launch_command`'s preconditions
    /// for what a remote launch would otherwise do with it.
    UnusableName(String),
}

/// The environment `base` ships, as the muted line under an overridden field.
/// One line rather than the field's own block, since that is the shape every
/// other inherited-base row on this page takes.
pub(in crate::settings_window) fn env_base_summary(base: &[(String, String)]) -> String {
    base.iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `text` states the same environment as `base` — the comparison
/// behind the field's inherited-base line, made on parsed pairs rather than
/// raw text so `K = v` does not read as an override of `K=v`. Text that does
/// not parse is not the preset's value either, so it reads as an override;
/// saving is what reports why.
pub(in crate::settings_window) fn env_follows_base(text: &str, base: &[(String, String)]) -> bool {
    let parsed: Option<BTreeMap<String, String>> = parse_env_lines(text).ok();
    parsed.is_some_and(|parsed| parsed.into_iter().collect::<Vec<_>>() == base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn an_unstated_and_an_empty_environment_both_render_as_empty_text() {
        assert_eq!(env_field_text(None), "");
        assert_eq!(env_field_text(Some(&[])), "");
    }

    #[test]
    fn a_stated_environment_renders_one_pair_per_line() {
        let env = pairs(&[("A", "1"), ("CODEX_CONFIG", r#"{"a":true}"#)]);
        assert_eq!(env_field_text(Some(&env)), "A=1\nCODEX_CONFIG={\"a\":true}");
    }

    #[test]
    fn text_and_value_round_trip_through_the_field() {
        let env = pairs(&[("A", "1"), ("B", "two words")]);
        let text = env_field_text(Some(&env));
        assert_eq!(stated_env(&text, None), Ok(Some(env)));
    }

    /// The three-state distinction at the field level: the same empty text
    /// means two different things depending on whether the preset ships an
    /// environment to opt out of.
    #[test]
    fn empty_text_clears_a_shipping_preset_and_states_none_otherwise() {
        let base = pairs(&[("CODEX_CONFIG", "{}")]);
        assert_eq!(stated_env("", Some(&base)), Ok(Some(Vec::new())));
        assert_eq!(stated_env("   \n\n  ", Some(&base)), Ok(Some(Vec::new())));

        assert_eq!(stated_env("", None), Ok(None));
        assert_eq!(stated_env("", Some(&[])), Ok(None));
    }

    #[test]
    fn a_malformed_line_is_reported_verbatim() {
        assert_eq!(
            stated_env("CODEX_CONFIG", None),
            Err(EnvFieldError::MalformedLine("CODEX_CONFIG".into()))
        );
        assert_eq!(
            stated_env("A=1\n  =2  ", None),
            Err(EnvFieldError::MalformedLine("=2".into()))
        );
    }

    /// The injection this refuses: `daruda_config::assemble_launch_command`
    /// quotes an env value but emits the *name* bare, so a name carrying `;`
    /// would run as its own command inside the remote `sh -c` script. The
    /// field is one of the two places that has to catch it.
    #[test]
    fn a_name_that_could_break_out_of_the_remote_shell_is_refused() {
        assert_eq!(
            stated_env("K; echo PWNED >&2 ; X=1", None),
            Err(EnvFieldError::UnusableName("K; echo PWNED >&2 ; X".into()))
        );
        for bad in [
            "MY VAR=1", "2FAST=1", "K`pwd`=1", "K$X=1", "K-DASH=1", "K.DOT=1", "K'q=1",
        ] {
            assert!(
                matches!(stated_env(bad, None), Err(EnvFieldError::UnusableName(_))),
                "{bad} must be refused"
            );
        }
        // The charset itself still passes, including a leading underscore and
        // digits after the first character.
        assert!(stated_env("_K1=1\nCODEX_CONFIG={}", None).is_ok());
    }

    /// One bad name refuses the whole field rather than silently dropping
    /// that pair: unlike the config-load path (which cannot fail without
    /// discarding the user's entire file), the editor can say so and let the
    /// user fix it.
    #[test]
    fn one_unusable_name_refuses_the_whole_field() {
        assert_eq!(
            stated_env("GOOD=1\nBAD NAME=2", None),
            Err(EnvFieldError::UnusableName("BAD NAME".into()))
        );
    }

    #[test]
    fn following_the_base_ignores_spacing_but_not_content() {
        let base = pairs(&[("A", "1"), ("B", "2")]);
        assert!(env_follows_base("A=1\nB=2", &base));
        assert!(env_follows_base("  A =  1 \n\nB=2\n", &base));
        // Key order does not matter: both sides are keyed.
        assert!(env_follows_base("B=2\nA=1", &base));
        assert!(!env_follows_base("A=1", &base));
        assert!(!env_follows_base("", &base));
        assert!(!env_follows_base("A=1\nB=2\nC=3", &base));
        // Unparseable text is not the preset's value either.
        assert!(!env_follows_base("nonsense", &base));
    }

    #[test]
    fn a_base_summary_stays_on_one_line() {
        assert_eq!(
            env_base_summary(&pairs(&[("A", "1"), ("B", "2")])),
            "A=1 B=2"
        );
        assert_eq!(env_base_summary(&[]), "");
    }
}
