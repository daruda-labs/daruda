use super::*;

#[test]
fn account_home_override_and_removal_precede_agent_and_ambient_values() {
    let env = vec![("HOME".into(), "agent-home".into())];
    let ambient = Some(PathBuf::from("ambient-home"));
    let account = daruda_config::AccountEnv {
        inject: vec![("HOME".into(), "account-home".into())],
        strip: vec!["HOME"],
    };
    assert_eq!(
        effective_home(&env, Some(&account), ambient.clone()),
        Some(PathBuf::from("account-home"))
    );
    let removed = daruda_config::AccountEnv {
        inject: vec![],
        strip: vec!["HOME"],
    };
    assert_eq!(effective_home(&env, Some(&removed), ambient.clone()), None);
    assert_eq!(
        effective_home(&[("HOME".into(), String::new())], None, ambient),
        Some(PathBuf::new())
    );
}

#[cfg(windows)]
#[test]
fn local_home_lookup_and_removal_accept_windows_environment_spelling() {
    let env = vec![("Home".into(), "agent-home".into())];
    assert_eq!(effective_home(&env, None, None), Some("agent-home".into()));
    let account = daruda_config::AccountEnv {
        inject: vec![],
        strip: vec!["hOmE"],
    };
    assert_eq!(effective_home(&env, Some(&account), None), None);
}

#[test]
fn home_follows_the_launch_environment_even_for_managed_accounts() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("custom");
    let env = vec![("HOME".into(), home.to_string_lossy().into_owned())];
    let ambient = Some(temp.path().join("ambient"));
    for provider in [Provider::Claude, Provider::Codex] {
        let config = Configuration::resolve(
            provider,
            effective_home(&env, None, ambient.clone()),
            None,
            None,
        )
        .unwrap();
        assert_eq!(config.home, home);
    }
    let account = daruda_config::AccountEnv {
        inject: vec![],
        strip: vec![],
    };
    let config = Configuration::resolve(
        Provider::Codex,
        effective_home(&env, Some(&account), ambient.clone()),
        Some(temp.path().join("account")),
        None,
    )
    .unwrap();
    assert_eq!(config.personal_skills, home.join(".agents/skills"));
    assert_eq!(effective_home(&[], None, ambient.clone()), ambient);
    assert!(
        Configuration::resolve(
            Provider::Claude,
            effective_home(&[("HOME".into(), "relative".into())], None, ambient),
            None,
            None
        )
        .is_none()
    );
}
