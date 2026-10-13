use daruda_acp::{ChatItem, DiffView, ToolCallItem, ToolKindView, ToolOutputBlock, ToolStatusView};
use daruda_store::project::PaneCwd;
use gpui::TestAppContext;

use super::super::view::tests::make_test_view;
use super::super::window_access::WindowAccess;
use super::tool_images::{
    MAX_RESOURCE_IMAGE_BYTES, MAX_RESOURCE_IMAGE_PREVIEWS, load_resource_image, resource_image_path,
};
use super::{ReconcileScope, mermaid_key};
use daruda_transcript::fold_mode::{FoldMode, FoldPreset};
use daruda_ui::diff_editor::DiffColors;

const KEY: &str = "call_1#0";

/// The reconcilers' non-fold caller (an async ACP event) resolves the window
/// from the stored handle. `window_handle` is `Copy`, so this borrows nothing
/// the caller still needs.
fn by_handle(v: &super::AgentChatView) -> WindowAccess<'static> {
    WindowAccess::ByHandle(v.window_handle)
}

/// A view whose fold mode opens every tool card, for the tests that need a
/// card's body — and so its embeds — on screen.
fn make_tools_expanded_view(cx: &mut TestAppContext) -> gpui::WindowHandle<super::AgentChatView> {
    let window = make_test_view(cx);
    window
        .update(cx, |v, _, _| v.fold.set_mode(all_tools_expanded()))
        .expect("the window is open");
    window
}

/// A syntax theme id the diff reconciler's highlight pass can resolve.
const SYNTAX_THEME: &str = "base16-ocean.dark";

/// A theme swap has to reach the diff embeds, and reach *only* them.
///
/// A diff embed bakes its palette in at build time: `set_highlight_override`
/// replaces the editor's own highlighter, and that override is what
/// `input/element.rs` reads instead of resolving
/// `cx.theme().highlight_theme` per paint. So nothing about a swap reaches a
/// built diff embed unless the reconciler rebuilds it — a dark→light swap
/// would otherwise leave dark foregrounds on light hunk rows for the rest of
/// the conversation. An output embed keeps the built-in highlighter and is
/// already theme-live, so rebuilding one would be pure churn (and would drop
/// its scroll position).
#[gpui::test]
fn a_theme_swap_rebuilds_diff_embeds_and_leaves_output_embeds_alone(cx: &mut TestAppContext) {
    let window = make_tools_expanded_view(cx);
    let view = window.root(cx).expect("the view is the window root");

    cx.update(|cx| daruda_ui::theme::set_agent_chat_bg(cx, 0, 0, 0));
    view.update(cx, |v, cx| {
        v.set_syntax_theme(SYNTAX_THEME);
        v.items = vec![tool_call_with(
            vec![raw("out")],
            vec![diff("a.rs", "first\n")],
        )];
        let is_light = daruda_ui::theme::agent_chat_syntax_is_light(cx);
        v.reconcile_diff_editors(
            SYNTAX_THEME,
            is_light,
            &ReconcileScope::All,
            &mut by_handle(v),
            cx,
        );
        v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
    });
    let built = |cx: &mut TestAppContext| {
        view.read_with(cx, |v, _| {
            (
                v.assets.diff_editors[KEY].entity_id(),
                v.assets.output_editors[KEY].entity_id(),
            )
        })
    };
    let dark = built(cx);

    // Same diff, different theme — the content-only rule would skip this.
    cx.update(|cx| daruda_ui::theme::set_agent_chat_bg(cx, 255, 255, 255));
    view.update(cx, |v, cx| v.reconcile_embeds_after_theme_change(cx));
    let light = built(cx);

    assert_ne!(
        dark.0, light.0,
        "a diff embed's baked palette only tracks a theme swap through a rebuild"
    );
    assert_eq!(
        dark.1, light.1,
        "an output embed resolves colours per paint, so a swap must not churn it"
    );
}

/// And the swap has to *reach* that pass on its own. A UI-preset switch goes
/// through `apply_ui_theme`, which only re-sets the `DarudaTheme` global — so
/// the view's own observer is the whole delivery path.
#[gpui::test]
fn setting_the_theme_global_rebuilds_diff_embeds_through_the_observer(cx: &mut TestAppContext) {
    let window = make_tools_expanded_view(cx);
    let view = window.root(cx).expect("the view is the window root");

    view.update(cx, |v, cx| {
        v.set_syntax_theme(SYNTAX_THEME);
        v.items = vec![diff_tool_call(vec![diff("a.rs", "first\n")])];
        let is_light = daruda_ui::theme::agent_chat_syntax_is_light(cx);
        v.reconcile_diff_editors(
            SYNTAX_THEME,
            is_light,
            &ReconcileScope::All,
            &mut by_handle(v),
            cx,
        );
    });
    let before = view.read_with(cx, |v, _| v.assets.diff_editors[KEY].entity_id());

    cx.update(|cx| {
        // Any diff colour the palette snapshot carries will do; this one is
        // read straight into `DiffColors::add_bg`.
        cx.set_global(daruda_ui::theme::DarudaTheme {
            file_diff_add_bg: gpui::hsla(0.5, 0.5, 0.5, 1.0),
            ..Default::default()
        });
    });
    cx.run_until_parked();

    assert_ne!(
        before,
        view.read_with(cx, |v, _| v.assets.diff_editors[KEY].entity_id()),
        "replacing the theme global must reach the diff embeds through the \
         view's own observer — nothing else delivers a UI-preset switch"
    );
}

