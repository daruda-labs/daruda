use super::gauges::{chart_heights, gauge_value_text};
use super::*;
use daruda_agent::activity::DayActivity;

#[gpui::test]
fn folded_usage_sections_have_equal_spacing(cx: &mut gpui::TestAppContext) {
    use gpui::{Render, VisualTestContext, Window};

    struct Probe {
        snap: RightDockSnapshot,
    }
    impl Render for Probe {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let activity = ActivityStats {
                daily: vec![DayActivity {
                    date: "2026-10-13".into(),
                    turns: 1,
                    tokens: 100,
                }],
                recent_sessions: Vec::new(),
            };
            super::super::right_panel_body()
                .w(px(240.))
                .child(provider_section(
                    &section(AccountRecipeId::Claude),
                    Some(&activity),
                    &self.snap,
                    cx,
                ))
                .child(recent_sessions_block(&[], false, &self.snap.workspace, cx))
        }
    }

    let (_, workspace) = crate::workspace::tests::build_workspace(cx);
    let snap = workspace.update(cx, |ws, cx| {
        let mut snap = ws.prepare_right_dock_snapshot(cx);
        for section in [DockSection::UsageTurns, DockSection::UsageTokens] {
            if snap.sections.is_open(section) {
                snap.sections.toggle(section);
            }
        }
        snap
    });
    let window = cx.add_window(|_, _| Probe { snap });
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let turns = vcx.debug_bounds("dock-section-UsageTurns").unwrap();
    let tokens = vcx.debug_bounds("dock-section-UsageTokens").unwrap();
    let recent = vcx
        .debug_bounds("dock-section-UsageRecentSessions")
        .unwrap();
    assert_eq!(turns.size.height, tokens.size.height);
    assert_eq!(tokens.size.height, recent.size.height);
    assert_eq!(
        tokens.top() - turns.bottom(),
        recent.top() - tokens.bottom()
    );
}

#[gpui::test]
fn restore_action_fits_the_session_row(cx: &mut gpui::TestAppContext) {
    crate::workspace::right_dock::row_tests::assert_hover_targets_fit(
        cx,
        &["usage-session-restore"],
        &[],
        |workspace, cx| {
            let session = RestorableSession {
                session_id: "sample-session".into(),
                agent_id: "claude-acp".into(),
                account: daruda_store::accounts::AccountSelection::SystemDefault,
                lane_ref: workspace.upgrade().unwrap().read(cx).active,
                title: Some("A previous session".into()),
                prompt_preview: None,
                git_branch: None,
                cwd: std::path::PathBuf::from("sample-project"),
                last_active: SystemTime::UNIX_EPOCH,
            };
            recent_session_row(&session, &workspace, cx).into_any_element()
        },
    );
}
use daruda_agent::UsageOutcome;

fn section(recipe: AccountRecipeId) -> UsageSectionSnapshot {
    UsageSectionSnapshot {
        recipe,
        account_label: "System".into(),
        outcome: UsageOutcome::Pending,
        service_status: None,
    }
}

#[test]
fn resolve_displayed_domain_cases() {
    let sections = [section(AccountRecipeId::Codex)];
    assert_eq!(
        resolve_displayed_domain(
            AccountDomain::Exactly(AccountRecipeId::Claude),
            Some(AccountRecipeId::Codex),
            &sections,
        ),
        Some(AccountRecipeId::Claude),
        "auto mode always wins, even when it names a domain absent from `sections`"
    );

    let sections = [
        section(AccountRecipeId::Claude),
        section(AccountRecipeId::Codex),
    ];
    assert_eq!(
        resolve_displayed_domain(AccountDomain::Any, Some(AccountRecipeId::Codex), &sections),
        Some(AccountRecipeId::Codex)
    );

    // The override names a domain that has since signed out (absent from
    // `sections`) — it must not be tracked/cleared separately, just
    // ignored in favor of the default.
    let sections = [section(AccountRecipeId::Claude)];
    assert_eq!(
        resolve_displayed_domain(AccountDomain::Any, Some(AccountRecipeId::Codex), &sections),
        Some(AccountRecipeId::Claude)
    );

    let sections = [section(AccountRecipeId::Codex)];
    assert_eq!(
        resolve_displayed_domain(AccountDomain::Unsupported, None, &sections),
        Some(AccountRecipeId::Codex)
    );

    assert_eq!(
        resolve_displayed_domain(AccountDomain::Any, None, &[]),
        None
    );
}

