//! Terminal-mirrored pane surfaces: the agent-chat / file-viewer background
//! and foreground globals and the tokens derived from them.

use super::*;

/// Terminal-theme background color, mirrored from config
/// `effective_colors().background` (i.e. `[colors]` + `[theme].terminal_preset`)
/// — the same value that fills the terminal pane. Lets the agent-chat pane
/// render on the terminal color theme rather than the UI theme's editor
/// surface, so the two panes match. Stored as raw RGB (the channel the
/// terminal fill uses) and converted to `Hsla` on read. Single update sites:
/// the construction seed in `globals` + the config-reload path.
#[derive(Clone, Copy, PartialEq)]
struct AgentChatBg {
    r: u8,
    g: u8,
    b: u8,
}

impl gpui::Global for AgentChatBg {}

/// Cohesive color set for pane-local surfaces that follow the terminal
/// color theme rather than the workspace chrome theme.
#[derive(Clone, Copy)]
pub struct PaneSurfaceTokens {
    pub background: gpui::Hsla,
    pub foreground: gpui::Hsla,
    pub foreground_muted: gpui::Hsla,
    pub foreground_subtle: gpui::Hsla,
    pub tint: gpui::Hsla,
    /// Fill a hovered control on this surface takes. Lighter than
    /// [`Self::tint`]: hover is a pointer response, not a card.
    pub hover_tint: gpui::Hsla,
    pub active_tint: gpui::Hsla,
    pub border_tint: gpui::Hsla,
    /// Resting edge for an interactive control on this surface — heavier than
    /// [`Self::border_tint`], which edges cards. See
    /// [`AGENT_CHAT_CONTROL_BORDER_ALPHA_ON_DARK`].
    pub control_border: gpui::Hsla,
    pub syntax_is_light: bool,
}

impl PaneSurfaceTokens {
    pub fn agent_chat(cx: &App) -> Self {
        Self::from_background_and_foreground(agent_chat_bg(cx), agent_chat_fg(cx))
    }

    pub fn file_viewer(cx: &App) -> Self {
        Self::from_background_and_foreground(file_viewer_pane_bg(cx), agent_chat_fg(cx))
    }

    /// The flow graph's canvas. Same source as the two above — a graph opened
    /// from a transcript should not land on a surface unrelated to the pane it
    /// came from.
    pub fn flow_graph(cx: &App) -> Self {
        Self::from_background_and_foreground(agent_chat_bg(cx), agent_chat_fg(cx))
    }

    /// The same surface dimmed for an inactive pane. Every token moves
    /// together, so a control built from these dims in step with the text
    /// beside it instead of staying at full strength.
    pub fn dimmed(self, amount: f32) -> Self {
        Self {
            background: dim_toward_gray(self.background, amount),
            foreground: dim_toward_gray(self.foreground, amount),
            foreground_muted: dim_toward_gray(self.foreground_muted, amount),
            foreground_subtle: dim_toward_gray(self.foreground_subtle, amount),
            tint: dim_toward_gray(self.tint, amount),
            hover_tint: dim_toward_gray(self.hover_tint, amount),
            active_tint: dim_toward_gray(self.active_tint, amount),
            border_tint: dim_toward_gray(self.border_tint, amount),
            control_border: dim_toward_gray(self.control_border, amount),
            syntax_is_light: self.syntax_is_light,
        }
    }

    fn from_background_and_foreground(background: gpui::Hsla, foreground: gpui::Hsla) -> Self {
        let overlay = neutral_overlay_for(background);
        // Darkening a near-white surface buys less contrast than lightening a
        // near-black one, so the control edge takes its alpha per direction.
        let control_alpha = if background.l < 0.5 {
            p::AGENT_CHAT_CONTROL_BORDER_ALPHA_ON_DARK
        } else {
            p::AGENT_CHAT_CONTROL_BORDER_ALPHA_ON_LIGHT
        };
        Self {
            background,
            foreground,
            foreground_muted: foreground.opacity(p::AGENT_CHAT_FG_MUTED_ALPHA),
            foreground_subtle: foreground.opacity(p::AGENT_CHAT_FG_SUBTLE_ALPHA),
            tint: p::with_alpha(overlay, p::AGENT_CHAT_CARD_TINT_ALPHA),
            hover_tint: p::with_alpha(overlay, p::AGENT_CHAT_HOVER_TINT_ALPHA),
            active_tint: p::with_alpha(overlay, p::AGENT_CHAT_CARD_BORDER_ALPHA),
            border_tint: p::with_alpha(overlay, p::AGENT_CHAT_CARD_BORDER_ALPHA),
            control_border: p::with_alpha(overlay, control_alpha),
            syntax_is_light: background.l >= 0.5,
        }
    }
}

