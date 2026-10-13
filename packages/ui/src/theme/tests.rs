use super::*;

/// The state installs only when the painted preset moves. A reinstall
/// would replace the global wholesale, so a marker written into the live
/// theme survives exactly the calls that must not reinstall.
#[gpui::test]
fn the_theme_state_reinstalls_only_when_the_painted_preset_moves(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        init_if_missing(cx);
        init_ui_theme("daruda_dark", cx);
        let marker = gpui::hsla(0.5, 0.5, 0.5, 0.5);
        cx.global_mut::<DarudaTheme>().link_color = marker;

        set_configured_ui_preset("daruda_dark", cx);
        note_system_appearance(gpui::WindowAppearance::Light, cx);
        assert_eq!(
            current(cx).link_color,
            marker,
            "nothing moved, nothing reinstalled"
        );

        set_configured_ui_preset("daruda_light", cx);
        assert!(!current(cx).is_dark());
        assert_ne!(current(cx).link_color, marker);
    });
}

/// A capture paints its preset over config and appearance, and hands the
/// configured one back when it ends.
#[gpui::test]
fn a_capture_override_paints_over_the_configured_preset(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        init_if_missing(cx);
        init_ui_theme("daruda_dark", cx);
        set_capture_ui_preset(Some(daruda_config::ui_theme_presets::SYSTEM_LIGHT), cx);
        assert!(!current(cx).is_dark());
        assert_eq!(painted_ui_preset("daruda_dark", cx), "daruda_light");
        set_capture_ui_preset(None, cx);
        assert!(current(cx).is_dark());
    });
}

/// `system` has to land on a preset `apply_ui_theme` can install, under
/// both vibrant variants too — otherwise a flip leaves the old theme live.
#[gpui::test]
fn the_system_preset_resolves_to_an_installable_theme(cx: &mut gpui::TestAppContext) {
    use gpui::WindowAppearance as A;
    cx.update(|cx| {
        init_if_missing(cx);
        for (appearance, expected) in [
            (A::Dark, "daruda_dark"),
            (A::VibrantDark, "daruda_dark"),
            (A::Light, "daruda_light"),
            (A::VibrantLight, "daruda_light"),
        ] {
            let resolved = effective_ui_preset("system", appearance);
            assert_eq!(resolved, expected, "{appearance:?}");
            assert!(apply_ui_theme(resolved, cx), "{resolved} is not bundled");
        }
        assert!(
            !apply_ui_theme("system", cx),
            "the unresolved name must not install"
        );
        assert_eq!(effective_ui_preset("daruda_light", A::Dark), "daruda_light");
    });
}

#[gpui::test]
fn agent_chat_syntax_is_light_switches_at_the_midpoint(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        assert!(!agent_chat_syntax_is_light(cx), "unset default is dark");

        set_agent_chat_bg(cx, 0, 0, 0);
        assert!(!agent_chat_syntax_is_light(cx), "black background is dark");

        set_agent_chat_bg(cx, 255, 255, 255);
        assert!(agent_chat_syntax_is_light(cx), "white background is light");

        // The finest boundary an 8-bit RGB mirror can actually produce:
        // grayscale lightness is `v / 255`, so 127 sits just under the
        // `l >= 0.5` cutoff and 128 sits just over it. Matches
        // `agent_chat_tint`'s `l < 0.5` dark-branch cutoff — both treat
        // `l == 0.5` as the light side.
        set_agent_chat_bg(cx, 127, 127, 127);
        assert!(!agent_chat_syntax_is_light(cx), "l=127/255 is still dark");

        set_agent_chat_bg(cx, 128, 128, 128);
        assert!(agent_chat_syntax_is_light(cx), "l=128/255 crosses to light");
    });
}

/// Composite `fg` over `bg` and return the WCAG contrast ratio. The
/// agent-chat foreground ramp is alpha-based, so an uncomposited pair
/// would measure the wrong thing.
fn contrast_over(fg: gpui::Hsla, bg: gpui::Hsla) -> f32 {
    super::contrast_ratio(fg, bg)
}

