//! Construction, connection-status transitions and per-pane display settings.

use super::*;

impl AgentChatView {
    /// Build a fresh view. The session is *not* started here — the Workspace
    /// connects it lazily on first focus (`maybe_connect_agent_chat`), so cold
    /// restore doesn't spin up an agent process per pane. `status` is decided
    /// by the caller (Idle when a cwd is present, Error otherwise).
    #[allow(clippy::too_many_arguments)] // Restore/create seed values — bundling them into a struct only wraps callers.
    pub fn new(
        pane_id: PaneId,
        window_handle: AnyWindowHandle,
        cwd: Option<PaneCwd>,
        status: AgentSessionStatus,
        session_id: Option<String>,
        agent: AgentIdentity,
        title: Option<String>,
        defaults: TranscriptDefaults,
        syntax_theme: String,
        agent_default_writer: AgentDefaultWriter,
        cx: &mut Context<Self>,
    ) -> Self {
        // Re-rasterize mermaid diagrams when the UI theme changes. The chat
        // surface/foreground come from the terminal mirror, but the host
        // profile still uses UI semantic colors for notes/status accents.
        let theme_observer = cx.observe_global::<daruda_ui::theme::DarudaTheme>(|this, cx| {
            this.assets.clear_mermaid();
            let dark = Self::host_is_dark(cx);
            this.reconcile_mermaid(dark, cx);
            // A theme swap invalidates the whole conversation's cached artifacts,
            // not one call's — the only correct scope here is the full pass.
            this.reconcile_tool_images(&super::super::reconcile::ReconcileScope::All, cx);
            // Diff embeds bake their palette in and cannot re-theme themselves.
            this.reconcile_embeds_after_theme_change(cx);
        });
        Self {
            pane_id,
            session: Session::Interactive(None),
            window_handle,
            focus_handle: cx.focus_handle(),
            cwd,
            status,
            session_id,
            // Both picks are patched in by session restore right after
            // construction — see `Workspace::rebuild_layout`.
            picked_mode_id: None,
            picked_model_id: None,
            agent_id: agent.id,
            agent_program: None,
            agent_vocabulary_source: None,
            agent_name: agent.name,
            agent_icon: agent.icon,
            // A restored pane connects lazily; the resume decision (and the
            // replay gate that coalesces it) is made at connect time by
            // `maybe_connect_agent_chat`, not here.
            replay: Replay::Live,
            items: Vec::new(),
            queue: PromptQueue::default(),
            briefing: None,
            _event_pump: None,
            pending_permissions: HashSet::new(),
            permissions_told_to_phone: HashSet::new(),
            phone_turn_state: None,
            activity: ActivityTracker::default(),
            assets: AssetCache::default(),
            fold: FoldState::with_mode(defaults.fold_mode),
            fold_editor: FoldEditorState::default(),
            filter_editor: FilterEditorState::default(),
            activity_options_tab: ActivityOptionsTab::Fold,
            #[cfg(feature = "screenshot")]
            screenshot_filter_open: false,
            #[cfg(feature = "screenshot")]
            screenshot_fold_open: false,
            #[cfg(feature = "screenshot")]
            screenshot_options_open: false,
            content_width: defaults.content_width,
            content_width_chosen: false,
            tail_steps: PaneChoice::Seeded(defaults.tail.steps),
            tail_calls: PaneChoice::Seeded(defaults.tail.calls),
            display_filter: PaneChoice::Seeded(defaults.filter),
            defaults,
            list_state: {
                // Starts empty; `sync_list_after` splices items in as events
                // arrive. `Top` alignment + `Tail` follow = scroll history up
                // freely, auto-stick to the bottom while streaming.
                let state = ListState::new(
                    0,
                    ListAlignment::Top,
                    px(daruda_ui::theme::AGENT_CHAT_LIST_OVERDRAW),
                );
                state.set_follow_mode(FollowMode::Tail);
                state
            },
            rows: Vec::new(),
            rows_activity: ActivityState::Idle,
            live_units: LiveSubagentUnits::default(),
            filter_matches: FilterMatchIndex::default(),
            command_analysis: CommandAnalysisIndex::default(),
            turn_boundary: Default::default(),
            syntax_theme,
            preferred_editor: String::new(),
            agent_default_writer,
            activity_title: None,
            session_config: SessionConfig::default(),
            session_capabilities: SessionCapabilitiesView::default(),
            draft_attachments: Vec::new(),
            session_usage: None,
            overage_notice: None,
            overage_stop_markers: BTreeSet::new(),
            plan: Vec::new(),
            // Seed the persisted title so a restored dormant pane shows its
            // label before the session loads; the agent replaces it on the
            // first `SessionInfoChanged` after resume. An empty string is
            // treated as absent, matching the tab-label seed in
            // `create_agent_chat_pane` so the header and tab agree.
            session_title: title.filter(|t| !t.is_empty()),
            session_updated_at: None,
            plan_collapsed: false,
            warned_dropped_terminal_output: false,
            plan_scroll: ScrollHandle::new(),
            list_bounds: None,
            pane_width: None,
            autoscroll_task: None,
            selection_drag_active: false,
            dim_amount: 0.0,
            _theme_observer: theme_observer,
            #[cfg(any(test, feature = "test-support"))]
            render_count: std::cell::Cell::new(0),
        }
    }

    /// The resolved activity-bar title, or `None` when the session has neither
    /// an agent-supplied title nor a user prompt yet (the caller falls back to
    /// the pane's agent name).
    pub fn activity_title(&self) -> Option<&str> {
        self.activity_title.as_deref()
    }

