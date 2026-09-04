use super::*;

const ADAPTER: &str = "npx -y @agentclientprotocol/codex-acp@latest";

fn ssh(target: &str, path: &str) -> LaneSessionHost {
    LaneSessionHost::Ssh {
        target: target.into(),
        session_path: path.into(),
        registry_id: None,
    }
}

#[test]
fn effective_session_host_precedence_cases() {
    let legacy_launch = AgentLaunch::Ssh {
        adapter_command: ADAPTER.into(),
        host: "old-box".into(),
    };
    let answered = ssh("new-box", "/srv/app");
    assert_eq!(
        effective_session_host(
            Some(&answered),
            Some("/legacy/path"),
            &legacy_launch,
            &[],
            &[]
        ),
        Ok(answered)
    );

    assert_eq!(
        effective_session_host(
            Some(&LaneSessionHost::Local),
            Some("/legacy/path"),
            &legacy_launch,
            &[],
            &[]
        ),
        Ok(LaneSessionHost::Local)
    );

    let launch = AgentLaunch::Ssh {
        adapter_command: ADAPTER.into(),
        host: "box".into(),
    };
    assert_eq!(
        effective_session_host(None, Some("/legacy/path"), &launch, &[], &[]),
        Ok(ssh("box", "/legacy/path"))
    );
    let launch = AgentLaunch::Docker {
        adapter_command: ADAPTER.into(),
        container: "dev".into(),
    };
    assert_eq!(
        effective_session_host(None, Some("/legacy/path"), &launch, &[], &[]),
        Ok(LaneSessionHost::Docker {
            container: "dev".into(),
            session_path: "/legacy/path".into(),
            registry_id: None,
        })
    );

    let remote_launch = AgentLaunch::Ssh {
        adapter_command: ADAPTER.into(),
        host: "box".into(),
    };
    let local_launch = AgentLaunch::Raw(ADAPTER.into());
    for (remote_cwd, launch) in [
        (None, &remote_launch),
        (Some("   "), &remote_launch),
        (Some("/path"), &local_launch),
    ] {
        assert_eq!(
            effective_session_host(None, remote_cwd, launch, &[], &[]),
            Ok(LaneSessionHost::Local)
        );
    }
}