fn tool_call(output: Vec<ToolOutputBlock>) -> ChatItem {
    tool_call_with(output, Vec::new())
}

fn diff_tool_call(diffs: Vec<DiffView>) -> ChatItem {
    tool_call_with(Vec::new(), diffs)
}

/// Live (`InProgress`). Running does not open a tool card, so the
/// editor-lifecycle tests put it on screen with [`make_tools_expanded_view`]
/// — a collapsed card by design has no embeds at all (see
/// `folding_a_tool_card_materializes_and_releases_its_editors`).
fn tool_call_with(output: Vec<ToolOutputBlock>, diffs: Vec<DiffView>) -> ChatItem {
    ChatItem::ToolCall(ToolCallItem {
        id: "call_1".to_string(),
        title: "Bash".to_string(),
        kind: ToolKindView::Execute,
        tool_name: None,
        status: ToolStatusView::InProgress,
        diffs,
        output,
        raw_input: None,
        locations: Vec::new(),
        parent_tool_id: None,
        exit: None,
    })
}

fn diff(path: &str, new_text: &str) -> DiffView {
    DiffView {
        path: path.into(),
        old_text: Some("old\n".to_string()),
        new_text: new_text.to_string(),
    }
}

fn raw(text: &str) -> ToolOutputBlock {
    ToolOutputBlock::RawText {
        text: text.to_string(),
        truncated_from: None,
    }
}

fn write_test_png(path: &std::path::Path) {
    image::RgbaImage::from_pixel(2, 1, image::Rgba([16, 32, 48, 255]))
        .save(path)
        .expect("write test PNG");
}

#[test]
fn resource_image_candidates_are_local_rasters_only() {
    let cwd = std::path::Path::new("/work/project");
    assert_eq!(
        resource_image_path("shots/result.PNG", None, cwd),
        Some(cwd.join("shots/result.PNG"))
    );
    assert_eq!(
        resource_image_path("artifact", Some("image/png"), cwd),
        Some(cwd.join("artifact"))
    );
    assert!(resource_image_path("notes.txt", None, cwd).is_none());
    assert!(resource_image_path("picture.png", Some("text/plain"), cwd).is_none());
    assert!(
        resource_image_path("https://example.com/picture.png", Some("image/png"), cwd).is_none()
    );
    assert!(resource_image_path("mcp://server/picture.png", Some("image/png"), cwd).is_none());
}

#[test]
fn resource_image_loader_authorizes_and_bounds_local_files() {
    let dir = tempfile::tempdir().unwrap();
    let image_path = dir.path().join("preview.png");
    write_test_png(&image_path);

    let loaded = load_resource_image(&image_path, dir.path()).expect("valid local PNG");
    assert_eq!((loaded.width, loaded.height), (2, 1));

    let too_large = dir.path().join("large.png");
    std::fs::File::create(&too_large)
        .unwrap()
        .set_len(MAX_RESOURCE_IMAGE_BYTES + 1)
        .unwrap();
    assert!(load_resource_image(&too_large, dir.path()).is_none());

    #[cfg(unix)]
    assert!(
        super::tool_images::authorized_resource_image_path(
            std::path::Path::new("/etc/hosts"),
            dir.path()
        )
        .is_none()
    );
}

#[gpui::test]
fn settled_local_resource_link_is_loaded_into_the_render_cache(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let image_path = dir.path().join("agent screenshot.png");
    write_test_png(&image_path);

    let window = make_test_view(cx);
    let view = window.root(cx).expect("the view is the window root");
    view.update(cx, |v, cx| {
        v.cwd = Some(PaneCwd::Local(dir.path().to_path_buf()));
        v.items = vec![settled_tool(
            "call_1",
            vec![ToolOutputBlock::ResourceLink {
                uri: image_path.to_string_lossy().into_owned(),
                name: "agent screenshot.png".into(),
                mime: None,
            }],
        )];
        v.reconcile_tool_images(&ReconcileScope::All, cx);
        assert!(
            v.assets.resource_image_sources.is_empty(),
            "a collapsed card must not allocate a preview"
        );
    });
    window
        .update(cx, |v, window, cx| {
            v.set_fold_mode(all_tools_expanded(), window, cx);
        })
        .expect("the window is open");
    cx.run_until_parked();

    view.read_with(cx, |v, _| {
        assert!(
            matches!(
                v.assets.resource_images.lock().unwrap().get(KEY),
                Some(Some(_))
            ),
            "the background file read must land a GPU-ready preview"
        );
    });
}

#[gpui::test]
fn turn_settle_revisits_a_resource_link_that_arrived_live(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let image_path = dir.path().join("live.png");
    write_test_png(&image_path);

    let window = make_test_view(cx);
    let view = window.root(cx).expect("the view is the window root");
    view.update(cx, |v, cx| {
        v.cwd = Some(PaneCwd::Local(dir.path().to_path_buf()));
        v.fold.set_mode(all_tools_expanded());
        v.items = vec![tool_named(
            "call_1",
            vec![ToolOutputBlock::ResourceLink {
                uri: image_path.to_string_lossy().into_owned(),
                name: "live.png".into(),
                mime: Some("image/png".into()),
            }],
        )];
        v.reconcile_tool_images(&ReconcileScope::All, cx);
        assert!(v.assets.resource_image_sources.is_empty());

        v.apply_event(
            daruda_acp::AcpEvent::TurnEnded {
                completed_normally: true,
                stop_reason: "EndTurn".into(),
                usage: None,
            },
            SYNTAX_THEME,
            false,
            cx,
        );
    });
    cx.run_until_parked();

    view.read_with(cx, |v, _| {
        assert!(matches!(
            v.assets.resource_images.lock().unwrap().get(KEY),
            Some(Some(_))
        ));
    });
}

