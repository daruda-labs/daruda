use super::*;
#[test]
fn missing_providers_fall_back_without_animating_settled_states() {
    let catalog = Catalog::default();
    let missing = daruda_config::StatusIconTheme::Custom("user/missing".into());
    assert_eq!(catalog.base(&missing), Base::Icons);
    assert!(catalog.animates(&missing, State::Working));
    for state in [
        State::Disconnected,
        State::Idle,
        State::Completed,
        State::Errored,
        State::Failed,
        State::NeedsAttention,
    ] {
        assert!(!catalog.animates(&missing, state));
    }
}

#[test]
fn custom_frames_drive_pulses_and_color_only_overrides_inherit_them() {
    let frame = Frame {
        key: "test".into(),
        svg: Arc::from(&b"test"[..]),
    };
    let id = daruda_config::StatusIconTheme::Custom("user/example".into());
    let mut theme = Theme {
        id: id.as_str().into(),
        name: "Example".into(),
        base: Base::Icons,
        states: BTreeMap::new(),
    };
    theme.states.insert(
        State::Working,
        Artwork {
            frames: vec![frame.clone()],
            ticks_per_frame: 1,
            colors: None,
        },
    );
    theme.states.insert(
        State::NeedsAttention,
        Artwork {
            frames: vec![frame.clone(), frame],
            ticks_per_frame: 2,
            colors: None,
        },
    );
    theme.states.insert(
        State::ExecutingTool,
        Artwork {
            frames: vec![],
            ticks_per_frame: 2,
            colors: Some((0, 0)),
        },
    );
    let catalog = Catalog {
        themes: BTreeMap::from([(theme.id.clone(), theme)]),
    };
    assert!(!catalog.animates(&id, State::Working));
    assert!(catalog.animates(&id, State::NeedsAttention));
    assert!(catalog.animates(&id, State::ExecutingTool));
}