/// The legacy fallback pair copies both of its halves verbatim, so a
/// hand-edited `[[agents]].ssh.host` and a hand-edited `remote_cwd` both
/// reach [`wrap`] unvalidated. `ssh` joins its trailing arguments into a
/// *remote* command line, so a host carrying its own flags is the sharp
/// shape. Refused with the offending field — never answered as `Local`,
/// which would run the session on this machine instead of where the lane
/// says.
#[test]
fn a_legacy_pair_that_would_break_the_quoting_is_refused_not_run_locally() {
    for host in [
        "vm -o ProxyCommand=touch /tmp/pwned",
        "vm; echo PWNED >&2 ; true",
    ] {
        let launch = AgentLaunch::Ssh {
            adapter_command: ADAPTER.into(),
            host: host.into(),
        };
        let refused =
            effective_session_host(None, Some("/work"), &launch, &[], &[]).expect_err(host);
        assert_eq!(
            refused.reason,
            SessionHostError::Unsafe(SessionHostField::Target),
            "{host}"
        );
        // The value is handed back so a caller can name it, never `Local`.
        assert_eq!(refused.host, ssh(host, "/work"), "{host}");
    }

    let launch = AgentLaunch::Docker {
        adapter_command: ADAPTER.into(),
        container: "dev --privileged".into(),
    };
    assert_eq!(
        effective_session_host(None, Some("/work"), &launch, &[], &[]).map_err(|u| u.reason),
        Err(SessionHostError::Unsafe(SessionHostField::Container))
    );

    // The lane's half of the pair is just as unvalidated.
    let launch = AgentLaunch::Ssh {
        adapter_command: ADAPTER.into(),
        host: "vm".into(),
    };
    assert_eq!(
        effective_session_host(
            None,
            Some(r#"/work" && touch /tmp/pwned && cd ""#),
            &launch,
            &[],
            &[]
        )
        .map_err(|u| u.reason),
        Err(SessionHostError::Unsafe(SessionHostField::SessionPath))
    );
}

/// The registry catalog is the other unvalidated entrance: a hand-edited
/// `[[session_hosts]]` entry is folded onto a linked lane host by
/// [`apply_catalog_entry`], overwriting the value the form checked.
#[test]
fn a_hand_edited_catalog_entry_cannot_smuggle_in_a_bare_word() {
    let id = SessionHostId::new();
    let cached = LaneSessionHost::Ssh {
        target: "checked-box".into(),
        session_path: "/srv/app".into(),
        registry_id: Some(id),
    };
    let catalog = vec![SessionHostEntry {
        id,
        label: "Hand-edited".into(),
        kind: SessionHostKind::Ssh {
            target: "vm -o ProxyCommand=touch /tmp/pwned".into(),
        },
    }];
    let launch = AgentLaunch::Raw(ADAPTER.into());
    assert_eq!(
        effective_session_host(Some(&cached), None, &launch, &catalog, &[]).map_err(|u| u.reason),
        Err(SessionHostError::Unsafe(SessionHostField::Target))
    );
}

/// Regression anchor: the strings a shipped config already produces must
/// not move. Mirrors `AgentLaunch::wrap`'s two remote arms.
#[test]
fn wrap_matches_the_shipped_assembly() {
    assert_eq!(wrap(&LaneSessionHost::Local, ADAPTER), ADAPTER);
    assert_eq!(
        wrap(&ssh("build-box", "/home/user/project"), ADAPTER),
        format!("ssh build-box sh -c 'cd \"/home/user/project\" && {ADAPTER}'")
    );
    assert_eq!(
        wrap(
            &LaneSessionHost::Docker {
                container: "dev-1".into(),
                session_path: "/workspace".into(),
                registry_id: None,
            },
            ADAPTER
        ),
        format!("docker exec -i dev-1 sh -c 'cd \"/workspace\" && {ADAPTER}'")
    );
}

#[test]
fn adapter_command_reads_through_any_launch_shape() {
    assert_eq!(adapter_command(&AgentLaunch::Raw(ADAPTER.into())), ADAPTER);
    assert_eq!(
        adapter_command(&AgentLaunch::Ssh {
            adapter_command: ADAPTER.into(),
            host: "old-box".into(),
        }),
        ADAPTER
    );
    assert_eq!(
        adapter_command(&AgentLaunch::Docker {
            adapter_command: ADAPTER.into(),
            container: "old-box".into(),
        }),
        ADAPTER
    );
}

/// Mirrors `wrap_with_env_prefixes_raw_command` /
/// `wrap_with_env_raw_emits_no_unset_flags` in `daruda_config`: a `Local`
/// host gets the same inject-only `KEY='value' ` prefix, never an
/// `unset`/`/usr/bin/env` — that would hide the launcher token from
/// `daruda_acp::node::command_needs_node`.
#[test]
fn wrap_with_env_local_matches_the_shipped_assembly() {
    let env = AccountEnv {
        inject: vec![(
            "CLAUDE_CONFIG_DIR".into(),
            "/Users/x/Library/Application Support/daruda/acc/alice".into(),
        )],
        strip: vec!["ANTHROPIC_API_KEY"],
    };
    let cmd = wrap_with_env(&LaneSessionHost::Local, "npx -y some-acp", &env);
    assert_eq!(
        cmd,
        "CLAUDE_CONFIG_DIR='/Users/x/Library/Application Support/daruda/acc/alice' npx -y some-acp"
    );
    assert!(!cmd.contains("unset "));
    assert!(!cmd.contains("/usr/bin/env"));
}

/// The script argument of a built `Ssh`/`Docker` command, recovered the
/// way the launcher gets it: tokenize the **whole** command with POSIX
/// rules and take the word after `-c`. Mirrors `sh_c_script` in
/// `daruda_config`'s agent tests — passing through that outer split is
/// the point, it is the first of the two layers a value must survive.
fn sh_c_script(cmd: &str) -> String {
    let tokens = shell_words::split(cmd).expect("built command tokenizes as one POSIX command");
    let dash_c = tokens
        .iter()
        .position(|t| t == "-c")
        .unwrap_or_else(|| panic!("no `-c` in tokens: {tokens:?}"));
    tokens
        .get(dash_c + 1)
        .unwrap_or_else(|| panic!("`-c` carries no script in tokens: {tokens:?}"))
        .clone()
}

/// The value `name` is exported to, as the remote shell tokenizes the
/// script it received — the second layer, applied to `sh_c_script`.
fn exported_value(cmd: &str, name: &str) -> String {
    let script = sh_c_script(cmd);
    let tokens = shell_words::split(&script).expect("remote shell can tokenize the script");
    let assignment = tokens
        .iter()
        .find(|t| t.starts_with(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no `{name}=...` assignment in tokens: {tokens:?}"));
    let (_, value) = assignment.split_once('=').expect("assignment has a value");
    value.strip_suffix(';').unwrap_or(value).to_string()
}

/// Mirrors `wrap_with_env_ssh_exports_and_unsets` in `daruda_config`.
/// Asserted on the script the remote shell actually receives, not the
/// outer command text — the outer layer quotes the script as a whole.
#[test]
fn wrap_with_env_ssh_matches_the_shipped_assembly() {
    let env = AccountEnv {
        inject: vec![("CLAUDE_CONFIG_DIR".into(), "/remote/acc".into())],
        strip: vec!["ANTHROPIC_API_KEY"],
    };
    let cmd = wrap_with_env(&ssh("vm", "/work"), "npx -y some-acp", &env);
    let script = sh_c_script(&cmd);
    assert!(
        script.contains("export CLAUDE_CONFIG_DIR='/remote/acc'"),
        "{script}"
    );
    assert!(script.contains("unset ANTHROPIC_API_KEY"), "{script}");
}

/// The `Local` prefix is re-tokenized downstream by
/// `daruda_acp::node::split_env_prefixed_tokens` / `AcpAgent::from_str`,
/// so a value holding a double quote has to come back out as one token.
#[test]
fn wrap_with_env_local_preserves_a_hostile_value_as_one_token() {
    let value = r#"{"features":{"multi_agent_v2":true}} it's $HOME `pwd` \ "q""#;
    let env = AccountEnv {
        inject: vec![("CODEX_CONFIG".into(), value.to_string())],
        strip: vec![],
    };
    let cmd = wrap_with_env(&LaneSessionHost::Local, ADAPTER, &env);
    let tokens = shell_words::split(&cmd).expect("Local prefix stays one shell-parseable command");
    let assignment = tokens.first().expect("env prefix token present");
    let (name, got) = assignment.split_once('=').expect("assignment has a value");
    assert_eq!(name, "CODEX_CONFIG");
    assert_eq!(got, value);
}

#[test]
fn wrap_with_env_remote_preserves_a_hostile_value_intact() {
    let value = r#"{"features":{"multi_agent_v2":true}} it's $HOME `pwd` \ "q""#;
    let env = AccountEnv {
        inject: vec![("CODEX_CONFIG".into(), value.to_string())],
        strip: vec![],
    };
    assert_eq!(
        exported_value(
            &wrap_with_env(&ssh("vm", "/work"), ADAPTER, &env),
            "CODEX_CONFIG"
        ),
        value
    );
    let docker = LaneSessionHost::Docker {
        container: "dev-1".into(),
        session_path: "/work".into(),
        registry_id: None,
    };
    assert_eq!(
        exported_value(&wrap_with_env(&docker, ADAPTER, &env), "CODEX_CONFIG"),
        value
    );
}

/// Both layers for real: POSIX-tokenize the built command, then hand the
/// `sh -c` argv to an actual shell that prints the variable back. A
/// modelled tokenizer can agree with a wrong implementation; a live shell
/// cannot. Mirrors `wrap_with_env_remote_value_survives_a_real_shell` in
/// `daruda_config`, on the path `resolve_session_command` actually takes.
///
/// Both remote hosts: they share the assembler today, but that is an
/// implementation detail a test should not assume — and each carries a
/// different transport prefix this has to skip past.
#[cfg(unix)]
#[test]
fn wrap_with_env_remote_value_survives_a_real_shell() {
    let value = r#"{"features":{"multi_agent_v2":true}} it's $HOME `pwd` \ "q""#;
    let env = AccountEnv {
        inject: vec![("CODEX_CONFIG".into(), value.to_string())],
        strip: vec![],
    };
    let hosts = [
        ssh("vm", "/tmp"),
        LaneSessionHost::Docker {
            container: "dev-1".into(),
            session_path: "/tmp".into(),
            registry_id: None,
        },
    ];
    for host in hosts {
        let cmd = wrap_with_env(&host, "printf %s \"$CODEX_CONFIG\"", &env);

        // Drop the `ssh <target>` / `docker exec -i <container>`
        // transport prefix and run the rest — the argv the remote shell
        // would have been exec'd with.
        let tokens =
            shell_words::split(&cmd).expect("built command tokenizes as one POSIX command");
        let sh = tokens
            .iter()
            .position(|t| t == "sh")
            .unwrap_or_else(|| panic!("no `sh` in tokens: {tokens:?}"));
        let out = std::process::Command::new(&tokens[sh])
            .args(&tokens[sh + 1..])
            .output()
            .expect("a POSIX shell runs");
        assert!(
            out.status.success(),
            "shell failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&out.stdout), value, "{cmd}");
    }
}

#[test]
fn sanitizing_trims_every_field() {
    assert_eq!(
        sanitized_ssh("  build-box \n", "\t/srv/app  ", None),
        Ok(ssh("build-box", "/srv/app"))
    );
}

#[test]
fn an_empty_field_is_rejected() {
    assert_eq!(
        sanitized_ssh("   ", "/srv/app", None),
        Err(SessionHostError::Empty(SessionHostField::Target))
    );
    assert_eq!(
        sanitized_ssh("box", "", None),
        Err(SessionHostError::Empty(SessionHostField::SessionPath))
    );
    assert_eq!(
        sanitized_docker("  ", "/srv", None),
        Err(SessionHostError::Empty(SessionHostField::Container))
    );
}

/// A bare word is unquoted in `wrap`'s output, so anything that could split
/// it or mean something to the shell is out — a space included.
#[test]
fn a_bare_word_rejects_anything_outside_a_shell_word() {
    for bad in [
        "box; rm -rf /",
        "box host",
        "box'x",
        "box\"x",
        "box`x`",
        "box$x",
        "box\\x",
        "box&x",
        "box|x",
        "box\nhost",
    ] {
        assert_eq!(
            sanitized_ssh(bad, "/srv", None),
            Err(SessionHostError::Unsafe(SessionHostField::Target)),
            "target {bad:?} must be rejected"
        );
        assert_eq!(
            sanitized_docker(bad, "/srv", None),
            Err(SessionHostError::Unsafe(SessionHostField::Container)),
            "container {bad:?} must be rejected"
        );
    }
}

#[test]
fn a_bare_word_accepts_the_shapes_a_host_really_takes() {
    for good in [
        "buildbox",
        "user@host",
        "host.example.com",
        "10.0.0.7",
        "[fe80::1%en0]",
        "my-alias_2",
        "host:2222",
    ] {
        assert!(
            sanitized_ssh(good, "/srv", None).is_ok(),
            "target {good:?} must be accepted"
        );
    }
}

/// The path sits inside `"…"` inside `'…'`, so only what escapes those is
/// out — a space or a semicolon stays inert and must be allowed.
#[test]
fn a_session_path_rejects_only_what_escapes_the_quoting() {
    for bad in [
        "/srv/a'b",
        "/srv/a\"b",
        "/srv/a`b`",
        "/srv/$HOME",
        "/srv/a\\b",
        "/srv/a\nb",
        "/srv/a\rb",
    ] {
        assert_eq!(
            sanitized_ssh("box", bad, None),
            Err(SessionHostError::Unsafe(SessionHostField::SessionPath)),
            "path {bad:?} must be rejected"
        );
    }
    for good in ["/srv/my project", "/srv/a;b", "/srv/a(b)", "/srv/a&b"] {
        assert!(
            sanitized_ssh("box", good, None).is_ok(),
            "path {good:?} must be accepted"
        );
    }
}

/// Regression: `~/work` used to be accepted, but `wrap` always emits
/// `cd "…"` — double quotes suppress POSIX tilde expansion, so a saved
/// `~/work` connected and then failed with "no such file or directory"
/// on every real remote host.
#[test]
fn a_session_path_starting_with_tilde_is_rejected() {
    for bad in ["~/work", "~", "~root/x"] {
        assert_eq!(
            sanitized_ssh("box", bad, None),
            Err(SessionHostError::Unsafe(SessionHostField::SessionPath)),
            "path {bad:?} must be rejected"
        );
    }
    // A `~` that isn't the leading character is inert — the shell only
    // ever expands a *leading* tilde.
    assert!(sanitized_ssh("box", "/srv/a~b", None).is_ok());
}

/// The link is what makes a picked registry entry resolvable later, so
/// building from an entry must stamp its id — no caller-side patch step.
#[test]
fn building_from_a_registry_entry_carries_its_id_and_value() {
    let id = SessionHostId::new();
    assert_eq!(
        from_registry_entry(
            &SessionHostEntry {
                id,
                label: "Build box".into(),
                kind: SessionHostKind::Ssh {
                    target: " vm-work ".into(),
                },
            },
            "/srv/app",
        ),
        Ok(LaneSessionHost::Ssh {
            target: "vm-work".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(id),
        })
    );
    assert_eq!(
        from_registry_entry(
            &SessionHostEntry {
                id,
                label: "Dev container".into(),
                kind: SessionHostKind::Docker {
                    container: "dev-1".into(),
                },
            },
            "/workspace",
        ),
        Ok(LaneSessionHost::Docker {
            container: "dev-1".into(),
            session_path: "/workspace".into(),
            registry_id: Some(id),
        })
    );
}

/// The entry's value goes through the same quoting check free-text input
/// did — a hand-edited `config.toml` is the reachable source here.
#[test]
fn building_from_a_registry_entry_still_rejects_an_unsafe_value() {
    assert_eq!(
        from_registry_entry(
            &SessionHostEntry {
                id: SessionHostId::new(),
                label: "Bad".into(),
                kind: SessionHostKind::Ssh {
                    target: "box; rm -rf /".into(),
                },
            },
            "/srv/app",
        ),
        Err(SessionHostError::Unsafe(SessionHostField::Target))
    );
}

fn tombstone(old_id: SessionHostId, redirected_to: Option<SessionHostId>) -> SessionHostTombstone {
    SessionHostTombstone {
        old_id,
        kind: SessionHostKind::Ssh {
            target: "irrelevant".into(),
        },
        value: "irrelevant".into(),
        removed_at: 0,
        redirected_to,
    }
}

/// Regression anchor: a `registry_id: None` host must resolve
/// byte-for-byte identically whether the catalog/tombstones are empty or
/// (as here) non-empty but unrelated — the vast majority of real lanes
/// never touch the registry at all.
#[test]
fn none_registry_id_ignores_the_catalog_entirely() {
    let host = ssh("box", "/srv/app");
    let unrelated_id = SessionHostId::new();
    let catalog = vec![SessionHostEntry {
        id: unrelated_id,
        label: "Something else".into(),
        kind: SessionHostKind::Ssh {
            target: "other-box".into(),
        },
    }];
    let tombstones = vec![tombstone(unrelated_id, None)];
    let launch = AgentLaunch::Raw(ADAPTER.into());
    assert_eq!(
        effective_session_host(Some(&host), None, &launch, &catalog, &tombstones),
        Ok(host)
    );
}

#[test]
fn a_linked_host_resolves_to_the_catalogs_latest_value() {
    let id = SessionHostId::new();
    let cached = LaneSessionHost::Ssh {
        target: "old-target".into(),
        session_path: "/srv/app".into(),
        registry_id: Some(id),
    };
    let catalog = vec![SessionHostEntry {
        id,
        label: "Build box".into(),
        kind: SessionHostKind::Ssh {
            target: "new-target".into(),
        },
    }];
    let launch = AgentLaunch::Raw(ADAPTER.into());
    let resolved = effective_session_host(Some(&cached), None, &launch, &catalog, &[])
        .expect("a form-checked host stays usable");
    assert_eq!(
        resolved,
        LaneSessionHost::Ssh {
            target: "new-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(id),
        }
    );
}

/// Matching is by id, never by label — a rename in the registry entry
/// must not orphan every lane that references it.
#[test]
fn a_relabeled_entry_still_resolves_by_id() {
    let id = SessionHostId::new();
    let cached = LaneSessionHost::Docker {
        container: "old-name".into(),
        session_path: "/workspace".into(),
        registry_id: Some(id),
    };
    let catalog = vec![SessionHostEntry {
        id,
        label: "Renamed label".into(),
        kind: SessionHostKind::Docker {
            container: "dev-2".into(),
        },
    }];
    let launch = AgentLaunch::Raw(ADAPTER.into());
    let resolved = effective_session_host(Some(&cached), None, &launch, &catalog, &[])
        .expect("a form-checked host stays usable");
    assert_eq!(
        resolved,
        LaneSessionHost::Docker {
            container: "dev-2".into(),
            session_path: "/workspace".into(),
            registry_id: Some(id),
        }
    );
}

#[test]
fn an_unresolvable_registry_id_keeps_the_cached_value() {
    let id = SessionHostId::new();
    let cached = LaneSessionHost::Ssh {
        target: "cached-target".into(),
        session_path: "/srv/app".into(),
        registry_id: Some(id),
    };
    let launch = AgentLaunch::Raw(ADAPTER.into());
    let resolved = effective_session_host(Some(&cached), None, &launch, &[], &[])
        .expect("a form-checked host stays usable");
    assert_eq!(resolved, cached);
    assert_eq!(registry_link_status(&cached, &[]), LinkStatus::Orphaned);
}

/// A hand-edited `config.toml` could give an `Ssh` lane a `registry_id`
/// that now belongs to a `Docker` entry — must be treated exactly like
/// "not found", never coerced across kinds.
#[test]
fn a_kind_mismatched_entry_is_treated_as_not_found() {
    let id = SessionHostId::new();
    let cached = LaneSessionHost::Ssh {
        target: "cached-target".into(),
        session_path: "/srv/app".into(),
        registry_id: Some(id),
    };
    let catalog = vec![SessionHostEntry {
        id,
        label: "Mismatched".into(),
        kind: SessionHostKind::Docker {
            container: "dev-1".into(),
        },
    }];
    let launch = AgentLaunch::Raw(ADAPTER.into());
    let resolved = effective_session_host(Some(&cached), None, &launch, &catalog, &[])
        .expect("a form-checked host stays usable");
    assert_eq!(resolved, cached);
    assert_eq!(
        registry_link_status(&cached, &catalog),
        LinkStatus::Orphaned
    );
}

#[test]
fn a_deleted_entry_resolves_through_a_single_tombstone_redirect() {
    let old_id = SessionHostId::new();
    let new_id = SessionHostId::new();
    let cached = LaneSessionHost::Ssh {
        target: "cached-target".into(),
        session_path: "/srv/app".into(),
        registry_id: Some(old_id),
    };
    let catalog = vec![SessionHostEntry {
        id: new_id,
        label: "Renamed box".into(),
        kind: SessionHostKind::Ssh {
            target: "merged-target".into(),
        },
    }];
    let tombstones = vec![tombstone(old_id, Some(new_id))];
    let launch = AgentLaunch::Raw(ADAPTER.into());
    let resolved = effective_session_host(Some(&cached), None, &launch, &catalog, &tombstones)
        .expect("a form-checked host stays usable");
    assert_eq!(
        resolved,
        LaneSessionHost::Ssh {
            target: "merged-target".into(),
            session_path: "/srv/app".into(),
            // Unchanged: rewriting the cached `registry_id` to the
            // redirected-to id is the write-back Task 3 owns, not this
            // read-only resolver.
            registry_id: Some(old_id),
        }
    );
}

/// The write-back half `resolved_registry_id` owns: unlike
/// `effective_session_host`'s returned value above (which keeps
/// `old_id`), this reports the *live* id so a caller can correct the
/// lane's cache to it.
#[test]
fn resolved_registry_id_cases() {
    let old_id = SessionHostId::new();
    let new_id = SessionHostId::new();
    let cached = LaneSessionHost::Ssh {
        target: "cached-target".into(),
        session_path: "/srv/app".into(),
        registry_id: Some(old_id),
    };
    let catalog = vec![SessionHostEntry {
        id: new_id,
        label: "Renamed box".into(),
        kind: SessionHostKind::Ssh {
            target: "merged-target".into(),
        },
    }];
    let tombstones = vec![tombstone(old_id, Some(new_id))];
    assert_eq!(
        resolved_registry_id(&cached, &catalog, &tombstones),
        Some(new_id)
    );

    let id = SessionHostId::new();
    let cached = LaneSessionHost::Ssh {
        target: "cached-target".into(),
        session_path: "/srv/app".into(),
        registry_id: Some(id),
    };
    let catalog = vec![SessionHostEntry {
        id,
        label: "Box".into(),
        kind: SessionHostKind::Ssh {
            target: "latest-target".into(),
        },
    }];
    assert_eq!(resolved_registry_id(&cached, &catalog, &[]), Some(id));

    let id = SessionHostId::new();
    let cached = LaneSessionHost::Ssh {
        target: "cached-target".into(),
        session_path: "/srv/app".into(),
        registry_id: Some(id),
    };
    assert_eq!(resolved_registry_id(&cached, &[], &[]), None);

    let free_text = LaneSessionHost::Ssh {
        target: "vm-work".into(),
        session_path: "/srv/app".into(),
        registry_id: None,
    };
    assert_eq!(resolved_registry_id(&free_text, &[], &[]), None);
    assert_eq!(
        resolved_registry_id(&LaneSessionHost::Local, &[], &[]),
        None
    );
}

#[test]
fn a_chain_of_tombstone_redirects_resolves_to_the_final_catalog_entry() {
    let id_a = SessionHostId::new();
    let id_b = SessionHostId::new();
    let id_c = SessionHostId::new();
    let cached = LaneSessionHost::Docker {
        container: "cached".into(),
        session_path: "/workspace".into(),
        registry_id: Some(id_a),
    };
    let catalog = vec![SessionHostEntry {
        id: id_c,
        label: "Final".into(),
        kind: SessionHostKind::Docker {
            container: "final-container".into(),
        },
    }];
    let tombstones = vec![tombstone(id_a, Some(id_b)), tombstone(id_b, Some(id_c))];
    let launch = AgentLaunch::Raw(ADAPTER.into());
    let resolved = effective_session_host(Some(&cached), None, &launch, &catalog, &tombstones)
        .expect("a form-checked host stays usable");
    assert_eq!(
        resolved,
        LaneSessionHost::Docker {
            container: "final-container".into(),
            session_path: "/workspace".into(),
            registry_id: Some(id_a),
        }
    );
}

/// A tombstone with no `redirected_to` is a dead end, not a hop — the
/// removal wasn't a merge, so there's nowhere further to look.
#[test]
fn a_tombstone_with_no_redirect_leaves_the_id_orphaned() {
    let id = SessionHostId::new();
    let cached = LaneSessionHost::Docker {
        container: "cached".into(),
        session_path: "/workspace".into(),
        registry_id: Some(id),
    };
    let tombstones = vec![tombstone(id, None)];
    let launch = AgentLaunch::Raw(ADAPTER.into());
    let resolved = effective_session_host(Some(&cached), None, &launch, &[], &tombstones)
        .expect("a form-checked host stays usable");
    assert_eq!(resolved, cached);
}

/// Exactly the hop budget: the 10th redirect lands on the catalog entry
/// and must still resolve.
#[test]
fn a_redirect_chain_of_exactly_ten_hops_still_resolves() {
    let ids: Vec<SessionHostId> = (0..11).map(|_| SessionHostId::new()).collect();
    let cached = LaneSessionHost::Ssh {
        target: "cached-target".into(),
        session_path: "/srv/app".into(),
        registry_id: Some(ids[0]),
    };
    let catalog = vec![SessionHostEntry {
        id: ids[10],
        label: "Reachable".into(),
        kind: SessionHostKind::Ssh {
            target: "just-in-time".into(),
        },
    }];
    let tombstones: Vec<SessionHostTombstone> = ids
        .windows(2)
        .map(|pair| tombstone(pair[0], Some(pair[1])))
        .collect();
    let launch = AgentLaunch::Raw(ADAPTER.into());
    let resolved = effective_session_host(Some(&cached), None, &launch, &catalog, &tombstones)
        .expect("a form-checked host stays usable");
    assert_eq!(
        resolved,
        LaneSessionHost::Ssh {
            target: "just-in-time".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(ids[0]),
        }
    );
}

/// One hop past the budget: the same shape as the exactly-ten-hops case,
/// but the catalog entry sits one redirect further out — must fall back
/// to the cached value rather than chase indefinitely.
#[test]
fn a_redirect_chain_longer_than_ten_hops_falls_back_to_the_cached_value() {
    let ids: Vec<SessionHostId> = (0..12).map(|_| SessionHostId::new()).collect();
    let cached = LaneSessionHost::Ssh {
        target: "cached-target".into(),
        session_path: "/srv/app".into(),
        registry_id: Some(ids[0]),
    };
    let catalog = vec![SessionHostEntry {
        id: ids[11],
        label: "Unreachable".into(),
        kind: SessionHostKind::Ssh {
            target: "too-far".into(),
        },
    }];
    let tombstones: Vec<SessionHostTombstone> = ids
        .windows(2)
        .map(|pair| tombstone(pair[0], Some(pair[1])))
        .collect();
    let launch = AgentLaunch::Raw(ADAPTER.into());
    let resolved = effective_session_host(Some(&cached), None, &launch, &catalog, &tombstones)
        .expect("a form-checked host stays usable");
    assert_eq!(resolved, cached);
}

/// A cyclic tombstone chain (corrupted/hand-edited config) must not hang
/// the resolver — the hop budget bounds it and the cached value survives.
#[test]
fn a_cyclic_tombstone_chain_terminates_and_falls_back() {
    let id_a = SessionHostId::new();
    let id_b = SessionHostId::new();
    let cached = LaneSessionHost::Ssh {
        target: "cached-target".into(),
        session_path: "/srv/app".into(),
        registry_id: Some(id_a),
    };
    let tombstones = vec![tombstone(id_a, Some(id_b)), tombstone(id_b, Some(id_a))];
    let launch = AgentLaunch::Raw(ADAPTER.into());
    let resolved = effective_session_host(Some(&cached), None, &launch, &[], &tombstones)
        .expect("a form-checked host stays usable");
    assert_eq!(resolved, cached);
}

#[test]
fn registry_link_status_covers_all_three_cases() {
    assert_eq!(
        registry_link_status(&LaneSessionHost::Local, &[]),
        LinkStatus::Unlinked
    );
    assert_eq!(
        registry_link_status(&ssh("box", "/srv"), &[]),
        LinkStatus::Unlinked
    );

    let id = SessionHostId::new();
    let linked = LaneSessionHost::Ssh {
        target: "box".into(),
        session_path: "/srv".into(),
        registry_id: Some(id),
    };
    assert_eq!(registry_link_status(&linked, &[]), LinkStatus::Orphaned);

    let catalog = vec![SessionHostEntry {
        id,
        label: "Build box".into(),
        kind: SessionHostKind::Ssh {
            target: "box".into(),
        },
    }];
    assert_eq!(registry_link_status(&linked, &catalog), LinkStatus::Fresh);
}