fn restorable_session(
    title: Option<&str>,
    cwd: &str,
    last_active: SystemTime,
) -> RestorableSession {
    RestorableSession {
        session_id: "s1".to_string(),
        agent_id: "claude".to_string(),
        account: daruda_store::accounts::AccountSelection::SystemDefault,
        lane_ref: daruda_store::project::LaneRef::default(),
        title: title.map(SharedString::from),
        prompt_preview: None,
        git_branch: None,
        cwd: std::path::PathBuf::from(cwd),
        last_active,
    }
}

#[test]
fn session_row_label_fallback_cases() {
    let mut session = restorable_session(Some("Fix the bug"), "/Users/x/proj", SystemTime::now());
    session.prompt_preview = Some(SharedString::from("Raw prompt"));
    assert_eq!(session_row_label(&session).as_ref(), "Fix the bug");

    let mut session = restorable_session(None, "/Users/x/proj", SystemTime::now());
    session.prompt_preview = Some(SharedString::from("Improve recent sessions"));
    assert_eq!(
        session_row_label(&session).as_ref(),
        "Improve recent sessions"
    );

    let session = restorable_session(None, "/Users/x/proj", SystemTime::now());
    assert_eq!(session_row_label(&session).as_ref(), "proj");

    let session = restorable_session(None, "/", SystemTime::now());
    assert_eq!(session_row_label(&session).as_ref(), "/");
}

#[test]
fn session_row_meta_label_includes_branch_and_one_relative_time() {
    let last_active = std::time::UNIX_EPOCH + Duration::from_secs(3_600);
    let mut session = restorable_session(None, "/Users/x/proj", last_active);
    session.git_branch = Some(SharedString::from("main"));
    assert_eq!(
        session_row_meta_label(&session).as_ref(),
        format!("main · {}", relative_time_label(last_active))
    );
}

#[test]
fn relative_time_label_buckets_like_the_refresh_badge_without_the_refresh_glyph() {
    let now = SystemTime::now();
    assert_eq!(relative_time_label(now), strings::usage::session_just_now());
    assert!(
        !relative_time_label(now).starts_with('\u{21bb}'),
        "recent-session timestamps must not show the refresh glyph"
    );
    assert_eq!(
        relative_time_label(now - Duration::from_secs(5 * 60)),
        strings::usage::session_minutes(5)
    );
    assert_eq!(
        relative_time_label(now - Duration::from_secs(3 * 3_600)),
        strings::usage::session_hours(3)
    );
    assert_eq!(
        relative_time_label(now - Duration::from_secs(2 * 86_400)),
        strings::usage::session_days(2)
    );
}

#[test]
fn refresh_badge_marks_stale_cached_values() {
    let now = SystemTime::now();
    let fresh = refresh_badge_label(Some(now), false, false);
    let stale = refresh_badge_label(Some(now), true, false);
    assert!(!fresh.contains(&strings::usage::stale_marker()));
    assert!(stale.contains(&strings::usage::stale_marker()));
    assert_eq!(
        refresh_badge_label(Some(now), true, true),
        strings::usage::refreshing()
    );
}

#[test]
fn gauge_value_marks_stale_percentages_only() {
    assert_eq!(gauge_value_text(Some(99.0), false), "99%");
    assert_eq!(
        gauge_value_text(Some(99.0), true),
        format!("99% {}", strings::usage::stale_marker())
    );
    assert_eq!(
        gauge_value_text(None, true),
        strings::usage::limit_unavailable()
    );
}

