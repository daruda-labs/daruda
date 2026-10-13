//! Public API tests for accounts recipe.

use daruda_agent::accounts::recipe::*;
use daruda_store::accounts::AccountRecipeId;

#[test]
fn recipe_for_claude_matches_current_constants() {
    let recipe = recipe_for(AccountRecipeId::Claude);
    assert_eq!(recipe.id(), AccountRecipeId::Claude);
    assert_eq!(recipe.config_dir_env(), "CLAUDE_CONFIG_DIR");
    assert_eq!(
        recipe.login_args(),
        ["--cli", "auth", "login", "--claudeai"]
    );
    assert_eq!(recipe.system_home_hint(), "~/.claude");
    let strip = recipe.strip_env();
    for var in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "AWS_BEARER_TOKEN_BEDROCK",
        "ANTHROPIC_CUSTOM_HEADERS",
    ] {
        assert!(strip.contains(&var), "missing strip var: {var}");
    }
    assert_eq!(strip.len(), 5);
}

#[test]
fn recipe_for_codex_matches_current_constants() {
    let recipe = recipe_for(AccountRecipeId::Codex);
    assert_eq!(recipe.id(), AccountRecipeId::Codex);
    assert_eq!(recipe.config_dir_env(), "CODEX_HOME");
    assert_eq!(recipe.login_args(), ["cli", "login"]);
    assert_eq!(recipe.system_home_hint(), "~/.codex");
    assert!(recipe.strip_env().is_empty());
}

#[test]
fn every_recipe_cleans_up_only_dir_scoped_state() {
    // The orphan sweep runs every `DirScoped` cleanup against a dir it
    // can't attribute to a domain, so a cleanup that ever reaches further
    // has to declare `External` here to stay out of that path.
    for id in AccountRecipeId::all() {
        assert_eq!(recipe_for(id).cleanup_scope(), CleanupScope::DirScoped);
    }
}

/// A system login runs against the user's own home rather than a minted
/// config dir, so every domain has to be able to name that home — a domain
/// that cannot has no way to probe whether its login landed.
#[test]
fn every_recipe_can_name_its_system_home() {
    for id in AccountRecipeId::all() {
        let recipe = recipe_for(id);
        let home = recipe
            .system_home_dir()
            .expect("a machine running the tests has a home directory");
        assert!(
            home.is_absolute(),
            "{id:?} yielded a relative home: {home:?}"
        );
    }
}

/// The hint is display copy and the dir is a real path, but they must name
/// the same place — a hint that says `~/.claude` next to a login writing
/// somewhere else is worse than no hint.
#[test]
fn the_system_home_hint_describes_the_system_home_dir() {
    for id in AccountRecipeId::all() {
        let recipe = recipe_for(id);
        // Skip when the user's environment overrides the home — then the
        // hint intentionally still shows the default location.
        if std::env::var_os(recipe.config_dir_env()).is_some() {
            continue;
        }
        let home = recipe.system_home_dir().expect("home directory");
        let hint = recipe.system_home_hint();
        let tail = hint.strip_prefix("~/").expect("hints are tilde paths");
        assert_eq!(
            home.file_name().and_then(std::ffi::OsStr::to_str),
            Some(tail),
            "{id:?}: hint {hint} does not name {home:?}"
        );
    }
}

/// Both domains report how a user signed in, in different shapes — and the
/// shape has to travel with the arguments, since reading JSON out of a
/// prose line (or the reverse) yields no method at all, which is
/// indistinguishable from being signed out.
#[test]
fn each_domain_declares_the_shape_of_its_own_status_output() {
    use daruda_agent::accounts::auth_status::AuthStatusFormat;
    let claude = recipe_for(AccountRecipeId::Claude)
        .status_probe()
        .expect("claude reports status");
    assert_eq!(claude.args, ["--cli", "auth", "status", "--json"]);
    assert_eq!(claude.format, AuthStatusFormat::Json);

    let codex = recipe_for(AccountRecipeId::Codex)
        .status_probe()
        .expect("codex reports status");
    assert_eq!(codex.args, ["cli", "login", "status"]);
    assert_eq!(codex.format, AuthStatusFormat::Prose);
}

use daruda_agent::accounts::login::LoginCompletion;

#[test]
fn claude_completes_a_login_on_process_exit() {
    assert_eq!(
        recipe_for(AccountRecipeId::Claude).login_completion(),
        LoginCompletion::OnExit
    );
}

#[test]
fn codex_completes_a_login_on_credentials_landing() {
    assert!(matches!(
        recipe_for(AccountRecipeId::Codex).login_completion(),
        LoginCompletion::OnCredentials { grace } if grace > std::time::Duration::ZERO
    ));
}