/// Read the agent-chat background color. Defaults to [`BG_EDITOR`]
/// before any config load, so the pane renders on the editor surface until the
/// terminal color is mirrored in.
pub fn agent_chat_bg(cx: &App) -> gpui::Hsla {
    match cx.try_global::<AgentChatBg>() {
        Some(c) => gpui::Rgba {
            r: f32::from(c.r) / 255.0,
            g: f32::from(c.g) / 255.0,
            b: f32::from(c.b) / 255.0,
            a: 1.0,
        }
        .into(),
        None => p::BG_EDITOR,
    }
}

/// File-viewer pane background. This is intentionally scoped to the file
/// viewer's own body/editor/toolbar/search-panel surfaces, not workspace chrome
/// such as the tab strip. It follows the same terminal-mirrored colour source
/// as Agent Chat so file links opened from a transcript do not land on a
/// visually unrelated editor surface. Kept opaque because editor internals
/// repaint sub-areas such as the gutter on top of the pane surface.
pub fn file_viewer_pane_bg(cx: &App) -> gpui::Hsla {
    agent_chat_bg(cx)
}

/// Mirror the resolved terminal background color for the agent-chat render
/// path. What changed is not this global's to say — every window writes it;
/// see `workspace::config_sync::SharedSurface`.
pub fn set_agent_chat_bg(cx: &mut App, r: u8, g: u8, b: u8) {
    cx.set_global(AgentChatBg { r, g, b });
}

/// Agent-chat foreground color, mirrored from the terminal color theme's
/// `effective_colors().foreground` — the counterpart to [`AgentChatBg`]. The
/// pane already renders its background on the terminal color theme; mirroring
/// the terminal foreground for the pane's text (prose, headers, summaries)
/// keeps glyph color consistent with the background instead of pulling from the
/// UI theme's text ramp. Code-block syntax highlighting is intentionally
/// unaffected — that follows the selectable syntax palette. Stored as raw RGB;
/// converted to `Hsla` on read. Single update sites: the `globals` seed + the
/// config-reload path.
#[derive(Clone, Copy, PartialEq)]
struct AgentChatFg {
    r: u8,
    g: u8,
    b: u8,
}

impl gpui::Global for AgentChatFg {}

/// Read the agent-chat foreground color. Mirrors the terminal color theme's
/// foreground, then lifts its lightness by [`AGENT_CHAT_FG_BRIGHTEN`]
/// so chat text reads a touch brighter than the raw terminal glyph **without
/// touching the terminal itself** (the terminal renders from `terminal_config`,
/// not this global). The lift is scaled by how dark the pane background is
/// (`1 - bg.l`), so it brightens light-on-dark text (the common case) but
/// barely nudges dark-on-light text — where "brighter" would instead cut
/// contrast. Defaults to [`TEXT_PRIMARY`] (already bright)
/// before any config load.
pub fn agent_chat_fg(cx: &App) -> gpui::Hsla {
    match cx.try_global::<AgentChatFg>() {
        Some(c) => {
            let mut hsla: gpui::Hsla = gpui::Rgba {
                r: f32::from(c.r) / 255.0,
                g: f32::from(c.g) / 255.0,
                b: f32::from(c.b) / 255.0,
                a: 1.0,
            }
            .into();
            let bg_l = agent_chat_bg(cx).l;
            hsla.l = (hsla.l + p::AGENT_CHAT_FG_BRIGHTEN * (1.0 - bg_l)).min(1.0);
            hsla
        }
        None => p::TEXT_PRIMARY,
    }
}

/// Dimmed agent-chat foreground for secondary text (muted labels, status).
/// Derived by blending the terminal foreground toward the pane background via
/// alpha, so it dims relative to the *actual* terminal background on any theme
/// (a fixed UI-theme grey would be wrong on a light or strongly tinted
/// terminal). Replaces `DarudaTheme::text_muted` in the agent-chat pane.
pub fn agent_chat_fg_muted(cx: &App) -> gpui::Hsla {
    agent_chat_fg(cx).opacity(p::AGENT_CHAT_FG_MUTED_ALPHA)
}

/// Most-dimmed agent-chat foreground for tertiary text (collapsed summaries,
/// disclosure chevrons). The subtle step of the terminal-foreground ramp;
/// replaces `DarudaTheme::text_subtle` in the agent-chat pane. See
/// [`agent_chat_fg_muted`] for why this blends toward the background.
pub fn agent_chat_fg_subtle(cx: &App) -> gpui::Hsla {
    agent_chat_fg(cx).opacity(p::AGENT_CHAT_FG_SUBTLE_ALPHA)
}

/// Clickable text on the terminal-mirrored agent-chat surface. The UI theme's
/// link hue is retained, but its lightness is resolved against the pane
/// background because `ui_preset` and `terminal_preset` are independent.
pub fn agent_chat_link_color(cx: &App) -> gpui::Hsla {
    link_on(agent_chat_bg(cx), current(cx).link_color)
}

/// Clickable text on the file-viewer pane. The same surface as Agent Chat's
/// ([`file_viewer_pane_bg`] delegates to it), resolved by the same rule.
pub fn file_viewer_pane_link_color(cx: &App) -> gpui::Hsla {
    link_on(file_viewer_pane_bg(cx), current(cx).link_color)
}