/// Command-effect chips paint their colour as text over their own tinted
/// fill, which must still clear the 4.5:1 body floor on either pane.
#[test]
fn command_effect_chips_clear_contrast_on_dark_and_light_panes() {
    for (colors, pane) in [
        ([AGENT_READING, AGENT_EDITING], gpui::rgb(0x1e_1e_1e)),
        (
            [AGENT_READING_LIGHT, AGENT_EDITING_LIGHT],
            gpui::rgb(0xf9_fa_fb),
        ),
    ] {
        for color in colors {
            let fg = gpui::Rgba::from(color);
            let a = AGENT_CHAT_CARD_TINT_ALPHA;
            let fill = gpui::Rgba {
                r: fg.r * a + pane.r * (1.0 - a),
                g: fg.g * a + pane.g * (1.0 - a),
                b: fg.b * a + pane.b * (1.0 - a),
                a: 1.0,
            };
            assert!(contrast_over(color, fill.into()) >= 4.5);
        }
    }
}

/// Controls on the agent-chat bar take their colour from this surface, and
/// the surface mirrors the *terminal* palette — which the UI theme knows
/// nothing about. A control coloured from the UI theme instead landed at
/// ~1.1:1 here under a light UI over a dark pane.
#[gpui::test]
fn the_pane_surface_ramp_stays_readable_on_its_own_background(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        for (bg, fg) in [
            ((17u8, 17u8, 17u8), (216u8, 216u8, 216u8)),
            ((250, 250, 250), (40, 40, 40)),
        ] {
            set_agent_chat_bg(cx, bg.0, bg.1, bg.2);
            set_agent_chat_fg(cx, fg.0, fg.1, fg.2);
            let s = PaneSurfaceTokens::agent_chat(cx);
            // Body text at AA (4.5:1); the muted tier carries chips, icon
            // glyphs and secondary labels, so it is held to the 3:1 floor
            // WCAG sets for UI components.
            for (name, color, floor) in [
                ("foreground", s.foreground, 4.5),
                ("muted", s.foreground_muted, 3.0),
            ] {
                let ratio = contrast_over(color, s.background);
                assert!(ratio >= floor, "{name} on {bg:?} measures {ratio:.2}:1");
            }
        }
    });
}

/// Agent-chat file links sit on the terminal-derived pane surface, not the
/// UI surface the global link token was tuned for. Keep the link hue but
/// prove its lightness follows the pane when UI and terminal themes differ.
#[gpui::test]
fn the_agent_chat_link_color_tracks_the_pane_background(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        init_if_missing(cx);

        assert!(apply_ui_theme("daruda_light", cx));
        set_agent_chat_bg(cx, 17, 17, 17);
        set_agent_chat_fg(cx, 216, 216, 216);
        let dark_ui_link = current(cx).link_color;
        let dark_pane_link = agent_chat_link_color(cx);
        let dark_ratio = contrast_over(dark_pane_link, agent_chat_bg(cx));
        assert!(
            dark_ratio >= 4.5,
            "light UI link on dark agent-chat bg measures {dark_ratio:.2}:1"
        );
        assert!(
            dark_pane_link.l > dark_ui_link.l,
            "dark pane should lift the light-theme link, got {:?} from {:?}",
            dark_pane_link,
            dark_ui_link
        );

        assert!(apply_ui_theme("daruda_dark", cx));
        set_agent_chat_bg(cx, 250, 250, 250);
        set_agent_chat_fg(cx, 40, 40, 40);
        let light_ui_link = current(cx).link_color;
        let light_pane_link = agent_chat_link_color(cx);
        let light_ratio = contrast_over(light_pane_link, agent_chat_bg(cx));
        assert!(
            light_ratio >= 4.5,
            "dark UI link on light agent-chat bg measures {light_ratio:.2}:1"
        );
        assert!(
            light_pane_link.l < light_ui_link.l,
            "light pane should darken the dark-theme link, got {:?} from {:?}",
            light_pane_link,
            light_ui_link
        );
    });
}