/// A tool call can land in the window between Stop and its ack — Stop's own
/// revisit already ran, and the call was live when it arrived. The ack is
/// what settles it, so the ack owes the revisit.
#[gpui::test]
fn a_cancel_ack_revisits_a_resource_link_that_landed_after_the_stop(cx: &mut TestAppContext) {
    use agent_client_protocol::schema::v1::{
        Content, ContentBlock, ResourceLink, SessionUpdate, ToolCall as WireToolCall,
        ToolCallContent,
    };

    let dir = tempfile::tempdir().unwrap();
    let image_path = dir.path().join("late.png");
    write_test_png(&image_path);

    let window = make_test_view(cx);
    let view = window.root(cx).expect("the view is the window root");
    view.update(cx, |v, cx| {
        v.cwd = Some(PaneCwd::Local(dir.path().to_path_buf()));
        v.fold.set_mode(all_tools_expanded());
        v.set_turn_in_flight();
        v.cancel_turn(cx);

        // Already on the wire when Stop landed: a live call carrying a
        // resource link, which Stop's revisit came too early to see.
        v.apply_event(
            daruda_acp::AcpEvent::Update(Box::new(SessionUpdate::ToolCall(
                WireToolCall::new("call_1", "Read").content(vec![ToolCallContent::Content(
                    Content::new(ContentBlock::ResourceLink(ResourceLink::new(
                        "late.png",
                        image_path.to_string_lossy().into_owned(),
                    ))),
                )]),
            ))),
            SYNTAX_THEME,
            false,
            cx,
        );
        assert!(
            v.assets.resource_image_sources.is_empty(),
            "a live call is not eligible yet"
        );

        v.apply_event(
            daruda_acp::AcpEvent::TurnEnded {
                completed_normally: false,
                stop_reason: "Cancelled".into(),
                usage: None,
            },
            SYNTAX_THEME,
            false,
            cx,
        );
    });
    cx.run_until_parked();

    view.read_with(cx, |v, _| {
        assert!(matches!(
            v.assets.resource_images.lock().unwrap().get(KEY),
            Some(Some(_))
        ));
    });
}

/// A `session/load` replay carries tool calls the prior process never
/// finished, and closes with `Connected` rather than a `TurnEnded`. That
/// exit settles them, which is the first moment their resource links are
/// eligible — so it owes the same revisit the turn-settle path does.
#[gpui::test]
fn a_finished_restore_revisits_a_resource_link_the_replay_left_live(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let image_path = dir.path().join("restored.png");
    write_test_png(&image_path);

    let window = make_test_view(cx);
    let view = window.root(cx).expect("the view is the window root");
    view.update(cx, |v, cx| {
        v.cwd = Some(PaneCwd::Local(dir.path().to_path_buf()));
        v.fold.set_mode(all_tools_expanded());
        v.begin_connect(Some("sess-1".into()), cx);
        v.items = vec![tool_named(
            "call_1",
            vec![ToolOutputBlock::ResourceLink {
                uri: image_path.to_string_lossy().into_owned(),
                name: "restored.png".into(),
                mime: None,
            }],
        )];
        v.reconcile_tool_images(&ReconcileScope::All, cx);
        assert!(
            v.assets.resource_image_sources.is_empty(),
            "a call the replay left live is not eligible yet"
        );

        v.apply_event(
            daruda_acp::AcpEvent::Connected {
                program: None,
                session_id: "sess-1".into(),
                modes: None,
                config_options: Vec::new(),
                capabilities: Default::default(),
                login_methods: Vec::new(),
            },
            SYNTAX_THEME,
            false,
            cx,
        );
    });
    cx.run_until_parked();

    view.read_with(cx, |v, _| {
        assert!(matches!(
            v.assets.resource_images.lock().unwrap().get(KEY),
            Some(Some(_))
        ));
    });
}

/// An adapter that closes the stream mid-load leaves no event behind it, so
/// this exit is the last chance to revisit: after it only a fold toggle or a
/// theme swap would ever reach the call again.
#[gpui::test]
fn an_aborted_restore_revisits_a_resource_link_the_replay_left_live(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let image_path = dir.path().join("aborted.png");
    write_test_png(&image_path);

    let window = make_test_view(cx);
    let view = window.root(cx).expect("the view is the window root");
    view.update(cx, |v, cx| {
        v.cwd = Some(PaneCwd::Local(dir.path().to_path_buf()));
        v.fold.set_mode(all_tools_expanded());
        v.begin_connect(Some("sess-1".into()), cx);
        v.items = vec![tool_named(
            "call_1",
            vec![ToolOutputBlock::ResourceLink {
                uri: image_path.to_string_lossy().into_owned(),
                name: "aborted.png".into(),
                mime: None,
            }],
        )];
        v.reconcile_tool_images(&ReconcileScope::All, cx);
        assert!(v.assets.resource_image_sources.is_empty());

        v.abort_restore(cx);
    });
    cx.run_until_parked();

    view.read_with(cx, |v, _| {
        assert!(matches!(
            v.assets.resource_images.lock().unwrap().get(KEY),
            Some(Some(_))
        ));
    });
}

