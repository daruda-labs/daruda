//! Terminal pane construction and PTY startup.

use super::*;

impl Workspace {
    pub(in crate::workspace) fn create_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Pane, PaneSpawnError> {
        let cwd = self.default_cwd_for_new_pane();
        self.create_pane_at(cwd, window, cx)
    }

    /// `create_pane` rooted at `cwd` rather than the inherited default — a
    /// task started in a lane must run at that lane's root.
    pub(in crate::workspace) fn create_pane_at(
        &mut self,
        cwd: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Pane, PaneSpawnError> {
        let account = self.default_account_selection_for_new_pane(None);
        let prepared = self.resolve_account(account, AccountDomain::Any);
        self.create_pane_with_cwd(cwd, account, prepared.as_ref(), window, cx)
    }

    /// Like `create_pane` but forces a specific initial cwd. Used by
    /// session restore so each restored pane starts at the directory
    /// it last tracked, independent of the focused-pane / project
    /// inheritance rules.
    ///
    /// `account` is the pane's own [`AccountSelection`], cached on the
    /// resulting `TerminalContent` purely for re-serialization; callers that
    /// want a fresh pane to inherit a configured default pass
    /// [`Self::default_account_selection_for_new_pane`]'s result here
    /// explicitly. `prepared` is that selection already resolved by
    /// [`resolve_pane_account`], carrying the env to inject into the spawned
    /// shell; `None` spawns with the ambient environment unchanged.
    pub(in crate::workspace) fn create_pane_with_cwd(
        &mut self,
        cwd: Option<PathBuf>,
        account: daruda_store::accounts::AccountSelection,
        prepared: Option<&PreparedAccount>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Pane, PaneSpawnError> {
        let pane_id = self.alloc_id();
        self.spawn_terminal_pane(pane_id, cwd, account, prepared, window, cx)
    }

    /// Spawn a shell for the pane `pane_id` — a new one, or one whose shell
    /// exited and is being started again in place.
    pub(in crate::workspace) fn spawn_terminal_pane(
        &mut self,
        pane_id: PaneId,
        cwd: Option<PathBuf>,
        account: daruda_store::accounts::AccountSelection,
        prepared: Option<&PreparedAccount>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Pane, PaneSpawnError> {
        // Propagate the workspace's terminal config so every pane
        // starts with the same font_size / spacing. Zoom actions
        // diverge each view's runtime font_size individually.
        let config = self.mirrors.terminal_config;
        // `shell_program` is the effective shell from `apply_config`; falls
        // back to `PtyConfig::default()`'s `$SHELL`/`/bin/zsh` when unset.
        let mut pty_config = PtyConfig {
            cwd,
            ..PtyConfig::default()
        };
        if let Some(program) = self.mirrors.shell_program.as_deref() {
            pty_config.shell = program.to_string();
        }
        if let Some(prepared) = prepared {
            self.prepare_account_dir(prepared, cx);
            apply_account_env(&mut pty_config, &prepared.env);
        }
        let handle = spawn_pty(&pty_config).map_err(PaneSpawnError::Pty)?;
        let pty_pid = handle.child_pid;
        let (stdin_tx, stdout_rx, exit_rx, error_rx, master) = handle.into_parts();
        // Keep a sibling sender so non-keyboard callers
        // (Workspace::send_to_pane) can write into the same PTY
        // without going through the TerminalView's keyboard handler.
        // The original `stdin_tx` is moved into the TerminalInput
        // closure below, so we clone before that.
        let pty_input_tx = stdin_tx.clone();

        // Create the VT session up front so its error can propagate out
        // of create_pane rather than panicking inside the cx.new closure.
        // Start at the default grid; `resize_terminal` immediately reshapes
        // the session to the pane's measured cols/rows on first layout.
        let session =
            TerminalSession::new(TerminalDims::default(), config).map_err(PaneSpawnError::Vt)?;
        // Every spawn path ends here, so a success is the retry the pinned
        // pane-spawn message asked for.
        self.last_error = None;

        // Wakes the stdout poll out of its idle backoff the instant
        // bytes head for the PTY, so the echo is drained at the fast
        // interval — typing latency stays at IDLE_POLL even after a
        // long quiet period.
        let (poke_tx, poke_rx) = futures::channel::mpsc::unbounded::<()>();
        let input_poke_tx = poke_tx.clone();

        let font_family = self.mirrors.font_family.clone();
        let view = cx.new(|cx| {
            let focus_handle = cx.focus_handle();
            let input = TerminalInput::new(move |bytes| {
                let _ = stdin_tx.send(bytes.to_vec());
                let _ = input_poke_tx.unbounded_send(());
            });
            let mut tv = TerminalView::new_with_input(session, focus_handle, input);
            tv.set_font(daruda_terminal::terminal_font_with_family(&font_family));
            tv
        });

        let workspace_entity = cx.entity().clone();
        let stdout_task = Self::spawn_stdout_poll(
            view.clone(),
            stdout_rx,
            exit_rx,
            error_rx,
            poke_rx,
            workspace_entity,
            pane_id,
            window,
            cx,
        );

        // Bridge `TerminalViewEvent`s into Workspace-side platform calls
        // (dock attention, notifications, …), gated by config. `pane_id` is
        // captured so the focus-gate can identify the raising pane; the
        // window-aware subscribe lets `ContextMenuRequested` open the host
        // context menu / annotation dialog (both need `&mut Window`).
        let captured_pane_id = pane_id;
        let view_event_sub = cx.subscribe_in(
            &view,
            window,
            move |this, _view, event: &daruda_terminal::TerminalViewEvent, window, cx| {
                handle_view_event(this, captured_pane_id, event, window, cx);
            },
        );

        // Register the pane's shell PID with the PTY tracker so the
        // sysinfo poller can find `claude` descendants of this pane
        // and bind them back to its session_id. Stub
        // panes (no pty_pid) skip registration.
        if let Some(pid) = pty_pid {
            self.claude.pty_tracker.register(pane_id, pid);
        }

        Ok(Pane {
            id: pane_id,
            content: PaneContent::Terminal(TerminalContent {
                task_run: None,
                view,
                master,
                shell_pid: pty_pid,
                exited: false,
                cached_title: daruda_terminal::ux::strings::fallback_title().into(),
                cached_cwd: None,
                _stdout_task: stdout_task,
                _view_event_subscription: view_event_sub,
                pty_input_tx: Some(pty_input_tx),
                poke_tx,
                account,
            }),
        })
    }
}