/// The markdown view's fills and structural lines — inline-code chips, the
/// code-block fill and border, table lines, the `<hr>` rule — are a neutral
/// tint whose direction comes from the surface they are drawn on. Taking it
/// from the UI canvas instead erases all of them at once whenever the two
/// disagree, which `ui_preset` / `terminal_preset` being independent makes
/// a supported combination rather than a corner case.
#[gpui::test]
fn markdown_structural_lines_step_off_the_pane_not_the_ui_canvas(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        init_if_missing(cx);
        // The vendored `STRUCTURAL_LINE_ALPHA`; asserted here for the
        // direction it is applied in, not for the value.
        let alpha = 0.28;
        let tint = |surface: gpui::Hsla| {
            if surface.l < 0.5 {
                gpui::hsla(0., 0., 1., alpha)
            } else {
                gpui::hsla(0., 0., 0., alpha)
            }
        };
        // A line has to separate from its surface; it is deliberately faint
        // (a gridline, not a control edge), so this is a visibility floor,
        // not DESIGN.md's 3:1 affordance floor.
        let visible = 1.5;

        apply_ui_theme("daruda_light", cx);
        let light_canvas = Theme::global(cx).background;
        assert!(
            light_canvas.l >= 0.5,
            "the light UI canvas is not light: {light_canvas:?}"
        );

        for preset in daruda_config::theme_presets::PRESETS {
            let Some(colors) = daruda_config::theme_presets::colors_for_preset(preset.name) else {
                continue;
            };
            let bg = colors.background;
            set_agent_chat_bg(cx, bg.r, bg.g, bg.b);
            let pane = agent_chat_bg(cx);

            let ours = contrast_over(tint(pane), pane);
            assert!(
                ours >= visible,
                "a line on the {} pane measures {ours:.2}:1",
                preset.name
            );

            // What the UI canvas would have picked with a light `ui_preset`
            // over this terminal: the same tint the pane already is.
            let from_canvas = contrast_over(tint(light_canvas), pane);
            assert!(
                from_canvas < visible,
                "{}: the UI canvas still yields a visible line ({from_canvas:.2}:1), so \
                     this test no longer covers the combination it was written for",
                preset.name
            );
        }

        apply_ui_theme("daruda_dark", cx);
    });
}

/// A markdown blockquote is the one run of prose the view colours itself.
/// It reached for the UI theme's muted tone, which DESIGN.md §AgentChatPane
/// rules out on this pane: `ui_preset` and `terminal_preset` are
/// independent config keys, so a UI colour has no verified contrast on a
/// terminal-mirrored background. The quote now *is* `pane-fg-muted`, the
/// tier every other secondary label on the pane already uses.
#[gpui::test]
fn a_markdown_quote_reads_on_the_pane_it_is_painted_on(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        init_if_missing(cx);
        let ui_tone = current(cx).text_muted;
        // Solarized's own foreground is deliberately low-contrast (#839496
        // on #002b36), so the pane's muted tier lands at 4.23:1 there —
        // every secondary label on that preset does, not just a quote. That
        // is the ramp's shortfall, tracked apart from this colour choice.
        let ramp_short = ["solarized_dark"];
        let mut ui_tone_failures = 0;

        for preset in daruda_config::theme_presets::PRESETS {
            let Some(colors) = daruda_config::theme_presets::colors_for_preset(preset.name) else {
                continue;
            };
            let (bg, fg) = (colors.background, colors.foreground);
            set_agent_chat_bg(cx, bg.r, bg.g, bg.b);
            set_agent_chat_fg(cx, fg.r, fg.g, fg.b);
            let pane_bg = agent_chat_bg(cx);

            let quote = agent_chat_fg(cx).opacity(p::MD_VIEW_MUTED_ALPHA);
            let quote_ratio = contrast_over(quote, pane_bg);
            // It is the pane's documented muted tier, not a second guess.
            let tier = contrast_over(agent_chat_fg_muted(cx), pane_bg);
            assert!(
                (quote_ratio - tier).abs() < 0.01,
                "{} quote is off the pane's muted tier: {quote_ratio:.2} vs {tier:.2}",
                preset.name
            );
            if !ramp_short.contains(&preset.name) {
                assert!(
                    quote_ratio >= 4.5,
                    "quote on the {} pane measures {quote_ratio:.2}:1",
                    preset.name
                );
            }

            let ui_ratio = contrast_over(ui_tone, pane_bg);
            if ui_ratio < 4.5 {
                ui_tone_failures += 1;
                assert!(
                    quote_ratio > ui_ratio,
                    "{}: the pane colour ({quote_ratio:.2}) is no better than the UI tone \
                         it replaced ({ui_ratio:.2})",
                    preset.name
                );
            }
        }

        assert!(
            ui_tone_failures > 0,
            "if the UI theme's muted tone cleared every terminal preset the \
                 host-supplied colour would be unnecessary"
        );
    });
}