#[gpui::test]
fn stop_revisits_a_resource_link_without_waiting_for_an_ack(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let image_path = dir.path().join("stopped.png");
    write_test_png(&image_path);

    let window = make_test_view(cx);
    let view = window.root(cx).expect("the view is the window root");
    view.update(cx, |v, cx| {
        v.cwd = Some(PaneCwd::Local(dir.path().to_path_buf()));
        v.fold.set_mode(all_tools_expanded());
        v.items = vec![tool_named(
            "call_1",
            vec![ToolOutputBlock::ResourceLink {
                uri: image_path.to_string_lossy().into_owned(),
                name: "stopped.png".into(),
                mime: None,
            }],
        )];
        v.set_turn_in_flight();
        v.cancel_turn(cx);
    });
    cx.run_until_parked();

    view.read_with(cx, |v, _| {
        assert!(matches!(
            v.assets.resource_images.lock().unwrap().get(KEY),
            Some(Some(_))
        ));
    });
}

#[gpui::test]
fn expanded_resource_preview_cache_stays_bounded(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut items = Vec::new();
    for ix in 0..MAX_RESOURCE_IMAGE_PREVIEWS + 2 {
        let path = dir.path().join(format!("preview-{ix}.png"));
        write_test_png(&path);
        items.push(settled_tool(
            &format!("call_{ix}"),
            vec![ToolOutputBlock::ResourceLink {
                uri: path.to_string_lossy().into_owned(),
                name: format!("preview-{ix}.png"),
                mime: None,
            }],
        ));
    }

    let window = make_test_view(cx);
    let view = window.root(cx).expect("the view is the window root");
    view.update(cx, |v, cx| {
        v.cwd = Some(PaneCwd::Local(dir.path().to_path_buf()));
        v.fold.set_mode(all_tools_expanded());
        v.items = items;
        v.reconcile_tool_images(&ReconcileScope::All, cx);
    });
    cx.run_until_parked();

    view.read_with(cx, |v, _| {
        assert!(v.assets.resource_image_inflight.is_empty());
        assert_eq!(
            v.assets.resource_image_sources.len(),
            MAX_RESOURCE_IMAGE_PREVIEWS
        );
        assert_eq!(
            v.assets.resource_images.lock().unwrap().len(),
            MAX_RESOURCE_IMAGE_PREVIEWS
        );
    });
}

/// Seeding replaces `items` outside the ACP pump, so it owes the same embed
/// pass an event does — without it the replay harness and the screenshot
/// scenarios render a path no live session takes.
#[cfg(feature = "devtools")]
#[gpui::test]
fn seeding_a_transcript_builds_its_diff_embeds(cx: &mut TestAppContext) {
    let window = make_tools_expanded_view(cx);

    // No `set_syntax_theme` here on purpose: the palette a diff embed is
    // fingerprinted against is a construction seed, so seeding is enough.
    window
        .update(cx, |v, window, cx| {
            v.seed_transcript(
                vec![tool_call_with(
                    vec![raw("out")],
                    vec![diff("a.rs", "first\nsecond\n")],
                )],
                window,
                cx,
            );
        })
        .expect("the test window is live");

    let view = window.root(cx).expect("the view is the window root");
    view.read_with(cx, |v, _| {
        assert!(
            v.assets.diff_editors.contains_key(KEY),
            "a seeded diff gets the same editor a streamed one does"
        );
        assert!(
            v.assets.output_editors.contains_key(KEY),
            "and so does its verbatim output"
        );
    });
}

/// Settled, so its card is collapsed and renders no body.
fn settled_tool(id: &str, output: Vec<ToolOutputBlock>) -> ChatItem {
    let ChatItem::ToolCall(mut tc) = tool_named(id, output) else {
        unreachable!("tool_named builds a ToolCall")
    };
    tc.status = ToolStatusView::Completed;
    ChatItem::ToolCall(tc)
}

/// A running call opens no card on its own: only the fold mode or a click
/// does, so with neither its body is off screen and needs no editor.
#[gpui::test]
fn a_live_tool_card_without_a_fold_rule_gets_no_embed_editors(cx: &mut TestAppContext) {
    let window = make_test_view(cx);
    let view = window.root(cx).expect("the view is the window root");

    view.update(cx, |v, cx| {
        v.items = vec![tool_named("call_a", vec![raw("streaming output")])];
        v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
    });
    view.read_with(cx, |v, _| {
        assert!(v.assets.output_editors.is_empty());
    });
}