    /// Seed the agent-supplied title the way a `SessionInfoChanged` would,
    /// including the `rebuild_rows` that derives `activity_title` from it —
    /// writing the field alone leaves the derived cache stale.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_session_title_for_test(&mut self, title: &str) {
        self.session_title = Some(title.to_string());
        self.rebuild_rows();
    }

    /// The Workspace-resolved syntax theme id this pane's diff embeds are built
    /// against. Always set — see the field's doc for why it is a construction
    /// seed rather than something the first event fills in.
    pub fn syntax_theme(&self) -> &str {
        &self.syntax_theme
    }

    /// Record the Workspace-resolved syntax theme id. Single update site for
    /// `syntax_theme`; `apply_event` writes the value the Workspace passes it
    /// each event, and the config-reload path writes the new palette name
    /// directly — an idle pane gets no events, so waiting for one would leave the
    /// diff embeds fingerprinted against a palette the user already left.
    pub fn set_preferred_editor(&mut self, editor: &str) {
        if self.preferred_editor != editor {
            self.preferred_editor = editor.to_owned();
        }
    }

    pub fn preferred_editor(&self) -> &str {
        &self.preferred_editor
    }

    pub fn set_syntax_theme(&mut self, theme: &str) {
        if self.syntax_theme != theme {
            self.syntax_theme = theme.to_owned();
        }
    }

    /// Set the inactive-pane dim (`0.0` = focused, `> 0.0` = unfocused split
    /// leaf). No `cx.notify` here — the caller notifies only the views whose
    /// amount changed, mirroring `TerminalView::set_dim_amount`.
    pub fn set_dim_amount(&mut self, amount: f32) {
        self.dim_amount = amount.clamp(0.0, 1.0);
    }

    /// Enter the `Connecting` status and repaint. Self-notifying so the event
    /// pump can't advance the connection state without dirtying the pane.
    pub fn set_connecting(&mut self, cx: &mut Context<Self>) {
        self.status = AgentSessionStatus::Connecting;
        cx.notify();
    }

    /// Enter `Connecting` for a connect that is about to be spawned, arming the
    /// replay gate from that connect's own resume target: `Some(id)` asks for
    /// `session/load` of `id`, `None` for a fresh `session/new`.
    ///
    /// Taking the resume target as the argument is the point — the gate and the
    /// request cannot disagree, because there is no way to open one without
    /// naming the other. Distinct from [`Self::set_connecting`], which only
    /// downgrades the status banner and must leave a live replay alone.
    pub fn begin_connect(&mut self, resume: Option<String>, cx: &mut Context<Self>) {
        self.replay = Replay::for_connect(resume);
        self.status = AgentSessionStatus::Connecting;
        cx.notify();
    }

    /// True while a `session/load` replay is still streaming — rows are being
    /// accumulated rather than projected. The gate is module-private, so this is
    /// how a test outside `view/` observes it (same shape as `turn_is_idle`).
    #[cfg(any(test, feature = "test-support"))]
    pub fn is_replaying(&self) -> bool {
        self.replay.is_loading()
    }

    /// Enter the `Error` status carrying `message` and repaint. Self-notifying
    /// so the event pump can't surface a failure without dirtying the pane.
    ///
    /// Callers that hold a classified failure pass its
    /// [`remedy`](daruda_acp::AcpFailure::remedy); a locally-raised error with
    /// nothing to offer passes
    /// [`Remedy::NoneAvailable`](daruda_acp::Remedy::NoneAvailable).
    pub fn set_error(
        &mut self,
        message: String,
        remedy: daruda_acp::Remedy,
        cx: &mut Context<Self>,
    ) {
        self.settle_mirror_intent();
        self.status = AgentSessionStatus::Error { message, remedy };
        cx.notify();
    }

    /// Enter the `PreparingRuntime` status at `phase` and repaint.
    /// Self-notifying so the runtime-progress drain can't advance the banner
    /// without dirtying the pane.
    pub fn set_preparing(&mut self, phase: RuntimePrepPhase, cx: &mut Context<Self>) {
        self.status = AgentSessionStatus::PreparingRuntime(phase);
        cx.notify();
    }

    /// Enter the `Handshaking` status at `phase` and repaint. Called from
    /// `apply_event` only while still `Connecting`/`Handshaking` — a
    /// `ConnectProgress` arriving after `Connected`/`Error` is stale and must
    /// not resurrect the connecting banner.
    pub(crate) fn set_handshaking(&mut self, phase: ConnectPhase, cx: &mut Context<Self>) {
        self.status = AgentSessionStatus::Handshaking(phase);
        cx.notify();
    }

    /// Blend `c` toward gray by the current dim amount, alpha preserved. The
    /// render wraps every color it applies with this so an unfocused pane
    /// grays like an inactive terminal while keeping window translucency.
    pub(crate) fn dim(&self, c: gpui::Hsla) -> gpui::Hsla {
        daruda_ui::theme::dim_toward_gray(c, self.dim_amount)
    }

    /// Whether the agent-chat paint surface is currently dark. This follows the
    /// terminal-mirrored chat background, not the surrounding UI theme, so a
    /// light UI shell with a dark terminal preset still gets dark-surface
    /// Mermaid colors and cache keys.
    pub(crate) fn host_is_dark(cx: &App) -> bool {
        !daruda_ui::theme::agent_chat_syntax_is_light(cx)
    }
}