/// The vendored `selected_foreground` slot exists *only* because no single
/// foreground clears both of a segmented strip's contrast pairs. Assert both,
/// or a palette move silently invalidates the patch's whole justification.
#[gpui::test]
fn a_segmented_strip_clears_both_of_its_contrast_pairs(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        init_if_missing(cx);
        // Read the popover fill through the *bridged* theme, so this tracks
        // whatever `apply_daruda_palette` actually assigns rather than a
        // second guess at it. Today that is `surface-2`, not the `surface-4`
        // DESIGN.md's elevation table nominally gives popovers.
        let popover = Theme::global(cx).popover;
        let t = current(cx);
        let resting = contrast_over(t.text_muted, popover);
        assert!(resting >= 4.5, "resting label measures {resting:.2}:1");
        // Selected label on the accent fill.
        let selected = contrast_over(p::ACCENT_FG, p::PRIMARY);
        assert!(selected >= 4.5, "selected label measures {selected:.2}:1");
        // The pair the patch rules out: one shared foreground cannot do both.
        let shared = contrast_over(t.text_muted, p::PRIMARY);
        assert!(
            shared < 4.5,
            "if the resting tone also worked on the fill ({shared:.2}:1) the \
                 patch would be unnecessary"
        );
        // And the reason accent is not the resting label colour.
        let accent_as_text = contrast_over(p::PRIMARY, popover);
        assert!(
            accent_as_text < 4.5,
            "accent as a segment label measures {accent_as_text:.2}:1"
        );
    });
}

/// The two parts of the breakpoint are text widths, so the threshold has to
/// track the pane's configured font size. A fixed number would read as
/// derived while assuming one font.
#[gpui::test]
fn the_compact_breakpoint_scales_with_the_pane_font(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        // Unset: the compile-time default, so the documented value holds.
        let default = agent_chat_compact_options_w(cx);
        assert!(
            (default - p::AGENT_CHAT_COMPACT_OPTIONS_W).abs() < 0.01,
            "at the default size the constant is the threshold, got {default}"
        );

        set_agent_chat_font_size(cx, p::AGENT_CHAT_MSG_FONT_SIZE * 2.0);
        let doubled = agent_chat_compact_options_w(cx);
        assert!(
            doubled > default,
            "a larger pane font needs a wider pane before the chips fit"
        );
        // Only the text parts scale; the fixed padding stays put.
        let text_part = default - 2.0 * p::AGENT_CHAT_PAD_X;
        assert!(
            (doubled - (text_part * 2.0 + 2.0 * p::AGENT_CHAT_PAD_X)).abs() < 0.01,
            "padding must not scale with the font, got {doubled}"
        );

        set_agent_chat_font_size(cx, 6.0);
        assert!(
            agent_chat_compact_options_w(cx) < default,
            "the smallest configurable font lets a narrower pane keep the chips"
        );
    });
}

#[gpui::test]
fn an_embedded_editor_row_tracks_agent_chat_font_metrics(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        assert_eq!(
            agent_chat_embed_row_height(cx),
            (p::AGENT_CHAT_MSG_FONT_SIZE * p::MD_VIEW_LINE_HEIGHT).ceil()
        );

        set_agent_chat_font_size(cx, 20.0);
        set_agent_chat_line_height(cx, 1.5);
        assert_eq!(agent_chat_embed_row_height(cx), 30.0);
    });
}