/// Materialization has to follow the fold both ways: a collapsed card gets
/// no editor, expanding it builds one, collapsing it again releases it. Each
/// embed editor is an `InputState` costing ~2 gpui focus handles, whose slot
/// `release_dropped_focus_handles` walks on every effect drain — and a
/// collapsed card renders no body (`FoldRow::block` only runs the body
/// closure when expanded), so an editor built for one is pure cost.
///
/// Driven through `window.update`, which is where the app drives it from: a
/// fold click arrives via `cx.listener`, i.e. from *inside* the window's own
/// update cycle. gpui takes the window out of `App::windows` for the duration
/// of an update, so an editor built by re-entering `update_window` there is
/// silently dropped ("window not found") and the body falls back to the
/// markdown walk the embed exists to avoid. Toggling through `Entity::update`
/// instead is outside that cycle and cannot see it.
#[gpui::test]
fn folding_a_tool_card_materializes_and_releases_its_editors(cx: &mut TestAppContext) {
    use super::super::fold::FoldKey;

    let window = make_test_view(cx);
    let view = window.root(cx).expect("the view is the window root");

    view.update(cx, |v, cx| {
        v.items = vec![settled_tool("call_a", vec![raw("out")])];
        v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
    });
    view.read_with(cx, |v, _| assert!(v.assets.output_editors.is_empty()));

    let click = |cx: &mut TestAppContext| {
        window
            .update(cx, |v, window, cx| {
                v.toggle_fold(FoldKey::Tool("call_a".into()), window, cx);
            })
            .expect("the window is open");
    };

    click(cx);
    view.read_with(cx, |v, _| {
        assert!(
            v.assets.output_editors.contains_key("call_a#0"),
            "expanding a card must build the editor its body needs"
        );
    });

    click(cx);
    view.read_with(cx, |v, _| {
        assert!(
            v.assets.output_editors.is_empty(),
            "collapsing releases the editor again"
        );
    });
}

/// A settled tool call carrying a diff: its card derives collapsed, so
/// nothing about it is on screen until a fold decision opens it.
fn settled_tool_with_diff(id: &str) -> ChatItem {
    let ChatItem::ToolCall(mut tc) =
        tool_call_with(vec![raw("out")], vec![diff("a.java", "new\n")])
    else {
        unreachable!("tool_call_with builds a ToolCall")
    };
    tc.id = id.to_string();
    tc.kind = ToolKindView::Edit;
    tc.status = ToolStatusView::Completed;
    ChatItem::ToolCall(tc)
}

/// Every card's tool rule set to `expanded`, on both turn positions so the
/// single-item fixture matches whichever one it derives.
fn all_tools_expanded() -> FoldMode {
    FoldMode::from_tokens(["auto", "past.tool=expanded", "last.tool=expanded"])
}

/// A fold *mode* edit moves the derived default for every card at once, so
/// it opens card bodies exactly like expand-all does — and owes the same
/// embed pass. Without it the renderer, which only ever *reads* the embed
/// caches, falls back to the inline per-line diff walk: `+`/`-` prefixed
/// lines with no gutter and no syntax colour, for as long as no ACP event
/// happens to arrive and reconcile on its own.
#[gpui::test]
fn a_fold_mode_edit_builds_the_embeds_it_reveals(cx: &mut TestAppContext) {
    let window = make_test_view(cx);
    let view = window.root(cx).expect("the view is the window root");

    view.update(cx, |v, cx| {
        v.set_syntax_theme(SYNTAX_THEME);
        v.items = vec![settled_tool_with_diff("call_1")];
        let is_light = daruda_ui::theme::agent_chat_syntax_is_light(cx);
        v.reconcile_diff_editors(
            SYNTAX_THEME,
            is_light,
            &ReconcileScope::All,
            &mut by_handle(v),
            cx,
        );
    });
    view.read_with(cx, |v, _| {
        assert!(
            v.assets.diff_editors.is_empty(),
            "a collapsed card renders no body, so it needs no editor yet"
        );
    });

    window
        .update(cx, |v, window, cx| {
            v.set_fold_mode(all_tools_expanded(), window, cx);
        })
        .expect("the window is open");

    view.read_with(cx, |v, _| {
        assert!(
            v.assets.diff_editors.contains_key(KEY),
            "a mode edit that opens a card must build the embeds its body needs"
        );
    });
}

/// The reset button hands the axis back to config, which is just as much a
/// whole-matrix move as picking a preset is.
#[gpui::test]
fn resetting_the_fold_mode_builds_the_embeds_it_reveals(cx: &mut TestAppContext) {
    let window = make_test_view(cx);
    let view = window.root(cx).expect("the view is the window root");

    view.update(cx, |v, _cx| {
        v.set_syntax_theme(SYNTAX_THEME);
        v.defaults.fold_mode = all_tools_expanded();
        v.items = vec![settled_tool_with_diff("call_1")];
    });
    // Pin the pane to a matrix that keeps every card shut, so the reset is
    // what opens them.
    window
        .update(cx, |v, window, cx| {
            v.set_fold_mode(FoldPreset::Summary.mode(), window, cx);
        })
        .expect("the window is open");
    view.read_with(cx, |v, _| assert!(v.assets.diff_editors.is_empty()));

    window
        .update(cx, |v, window, cx| v.reset_fold_mode(window, cx))
        .expect("the window is open");

    view.read_with(cx, |v, _| {
        assert!(
            v.assets.diff_editors.contains_key(KEY),
            "following config again must build the embeds that default reveals"
        );
    });
}