#[test]
fn show_domain_switcher_cases() {
    assert!(!show_domain_switcher(
        AccountDomain::Exactly(AccountRecipeId::Claude),
        2
    ));
    assert!(!show_domain_switcher(AccountDomain::Any, 1));

    assert!(show_domain_switcher(AccountDomain::Any, 2));
    assert!(show_domain_switcher(AccountDomain::Unsupported, 2));
}

#[test]
fn chart_heights_cases() {
    let h = chart_heights(&[10, 20, 0, 5], 44.0, 3.0);
    assert_eq!(h, vec![22.0, 44.0, 3.0, 11.0]);

    let h = chart_heights(&[0, 0, 0], 44.0, 3.0);
    assert_eq!(h, vec![3.0, 3.0, 3.0]);

    assert!(chart_heights(&[], 44.0, 3.0).is_empty());
}

fn plan(sub: Option<&str>, tier: Option<&str>) -> PlanInfo {
    PlanInfo {
        tier: sub.map(str::to_string),
        qualifier: tier.map(str::to_string),
    }
}

#[test]
fn plan_badge_label_maps_tier() {
    assert_eq!(
        plan_badge_label(Some(&plan(Some("team"), Some("default_claude_ai_5x")))),
        Some("TEAM 5x".to_string())
    );
    assert_eq!(
        plan_badge_label(Some(&plan(Some("max"), Some("claude_20x")))),
        Some("MAX 20x".to_string())
    );
    // Multiplier is suppressed for non-max/team tiers.
    assert_eq!(
        plan_badge_label(Some(&plan(Some("pro"), Some("claude_5x")))),
        Some("PRO".to_string())
    );
    // team with no tier → no multiplier.
    assert_eq!(
        plan_badge_label(Some(&plan(Some("team"), None))),
        Some("TEAM".to_string())
    );
    // Unknown subscription falls back to uppercase.
    assert_eq!(
        plan_badge_label(Some(&plan(Some("scale"), None))),
        Some("SCALE".to_string())
    );
    // Empty subscription → generic brand.
    assert_eq!(
        plan_badge_label(Some(&plan(None, None))),
        Some("CLAUDE".to_string())
    );
    // No plan metadata → no badge.
    assert_eq!(plan_badge_label(None), None);
}

#[test]
fn cache_age_bucket_maps_missing_age_to_never_and_whole_seconds_to_core() {
    assert_eq!(cache_age_bucket(None), None);
    assert_eq!(
        cache_age_bucket(Some(Duration::from_millis(59_999))),
        Some(AgeBucket::JustNow)
    );
    assert_eq!(
        cache_age_bucket(Some(Duration::from_secs(3_600))),
        Some(AgeBucket::Hours(1))
    );
}

#[gpui::test]
fn severity_color_matches_uebersicht_palette(cx: &mut gpui::TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    cx.update(|_cx| {
        assert_eq!(severity_color(LimitSeverity::Low), theme::SIGNAL_GREEN);
        assert_eq!(severity_color(LimitSeverity::Medium), theme::SIGNAL_YELLOW);
        assert_eq!(severity_color(LimitSeverity::High), theme::SIGNAL_RED);
    });
}

#[gpui::test]
fn indicator_color_dims_unknown(cx: &mut gpui::TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    cx.update(|cx| {
        assert_eq!(
            indicator_color(StatusIndicator::None, cx),
            theme::SIGNAL_GREEN
        );
        assert_eq!(
            indicator_color(StatusIndicator::Minor, cx),
            theme::SIGNAL_YELLOW
        );
        assert_eq!(
            indicator_color(StatusIndicator::Major, cx),
            theme::SIGNAL_ORANGE
        );
        assert_eq!(
            indicator_color(StatusIndicator::Critical, cx),
            theme::SIGNAL_RED
        );
        assert_eq!(
            indicator_color(StatusIndicator::Unknown, cx),
            theme::TEXT_TERTIARY
        );
    });
}