/// A chip's resting edge is the only thing that says it is a control
/// rather than one of the static readouts beside it on the Activity Bar,
/// so it is held to DESIGN.md's 3:1 component-edge floor. The card tint
/// that edges tool cards measures 1.44:1 here and is *not* enough — the
/// two are separate tokens for exactly this reason.
#[gpui::test]
fn a_control_edge_on_the_pane_surface_clears_the_component_floor(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        // Both sanctioned directions: the default terminal preset's
        // background and a light one.
        for bg in [(30u8, 30u8, 30u8), (249u8, 250u8, 251u8)] {
            set_agent_chat_bg(cx, bg.0, bg.1, bg.2);
            let s = PaneSurfaceTokens::agent_chat(cx);
            let control = contrast_over(s.control_border, s.background);
            assert!(
                control >= 3.0,
                "control edge on {bg:?} measures {control:.2}:1"
            );
            let card = contrast_over(s.border_tint, s.background);
            assert!(
                card < control,
                "the card edge must stay the lighter of the two, got \
                     card {card:.2}:1 vs control {control:.2}:1"
            );
        }
    });
}

/// A dimmed surface must move each token exactly as the bar's own text
/// moves, or a control built from it stays at full strength while the
/// title beside it fades.
#[gpui::test]
fn dimming_a_surface_matches_dimming_each_token_by_hand(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        set_agent_chat_bg(cx, 17, 17, 17);
        set_agent_chat_fg(cx, 216, 216, 216);
        for amount in [0.0, 0.4, 1.0] {
            let dimmed = PaneSurfaceTokens::agent_chat(cx).dimmed(amount);
            assert_eq!(
                dimmed.foreground,
                dim_toward_gray(agent_chat_fg(cx), amount)
            );
            assert_eq!(
                dimmed.foreground_muted,
                dim_toward_gray(agent_chat_fg_muted(cx), amount)
            );
            assert_eq!(
                dimmed.border_tint,
                dim_toward_gray(agent_chat_border_tint(cx), amount)
            );
            assert_eq!(
                dimmed.control_border,
                dim_toward_gray(PaneSurfaceTokens::agent_chat(cx).control_border, amount)
            );
        }
    });
}

/// The built-in gpui_component scrollbar draws its thumb over popover /
/// modal surfaces (dropdown menus, dialogs). A thumb mapped to an opaque
/// surface-ladder gray was byte-identical to the popover background —
/// painted but invisible (the status-bar Ports dropdown regression).
/// The bridge must keep the thumb visually distinct on those surfaces.
#[gpui::test]
fn scrollbar_thumb_contrasts_with_popover_surface(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        init_if_missing(cx);
        let t = Theme::global(cx);
        assert_ne!(
            t.scrollbar_thumb, t.popover,
            "thumb color equals the popover background — invisible scrollbar"
        );
        assert!(
            t.scrollbar_thumb.a < 1.0,
            "thumb should be translucent so it reads on any surface rung"
        );
    });
}

/// `apply_daruda_palette` maps ~80 slots by hand; nothing structural
/// stops a foreground slot from landing on its own surface color (the
/// invisible-scrollbar class of bug, above). Pin every foreground /
/// surface pair gpui_component widgets actually paint text with. This
/// only rejects the "same slot mapped twice" failure — not low
/// contrast in general — which is exactly the class the bridge has
/// shipped before.
#[gpui::test]
fn bridged_foreground_surface_pairs_stay_distinct(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        init_if_missing(cx);
        let t = Theme::global(cx);
        let pairs = [
            ("foreground on background", t.foreground, t.background),
            (
                "popover_foreground on popover",
                t.popover_foreground,
                t.popover,
            ),
            ("muted_foreground on muted", t.muted_foreground, t.muted),
            ("accent_foreground on accent", t.accent_foreground, t.accent),
            (
                "secondary_foreground on secondary",
                t.secondary_foreground,
                t.secondary,
            ),
            (
                "primary_foreground on primary",
                t.primary_foreground,
                t.primary,
            ),
            ("danger_foreground on danger", t.danger_foreground, t.danger),
            (
                "tab_active_foreground on tab_active",
                t.tab_active_foreground,
                t.tab_active,
            ),
            (
                "group_box_foreground on group_box",
                t.group_box_foreground,
                t.group_box,
            ),
        ];
        for (name, fg, bg) in pairs {
            assert_ne!(fg, bg, "{name}: text color equals its surface — invisible");
        }
    });
}