/// The config-reload path: a pane open across an edit to the agent's
/// transcript defaults follows the new fold matrix, which opens card bodies
/// with no click and no event behind them. Unlike the two above this runs
/// outside any window update, so its reconcile resolves the window from the
/// stored handle.
#[gpui::test]
fn reseeded_defaults_build_the_embeds_they_reveal(cx: &mut TestAppContext) {
    use super::super::transcript_defaults::TranscriptDefaults;

    let window = make_test_view(cx);
    let view = window.root(cx).expect("the view is the window root");

    view.update(cx, |v, cx| {
        v.set_syntax_theme(SYNTAX_THEME);
        v.items = vec![settled_tool_with_diff("call_1")];
        let is_light = daruda_ui::theme::agent_chat_syntax_is_light(cx);
        v.reconcile_diff_editors(
            SYNTAX_THEME,
            is_light,
            &ReconcileScope::All,
            &mut by_handle(v),
            cx,
        );
    });
    view.read_with(cx, |v, _| assert!(v.assets.diff_editors.is_empty()));

    view.update(cx, |v, cx| {
        let defaults = TranscriptDefaults {
            fold_mode: all_tools_expanded(),
            ..v.defaults
        };
        v.reseed_transcript_defaults(&defaults, cx);
    });

    view.read_with(cx, |v, _| {
        assert!(
            v.assets.diff_editors.contains_key(KEY),
            "a reseeded default that opens a card must build its embeds too"
        );
    });
}

/// The property the fix is for: live editors — and so live focus handles —
/// track what is on screen, not how long the conversation has run.
#[gpui::test]
fn live_embed_editors_stay_bounded_as_the_conversation_grows(cx: &mut TestAppContext) {
    use super::super::fold::FoldKey;

    let window = make_test_view(cx);
    let view = window.root(cx).expect("the view is the window root");

    let mut items: Vec<ChatItem> = (0..200)
        .map(|i| settled_tool(&format!("call_{i}"), vec![raw("settled output")]))
        .collect();
    items.push(tool_named("call_live", vec![raw("streaming")]));
    view.update(cx, |v, cx| {
        v.items = items;
        v.fold.set_all([FoldKey::Tool("call_live".into())], true);
        v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
    });
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.assets.output_editors.len(),
            1,
            "only the opened card's body is on screen"
        );
    });
}

fn tool_named(id: &str, output: Vec<ToolOutputBlock>) -> ChatItem {
    let ChatItem::ToolCall(mut tc) = tool_call(output) else {
        unreachable!("tool_call builds a ToolCall")
    };
    tc.id = id.to_string();
    ChatItem::ToolCall(tc)
}

/// A `ToolCallUpdate` names the one call it replaced, so a reconcile pass
/// scoped to it must not touch — least of all evict — any other call's cached
/// editors. Getting this wrong is the failure mode scoping introduces: the
/// full pass derived staleness from a whole-conversation `live` set, and a
/// scoped pass that kept doing so would drop every other tool's editor.
#[gpui::test]
fn a_scoped_reconcile_leaves_other_tools_editors_alone(cx: &mut TestAppContext) {
    let window = make_tools_expanded_view(cx);
    let view = window.root(cx).expect("the view is the window root");

    view.update(cx, |v, cx| {
        v.items = vec![
            tool_named("call_a", vec![raw("a output")]),
            tool_named("call_b", vec![raw("b output")]),
        ];
        v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
    });
    let a_editor = view.read_with(cx, |v, _| {
        v.assets
            .output_editors
            .get("call_a#0")
            .map(|e| e.entity_id())
            .expect("the full pass builds both editors")
    });

    // Only call_b streamed more output.
    view.update(cx, |v, cx| {
        v.items = vec![
            tool_named("call_a", vec![raw("a output")]),
            tool_named("call_b", vec![raw("b output\nmore")]),
        ];
        v.reconcile_output_editors(
            &ReconcileScope::Tool("call_b".into()),
            &mut by_handle(v),
            cx,
        );
    });
    view.read_with(cx, |v, cx| {
        assert_eq!(
            v.assets
                .output_editors
                .get("call_a#0")
                .map(|e| e.entity_id()),
            Some(a_editor),
            "an untouched tool keeps its editor entity"
        );
        assert_eq!(
            v.assets.output_editors["call_b#0"].read(cx).value(),
            "b output\nmore",
            "the scoped tool still rebuilds on grown content"
        );
    });
}

/// Scoping narrows *which* calls are visited, not the eviction guarantee for
/// the call being visited: a shrunken `output` vec must still drop the keys
/// its indexes no longer reach.
#[gpui::test]
fn a_scoped_reconcile_still_evicts_its_own_vanished_keys(cx: &mut TestAppContext) {
    let window = make_tools_expanded_view(cx);
    let view = window.root(cx).expect("the view is the window root");

    view.update(cx, |v, cx| {
        v.items = vec![
            tool_named("call_a", vec![raw("keep me")]),
            tool_named("call_b", vec![raw("first"), raw("second")]),
        ];
        v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
    });
    view.read_with(cx, |v, _| assert_eq!(v.assets.output_editors.len(), 3));

    view.update(cx, |v, cx| {
        v.items = vec![
            tool_named("call_a", vec![raw("keep me")]),
            tool_named("call_b", vec![raw("first")]),
        ];
        v.reconcile_output_editors(
            &ReconcileScope::Tool("call_b".into()),
            &mut by_handle(v),
            cx,
        );
    });
    view.read_with(cx, |v, _| {
        assert!(
            !v.assets.output_editors.contains_key("call_b#1"),
            "an index the shrunken output vec no longer reaches is dropped"
        );
        assert!(v.assets.output_editors.contains_key("call_b#0"));
        assert!(
            v.assets.output_editors.contains_key("call_a#0"),
            "the untouched tool is unaffected by the eviction"
        );
    });
}