/// Keep a link's hue, resolve its lightness against the surface it sits on.
fn link_on(background: gpui::Hsla, mut link: gpui::Hsla) -> gpui::Hsla {
    link.l = if background.l < 0.5 {
        link.l.max(p::PANE_LINK_MIN_L_ON_DARK)
    } else {
        link.l.min(p::PANE_LINK_MAX_L_ON_LIGHT)
    };
    link
}

/// File-viewer pane foreground. Mirrors Agent Chat's terminal-derived text
/// ramp so file-viewer chrome tracks live terminal theme changes.
pub fn file_viewer_pane_fg(cx: &App) -> gpui::Hsla {
    PaneSurfaceTokens::file_viewer(cx).foreground
}

/// Secondary file-viewer pane foreground for counters and inactive controls.
pub fn file_viewer_pane_fg_muted(cx: &App) -> gpui::Hsla {
    PaneSurfaceTokens::file_viewer(cx).foreground_muted
}

/// Tertiary file-viewer pane foreground for low-emphasis labels.
pub fn file_viewer_pane_fg_subtle(cx: &App) -> gpui::Hsla {
    PaneSurfaceTokens::file_viewer(cx).foreground_subtle
}

/// Mirror the resolved terminal foreground color for the agent-chat render
/// path. What changed is not this global's to say — every window writes it;
/// see `workspace::config_sync::SharedSurface`.
pub fn set_agent_chat_fg(cx: &mut App, r: u8, g: u8, b: u8) {
    cx.set_global(AgentChatFg { r, g, b });
}

fn neutral_overlay_for(bg: gpui::Hsla) -> gpui::Hsla {
    if bg.l < 0.5 {
        p::OVERLAY_WHITE
    } else {
        p::OVERLAY_BLACK
    }
}

/// Background-derived elevation tint for the agent-chat tool cards. A
/// translucent neutral overlay picked by the pane background's *lightness*
/// (white over a dark background, black over a light one), so a tool card
/// reads one step above the pane on any background color — and, being
/// translucent, keeps the pane's window opacity showing through. Mirrors the
/// inline-code tint in the vendored `text/node.rs`.
pub fn agent_chat_tint(cx: &App) -> gpui::Hsla {
    PaneSurfaceTokens::agent_chat(cx).tint
}

/// Background-derived fill for a hovered fold header in the agent chat. Same
/// neutral-overlay rule as [`agent_chat_tint`], one step lighter — the row has
/// to answer the pointer without reading as a card.
pub fn agent_chat_hover_tint(cx: &App) -> gpui::Hsla {
    PaneSurfaceTokens::agent_chat(cx).hover_tint
}

/// Background-derived border for the agent-chat tool cards — the same neutral
/// overlay as [`agent_chat_tint`] but one step stronger, so the hairline edge
/// tracks the pane background instead of a fixed line color. Pairs with the
/// fill tint on the same card.
pub fn agent_chat_border_tint(cx: &App) -> gpui::Hsla {
    PaneSurfaceTokens::agent_chat(cx).border_tint
}

/// Background-derived tint for file-viewer pane chrome, including the toolbar,
/// mode chips, and floating search panel. Uses the same neutral-overlay rule as
/// Agent Chat so the two pane types move together under live terminal theme
/// changes without leaking this colour into the workspace tab strip.
pub fn file_viewer_pane_tint(cx: &App) -> gpui::Hsla {
    PaneSurfaceTokens::file_viewer(cx).tint
}

/// Stronger background-derived tint for active controls inside the file-viewer
/// pane, such as the Raw/Preview/Changes mode chips.
pub fn file_viewer_pane_active_tint(cx: &App) -> gpui::Hsla {
    PaneSurfaceTokens::file_viewer(cx).active_tint
}

/// Hairline tint for file-viewer pane chrome.
pub fn file_viewer_pane_border_tint(cx: &App) -> gpui::Hsla {
    PaneSurfaceTokens::file_viewer(cx).border_tint
}

/// Whether agent-chat content should pick the *light* variant of a
/// light/dark-aware palette (diff/markdown syntax highlighting, mermaid
/// diagrams, the diff embed's own fallback text colour) — judged by
/// [`agent_chat_bg`]'s own lightness, not the UI theme's `DarudaTheme::is_dark`.
/// The pane's actual paint surface is the terminal-preset background mirrored
/// into `agent_chat_bg`, which can disagree with the UI theme on light vs
/// dark; content painted on it needs to match *that* background. Single
/// source shared by `Workspace::agent_chat_theme_params` (feeds the diff/
/// mermaid reconcilers) and `ui::code_editor`'s agent-chat diff viewer.
pub fn agent_chat_syntax_is_light(cx: &App) -> bool {
    PaneSurfaceTokens::agent_chat(cx).syntax_is_light
}

pub fn file_viewer_pane_syntax_is_light(cx: &App) -> bool {
    PaneSurfaceTokens::file_viewer(cx).syntax_is_light
}