/// `All` is what a wholesale `items` replacement (connect / load catch-up)
/// needs: a tool that left the conversation entirely takes its editors with
/// it. No scoped pass can see that, which is why the variant exists.
#[gpui::test]
fn the_all_scope_evicts_a_tool_that_left_the_conversation(cx: &mut TestAppContext) {
    let window = make_tools_expanded_view(cx);
    let view = window.root(cx).expect("the view is the window root");

    view.update(cx, |v, cx| {
        v.items = vec![
            tool_named("call_a", vec![raw("a")]),
            tool_named("call_b", vec![raw("b")]),
        ];
        v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
    });
    view.update(cx, |v, cx| {
        v.items = vec![tool_named("call_a", vec![raw("a")])];
        v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
    });
    view.read_with(cx, |v, _| {
        assert!(v.assets.output_editors.contains_key("call_a#0"));
        assert!(
            !v.assets.output_editors.contains_key("call_b#0"),
            "a vanished tool's editors are dropped by the full pass"
        );
    });
}

/// The cost this scoping exists to remove: the full pass fingerprints every
/// diff's whole `old_text` + `new_text` in the conversation, so a streamed
/// `ToolCallUpdate` re-hashed every diff ever produced — quadratic over a
/// turn. A scoped pass must touch one call's content, not all of it.
#[gpui::test]
fn a_scoped_diff_reconcile_does_not_rehash_the_whole_conversation(cx: &mut TestAppContext) {
    let window = make_tools_expanded_view(cx);
    let view = window.root(cx).expect("the view is the window root");

    // 200 settled calls, ~128 KiB of diff text each: ~25 MiB the full pass
    // re-fingerprints per event.
    let body = "fn main() { let x = 1; }\n".repeat(5_000);
    let seeded: Vec<ChatItem> = (0..200)
        .map(|i| {
            let ChatItem::ToolCall(mut tc) = diff_tool_call(vec![diff("a.rs", &body)]) else {
                unreachable!("diff_tool_call builds a ToolCall")
            };
            tc.id = format!("call_{i}");
            ChatItem::ToolCall(tc)
        })
        .collect();
    // The pass will fingerprint each diff against this same palette, so the
    // pre-seed below has to use it too or every key would look changed.
    let theme = cx.update(|cx| {
        let surface = daruda_ui::theme::PaneSurfaceTokens::agent_chat(cx);
        let colors = DiffColors::from_agent_chat_surface(
            cx.global::<daruda_ui::theme::DarudaTheme>(),
            surface,
        );
        super::diff_theme_fingerprint(SYNTAX_THEME, false, &colors)
    });
    view.update(cx, |v, _| {
        v.items = seeded;
        // Pre-seed matching fingerprints so neither pass builds an editor.
        // What is left is exactly the change-detection walk — the cost
        // scoping exists to remove, isolated from the one legitimate rebuild
        // a real update would also do.
        for item in &v.items {
            let ChatItem::ToolCall(tc) = item else {
                unreachable!("every seeded item is a tool call")
            };
            for (di, d) in tc.diffs.iter().enumerate() {
                v.assets.diff_editor_sources.insert(
                    super::diff_editor_key(&tc.id, di),
                    super::diff_build_fingerprint(d, theme),
                );
            }
        }
    });

    let time = |cx: &mut TestAppContext, scope: ReconcileScope| {
        let started = std::time::Instant::now();
        view.update(cx, |v, cx| {
            v.reconcile_diff_editors(SYNTAX_THEME, false, &scope, &mut by_handle(v), cx);
        });
        started.elapsed()
    };
    let full = time(cx, ReconcileScope::All);
    let scoped = time(cx, ReconcileScope::Tool("call_0".into()));
    assert!(
        scoped * 10 < full,
        "scoped {scoped:?} vs full {full:?} — the scoped pass is still \
         walking the whole conversation"
    );
}

/// The reconciler owns the whole editor lifecycle: one entity per
/// qualifying output block, reused while the content is unchanged, rebuilt
/// when a streamed output grows, and dropped once the block (or its index)
/// is gone. Driven the way an ACP event drives it — outside any window
/// update, resolving the window from the stored handle. The fold-click path
/// (inside the cycle) is covered by
/// `folding_a_tool_card_materializes_and_releases_its_editors`.
#[gpui::test]
fn output_editors_are_built_reused_rebuilt_and_dropped(cx: &mut TestAppContext) {
    let window = make_tools_expanded_view(cx);
    let view = window.root(cx).expect("the view is the window root");

    let reconcile_with = |cx: &mut TestAppContext, output: Vec<ToolOutputBlock>| {
        view.update(cx, |v, cx| {
            v.items = vec![tool_call(output)];
            v.reconcile_output_editors(&ReconcileScope::All, &mut by_handle(v), cx);
        });
    };
    let editor_id = |cx: &mut TestAppContext| {
        view.read_with(cx, |v, _| {
            v.assets.output_editors.get(KEY).map(|e| e.entity_id())
        })
    };
    let editor_text = |cx: &mut TestAppContext| {
        view.read_with(cx, |v, cx| {
            v.assets.output_editors[KEY].read(cx).value().to_string()
        })
    };

    reconcile_with(cx, vec![raw("line 1\nline 2")]);
    let built = editor_id(cx).expect("a verbatim block gets an editor");
    assert_eq!(editor_text(cx), "line 1\nline 2");
    view.read_with(cx, |v, _| {
        assert_eq!(v.assets.output_editors.len(), 1);
        assert!(v.assets.output_editor_sources.contains_key(KEY));
    });

    reconcile_with(cx, vec![raw("line 1\nline 2")]);
    assert_eq!(
        editor_id(cx),
        Some(built),
        "unchanged content reuses the cached editor"
    );

    reconcile_with(cx, vec![raw("line 1\nline 2\nline 3")]);
    assert_ne!(
        editor_id(cx),
        Some(built),
        "a grown output must not stay frozen on the partial snapshot"
    );
    assert_eq!(editor_text(cx), "line 1\nline 2\nline 3");

    // A block that stopped qualifying drops its editor so the markdown /
    // monospace fallback renders again.
    reconcile_with(
        cx,
        vec![ToolOutputBlock::Image {
            data: "AAAA".to_string(),
            mime: "image/png".to_string(),
        }],
    );
    view.read_with(cx, |v, _| {
        assert!(v.assets.output_editors.is_empty());
        assert!(v.assets.output_editor_sources.is_empty());
    });

    // An index the replacement output vec no longer reaches is dropped too.
    reconcile_with(cx, vec![raw("back again")]);
    assert!(editor_id(cx).is_some());
    reconcile_with(cx, Vec::new());
    view.read_with(cx, |v, _| {
        assert!(
            v.assets.output_editors.is_empty() && v.assets.output_editor_sources.is_empty(),
            "a shrunken output vec must not leak editors under stale indexes"
        );
    });
}

/// The diff reconciler drops by the same rule: an entry whose diff the pass
/// no longer reaches — a shrunken `diffs` vec, or a tool call gone from
/// `items` — leaves all three parallel maps, while a diff that is merely
/// unchanged keeps the editor it already has.
#[gpui::test]
fn diff_editors_are_dropped_when_their_diff_is_gone(cx: &mut TestAppContext) {
    let window = make_tools_expanded_view(cx);
    let view = window.root(cx).expect("the view is the window root");

    let reconcile_with = |cx: &mut TestAppContext, items: Vec<ChatItem>| {
        view.update(cx, |v, cx| {
            v.items = items;
            v.reconcile_diff_editors(
                SYNTAX_THEME,
                false,
                &ReconcileScope::All,
                &mut by_handle(v),
                cx,
            );
        });
    };
    let cached = |cx: &mut TestAppContext, key: &str| {
        let key = key.to_string();
        view.read_with(cx, |v, _| {
            (
                v.assets.diff_editors.get(&key).map(|e| e.entity_id()),
                v.assets.diff_stats.contains_key(&key),
                v.assets.diff_editor_sources.contains_key(&key),
            )
        })
    };

    reconcile_with(
        cx,
        vec![diff_tool_call(vec![
            diff("a.rs", "first\n"),
            diff("b.rs", "second\n"),
        ])],
    );
    let first = cached(cx, "call_1#0");
    assert!(
        first.0.is_some() && first.1 && first.2,
        "each diff gets an editor plus its stat and fingerprint"
    );
    assert!(cached(cx, "call_1#1").0.is_some());

    // The second diff is gone from the replacement vec, so nothing keyed to
    // it may survive — but the first is unchanged and keeps its editor.
    reconcile_with(cx, vec![diff_tool_call(vec![diff("a.rs", "first\n")])]);
    assert_eq!(
        cached(cx, "call_1#0"),
        first,
        "an unchanged diff must keep the editor it already built"
    );
    assert_eq!(
        cached(cx, "call_1#1"),
        (None, false, false),
        "a shrunken diffs vec must not leak an editor, stat or fingerprint"
    );

    // The whole tool call left the conversation.
    reconcile_with(cx, Vec::new());
    view.read_with(cx, |v, _| {
        assert!(
            v.assets.diff_editors.is_empty()
                && v.assets.diff_stats.is_empty()
                && v.assets.diff_editor_sources.is_empty(),
            "a tool call that left `items` must not leak its diff entries"
        );
    });
}

/// A mermaid fence inside a subagent-launch's `prompt` (rendered as
/// markdown in the "Instructions" section, `render/tool.rs`) must be
/// scanned the same as one in assistant text or tool output — proven
/// end-to-end through the real reconciler, not just the pure
/// `chat_item_mermaid_texts` helper it calls. Checked via
/// `mermaid_inflight` (set synchronously, before the background
/// rasterization task is even spawned) rather than `mermaid_images`, so
/// the test doesn't depend on the real rasterizer completing.
#[gpui::test]
fn reconcile_mermaid_scans_a_subagent_prompt(cx: &mut TestAppContext) {
    let window = make_test_view(cx);
    let view = window.root(cx).expect("the view is the window root");

    let source = "flowchart TD\n  A-->B";
    view.update(cx, |v, cx| {
        v.items = vec![ChatItem::ToolCall(ToolCallItem {
            id: "call_1".to_string(),
            title: "Implement Task 2".to_string(),
            kind: ToolKindView::Think,
            tool_name: None,
            status: ToolStatusView::InProgress,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: Some(serde_json::json!({
                "subagent_type": "general-purpose",
                "prompt": format!("Draw it:\n```mermaid\n{source}\n```"),
            })),
            locations: Vec::new(),
            parent_tool_id: None,
            exit: None,
        })];
        v.reconcile_mermaid(false, cx);
    });

    let key = mermaid_key(source, false);
    view.read_with(cx, |v, _| {
        assert!(
            v.assets.mermaid_inflight.contains(&key),
            "the prompt's mermaid fence must be picked up for rasterization"
        );
    });
}
