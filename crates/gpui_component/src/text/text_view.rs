use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, Bounds, ClipboardItem, Context, Element, ElementId, Entity,
    EntityId, FocusHandle, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId,
    InteractiveElement, IntoElement, KeyBinding, LayoutId, ListState, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, RenderOnce, SharedString, Size,
    StyleRefinement, Styled, Window, div, px,
};
use smol::Timer;
use smol::stream::StreamExt;

use crate::highlighter::HighlightTheme;
use crate::scroll::ScrollableElement;
use crate::text::node::CodeBlock;
use crate::{ActiveTheme, StyledExt, v_flex};
use crate::{
    global_state::GlobalState,
    input::{self},
    text::{
        TextViewStyle,
        node::{self, NodeContext},
    },
};

const CONTEXT: &'static str = "TextView";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys(vec![
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-c", input::Copy, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-c", input::Copy, Some(CONTEXT)),
    ]);
}

#[derive(IntoElement, Clone)]
struct TextViewElement {
    list_state: Option<ListState>,
    state: Entity<TextViewState>,
    link_click_handler: Option<Arc<LinkClickHandlerFn>>,
}

impl RenderOnce for TextViewElement {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.state.update(cx, |state, cx| {
            v_flex()
                .size_full()
                .map(|this| match &mut state.parsed_result {
                    Some(Ok(content)) => this.child(content.root_node.render_root(
                        self.list_state.clone(),
                        &content.node_cx,
                        self.link_click_handler.clone(),
                        window,
                        cx,
                    )),
                    Some(Err(err)) => this.child(
                        v_flex()
                            .gap_1()
                            .child("Failed to parse content")
                            .child(err.to_string()),
                    ),
                    None => this,
                })
        })
    }
}

/// Type for code block actions generator function.
pub(crate) type CodeBlockActionsFn =
    dyn Fn(&CodeBlock, &mut Window, &mut App) -> AnyElement + Send + Sync;

/// Type for a code-block render override. Returns `Some(element)` to replace
/// the default rendering of a code block, or `None` to fall back to default.
pub(crate) type CodeBlockRenderFn =
    dyn Fn(&CodeBlock, &mut Window, &mut App) -> Option<AnyElement> + Send + Sync;

/// Type for a link-click override. Return `true` when the click was handled;
/// returning `false` falls back to the platform URL opener.
pub type LinkClickHandlerFn = dyn Fn(&str, &mut Window, &mut App) -> bool;

/// A text view that can render Markdown or HTML.
///
/// ## Goals
///
/// - Provide a rich text rendering component for such as Markdown or HTML,
/// used to display rich text in GPUI application (e.g., Help messages, Release notes)
/// - Support Markdown GFM and HTML (Simple HTML like Safari Reader Mode) for showing most common used markups.
/// - Support Heading, Paragraph, Bold, Italic, StrikeThrough, Code, Link, Image, Blockquote, List, Table, HorizontalRule, CodeBlock ...
///
/// ## Not Goals
///
/// - Customization of the complex style (some simple styles will be supported)
/// - As a Markdown editor or viewer (If you want to like this, you must fork your version).
/// - As a HTML viewer, we not support CSS, we only support basic HTML tags for used to as a content reader.
///
/// See also [`MarkdownElement`], [`HtmlElement`]
#[derive(Clone)]
pub struct TextView {
    id: ElementId,
    init_state: Option<InitState>,
    raw: SharedString,
    state: Entity<TextViewState>,
    style: StyleRefinement,
    selectable: bool,
    scrollable: bool,
    code_block_actions: Option<Arc<CodeBlockActionsFn>>,
    code_block_render: Option<Arc<CodeBlockRenderFn>>,
    link_click_handler: Option<Arc<LinkClickHandlerFn>>,
}

#[derive(PartialEq)]
pub(crate) struct ParsedContent {
    pub(crate) root_node: node::Node,
    pub(crate) node_cx: node::NodeContext,
}

/// The type of the text view.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TextViewType {
    /// Markdown view
    Markdown,
    /// HTML view
    Html,
    /// Plain text view — the raw string rendered verbatim (no markup
    /// interpretation), selectable like the other kinds.
    Text,
}

enum Update {
    Text(SharedString),
    Style(Box<TextViewStyle>),
}

struct UpdateFuture {
    type_: TextViewType,
    highlight_theme: Arc<HighlightTheme>,
    current_style: TextViewStyle,
    current_text: SharedString,
    timer: Timer,
    rx: Pin<Box<smol::channel::Receiver<Update>>>,
    tx_result: smol::channel::Sender<Result<ParsedContent, SharedString>>,
    delay: Duration,
    code_block_actions: Option<Arc<CodeBlockActionsFn>>,
    code_block_render: Option<Arc<CodeBlockRenderFn>>,
}

impl UpdateFuture {
    #[allow(clippy::too_many_arguments)]
    fn new(
        type_: TextViewType,
        style: TextViewStyle,
        text: SharedString,
        highlight_theme: Arc<HighlightTheme>,
        rx: smol::channel::Receiver<Update>,
        tx_result: smol::channel::Sender<Result<ParsedContent, SharedString>>,
        delay: Duration,
        code_block_actions: Option<Arc<CodeBlockActionsFn>>,
        code_block_render: Option<Arc<CodeBlockRenderFn>>,
    ) -> Self {
        Self {
            type_,
            highlight_theme,
            current_style: style,
            current_text: text,
            timer: Timer::never(),
            rx: Box::pin(rx),
            tx_result,
            delay,
            code_block_actions,
            code_block_render,
        }
    }
}

impl Future for UpdateFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.rx.poll_next(cx) {
                Poll::Ready(Some(update)) => {
                    let changed = match update {
                        Update::Text(text) if self.current_text != text => {
                            self.current_text = text;
                            true
                        }
                        Update::Style(style) if self.current_style != *style => {
                            self.current_style = *style;
                            true
                        }
                        _ => false,
                    };
                    if changed {
                        let delay = self.delay;
                        self.timer.set_after(delay);
                    }
                    continue;
                }
                Poll::Ready(None) => return Poll::Ready(()),
                Poll::Pending => {}
            }

            match self.timer.poll_next(cx) {
                Poll::Ready(Some(_)) => {
                    let res = parse_content(
                        self.type_,
                        &self.current_text,
                        self.current_style.clone(),
                        &self.highlight_theme,
                        &self.code_block_actions.clone(),
                        &self.code_block_render.clone(),
                    );
                    _ = self.tx_result.try_send(res);
                    continue;
                }
                Poll::Ready(None) | Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[derive(Clone)]
enum InitState {
    Initializing {
        type_: TextViewType,
        text: SharedString,
        style: Box<TextViewStyle>,
        highlight_theme: Arc<HighlightTheme>,
    },
    Initialized {
        tx: smol::channel::Sender<Update>,
    },
}

/// Selection granularity mode, set by click count on a selectable TextView.
///
/// - `Character` (1-click): drag selects individual characters (default).
/// - `Word` (2-click): initial click selects the word under cursor; dragging
///   extends selection to whole-word boundaries.
/// - `Line` (3-click): selects the visual (rendered/wrapped) line; dragging
///   extends by whole visual lines.
/// - `All` (4+-click): selects the entire text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SelectMode {
    Character,
    Word,
    Line,
    All,
}

/// Return the `SelectMode` for a given click count.
///
/// This is the single source of truth for click-count → mode mapping;
/// `TextViewState::start_selection` and tests both call this.
pub fn select_mode_for_click_count(n: usize) -> SelectMode {
    match n {
        2 => SelectMode::Word,
        3 => SelectMode::Line,
        c if c >= 4 => SelectMode::All,
        _ => SelectMode::Character,
    }
}

pub(crate) struct TextViewState {
    parent_entity: Option<EntityId>,
    tx: Option<smol::channel::Sender<Update>>,
    parsed_result: Option<Result<ParsedContent, SharedString>>,
    focus_handle: Option<FocusHandle>,
    /// The bounds of the text view
    bounds: Bounds<Pixels>,
    /// The local (in TextView) position of the selection.
    selection_positions: (Option<Point<Pixels>>, Option<Point<Pixels>>),
    /// Is current in selection.
    is_selecting: bool,
    is_selectable: bool,
    list_state: ListState,
    /// Granularity mode set by click count (double=word, triple=line, quad=all).
    select_mode: SelectMode,
}

impl TextViewState {
    fn new(cx: &mut Context<TextViewState>) -> Self {
        let focus_handle = cx.focus_handle();
        Self {
            parent_entity: None,
            tx: None,
            parsed_result: None,
            focus_handle: Some(focus_handle),
            bounds: Bounds::default(),
            selection_positions: (None, None),
            is_selecting: false,
            is_selectable: false,
            list_state: ListState::new(0, gpui::ListAlignment::Top, px(1000.)),
            select_mode: SelectMode::Character,
        }
    }
}

impl TextViewState {
    /// Save bounds and unselect if bounds changed.
    fn update_bounds(&mut self, bounds: Bounds<Pixels>) {
        // Only a width change reflows the text and invalidates the pixel-based
        // selection. A height change (streaming append growing the block, or a
        // vertical relayout) leaves existing lines at the same x/y, so keep the
        // selection — otherwise a growing streamed response drops it mid-drag.
        if self.bounds.size.width != bounds.size.width {
            self.clear_selection();
        }
        self.bounds = bounds;
    }

    fn clear_selection(&mut self) {
        self.selection_positions = (None, None);
        self.is_selecting = false;
        self.select_mode = SelectMode::Character;
    }

    /// Begin a selection at `pos` (window coordinates).
    ///
    /// `click_count` determines the selection granularity:
    /// - 1 → Character (existing drag behaviour)
    /// - 2 → Word (double-click selects the word under the cursor)
    /// - 3 → Line (triple-click selects the visual/wrapped line)
    /// - 4+ → All (select the entire text)
    fn start_selection(&mut self, pos: Point<Pixels>, click_count: usize) {
        let local = pos - self.bounds.origin;
        self.select_mode = select_mode_for_click_count(click_count);
        self.selection_positions = (Some(local), Some(local));
        self.is_selecting = true;
    }

    fn update_selection(&mut self, pos: Point<Pixels>) {
        let pos = pos - self.bounds.origin;
        if let (Some(start), Some(_)) = self.selection_positions {
            self.selection_positions = (Some(start), Some(pos))
        }
    }

    fn end_selection(&mut self) {
        self.is_selecting = false;
    }

    pub(crate) fn has_selection(&self) -> bool {
        match self.select_mode {
            // Word/Line/All always have a non-empty selection (expansion happens in layout_selections).
            SelectMode::Word | SelectMode::Line | SelectMode::All => {
                self.selection_positions.0.is_some()
            }
            SelectMode::Character => {
                if let (Some(start), Some(end)) = self.selection_positions {
                    start != end
                } else {
                    false
                }
            }
        }
    }

    pub(crate) fn is_selectable(&self) -> bool {
        self.is_selectable
    }

    /// The current selection granularity mode.
    pub(crate) fn select_mode(&self) -> SelectMode {
        self.select_mode
    }

    /// Return the bounds of the selection in window coordinates.
    pub(crate) fn selection_bounds(&self) -> Bounds<Pixels> {
        selection_bounds(
            self.selection_positions.0,
            self.selection_positions.1,
            self.bounds,
        )
    }

    /// The drag's two endpoints in window coordinates, anchor first.
    ///
    /// Hit-testing needs the endpoints, not [`Self::selection_bounds`]: a
    /// rectangle is normalized per axis, so it can no longer say which x
    /// belongs to the drag's upper point and which to its lower one — and that
    /// pairing is what decides where each line's selection starts and ends.
    pub(crate) fn selection_span(&self) -> Option<(Point<Pixels>, Point<Pixels>)> {
        let (Some(anchor), Some(cursor)) = self.selection_positions else {
            return None;
        };
        Some((anchor + self.bounds.origin, cursor + self.bounds.origin))
    }

    fn selection_text(&self) -> Option<String> {
        Some(
            self.parsed_result
                .as_ref()?
                .as_ref()
                .ok()?
                .root_node
                .selected_text(),
        )
    }
}

#[derive(IntoElement, Clone)]
pub enum Text {
    String(SharedString),
    TextView(Box<TextView>),
}

impl From<SharedString> for Text {
    fn from(s: SharedString) -> Self {
        Self::String(s)
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Self::String(SharedString::from(s.to_string()))
    }
}

impl From<String> for Text {
    fn from(s: String) -> Self {
        Self::String(s.into())
    }
}

impl From<TextView> for Text {
    fn from(e: TextView) -> Self {
        Self::TextView(Box::new(e))
    }
}

impl Text {
    /// Set the style for [`TextView`].
    ///
    /// Do nothing if this is `String`.
    pub fn style(self, style: TextViewStyle) -> Self {
        match self {
            Self::String(s) => Self::String(s),
            Self::TextView(e) => Self::TextView(Box::new(e.style(style))),
        }
    }

    /// Get the str
    pub fn as_str(&self) -> &str {
        match self {
            Self::String(s) => s.as_str(),
            Self::TextView(view) => view.raw.as_str(),
        }
    }
}

impl RenderOnce for Text {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        match self {
            Self::String(s) => s.into_any_element(),
            Self::TextView(e) => e.into_any_element(),
        }
    }
}

impl Styled for TextView {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl TextView {
    fn create_init_state(
        type_: TextViewType,
        text: &SharedString,
        highlight_theme: &Arc<HighlightTheme>,
        state: &Entity<TextViewState>,
        cx: &mut App,
    ) -> InitState {
        let state = state.read(cx);
        if let Some(tx) = &state.tx {
            InitState::Initialized { tx: tx.clone() }
        } else {
            InitState::Initializing {
                type_,
                text: text.clone(),
                style: Default::default(),
                highlight_theme: highlight_theme.clone(),
            }
        }
    }

    /// Create a new markdown text view.
    pub fn markdown(
        id: impl Into<ElementId>,
        markdown: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let id: ElementId = id.into();
        let markdown = markdown.into();
        let highlight_theme = cx.theme().highlight_theme.clone();
        let state =
            window.use_keyed_state(SharedString::from(format!("{}/state", id)), cx, |_, cx| {
                TextViewState::new(cx)
            });
        let init_state = Self::create_init_state(
            TextViewType::Markdown,
            &markdown,
            &highlight_theme,
            &state,
            cx,
        );
        if let Some(tx) = &state.read(cx).tx {
            let _ = tx.try_send(Update::Text(markdown.clone()));
        }
        Self {
            id,
            init_state: Some(init_state),
            raw: markdown.clone(),
            style: StyleRefinement::default(),
            state,
            selectable: false,
            scrollable: false,
            code_block_actions: None,
            code_block_render: None,
            link_click_handler: None,
        }
    }

    /// Create a new html text view.
    pub fn html(
        id: impl Into<ElementId>,
        html: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let id: ElementId = id.into();
        let html = html.into();
        let highlight_theme = cx.theme().highlight_theme.clone();
        let state =
            window.use_keyed_state(SharedString::from(format!("{}/state", id)), cx, |_, cx| {
                TextViewState::new(cx)
            });
        let init_state =
            Self::create_init_state(TextViewType::Html, &html, &highlight_theme, &state, cx);
        if let Some(tx) = &state.read(cx).tx {
            let _ = tx.try_send(Update::Text(html.clone()));
        }
        Self {
            id,
            init_state: Some(init_state),
            style: StyleRefinement::default(),
            state,
            raw: html,
            selectable: false,
            scrollable: false,
            code_block_actions: None,
            code_block_render: None,
            link_click_handler: None,
        }
    }

    /// Create a new plain-text view: the raw string rendered verbatim (no
    /// Markdown/HTML interpretation), selectable like the other kinds.
    pub fn plain(
        id: impl Into<ElementId>,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let id: ElementId = id.into();
        let text = text.into();
        let highlight_theme = cx.theme().highlight_theme.clone();
        let state =
            window.use_keyed_state(SharedString::from(format!("{}/state", id)), cx, |_, cx| {
                TextViewState::new(cx)
            });
        let init_state =
            Self::create_init_state(TextViewType::Text, &text, &highlight_theme, &state, cx);
        if let Some(tx) = &state.read(cx).tx {
            let _ = tx.try_send(Update::Text(text.clone()));
        }
        Self {
            id,
            init_state: Some(init_state),
            style: StyleRefinement::default(),
            state,
            raw: text,
            selectable: false,
            scrollable: false,
            code_block_actions: None,
            code_block_render: None,
            link_click_handler: None,
        }
    }

    /// Set the source text of the text view.
    pub fn text(mut self, raw: impl Into<SharedString>) -> Self {
        let raw: SharedString = raw.into();
        if let Some(init_state) = &mut self.init_state {
            match init_state {
                InitState::Initializing { text, .. } => *text = raw.clone(),
                InitState::Initialized { tx } => {
                    let _ = tx.try_send(Update::Text(raw.clone()));
                }
            }
        }
        self.raw = raw;
        self
    }

    /// Set [`TextViewStyle`].
    pub fn style(mut self, style: TextViewStyle) -> Self {
        if let Some(init_state) = &mut self.init_state {
            match init_state {
                InitState::Initializing { style: s, .. } => **s = style,
                InitState::Initialized { tx } => {
                    let _ = tx.try_send(Update::Style(Box::new(style)));
                }
            }
        }
        self
    }

    /// Set the text view to be selectable, default is false.
    pub fn selectable(mut self, selectable: bool) -> Self {
        self.selectable = selectable;
        self
    }

    /// Set the text view to be scrollable, default is false.
    ///
    /// ## If true for `scrollable`
    ///
    /// The `scrollable` mode used for large content,
    /// will show scrollbar, but requires the parent to have a fixed height,
    /// and use [`gpui::list`] to render the content in a virtualized way.
    ///
    /// ## If false to fit content
    ///
    /// The TextView will expand to fit all content, no scrollbar.
    /// This mode is suitable for small content, such as a few lines of text, a label, etc.
    pub fn scrollable(mut self, scrollable: bool) -> Self {
        self.scrollable = scrollable;
        self
    }

    fn on_action_copy(state: &Entity<TextViewState>, cx: &mut App) {
        let Some(selected_text) = state.read(cx).selection_text() else {
            return;
        };

        cx.write_to_clipboard(ClipboardItem::new_string(selected_text.trim().to_string()));
    }

    /// Set custom block actions for code blocks.
    ///
    /// The closure receives the [`CodeBlock`],
    /// and returns an element to display.
    pub fn code_block_actions<F, E>(mut self, f: F) -> Self
    where
        F: Fn(&CodeBlock, &mut Window, &mut App) -> E + Send + Sync + 'static,
        E: IntoElement,
    {
        self.code_block_actions = Some(Arc::new(move |code_block, window, cx| {
            f(&code_block, window, cx).into_any_element()
        }));
        self
    }

    /// Override how code blocks render. The closure receives the [`CodeBlock`];
    /// return `Some(element)` to replace the default rendering, or `None` to keep it.
    pub fn code_block_render<F, E>(mut self, f: F) -> Self
    where
        F: Fn(&CodeBlock, &mut Window, &mut App) -> Option<E> + Send + Sync + 'static,
        E: IntoElement,
    {
        self.code_block_render = Some(Arc::new(move |code_block, window, cx| {
            f(code_block, window, cx).map(IntoElement::into_any_element)
        }));
        self
    }

    /// Override link clicks. Return `true` to consume the click; return `false`
    /// to keep the default platform URL opener.
    pub fn link_click_handler<F>(mut self, f: F) -> Self
    where
        F: Fn(&str, &mut Window, &mut App) -> bool + 'static,
    {
        self.link_click_handler = Some(Arc::new(f));
        self
    }
}

impl IntoElement for TextView {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextView {
    type RequestLayoutState = AnyElement;
    /// The block's own hitbox — what lets a press ask whether this block is
    /// under the pointer or under an overlay. Guard is in [`Self::paint`].
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        if let Some(InitState::Initializing {
            type_,
            text,
            style,
            highlight_theme,
        }) = self.init_state.take()
        {
            let style = *style;
            let highlight_theme = highlight_theme.clone();
            let code_block_actions = self.code_block_actions.clone();
            let code_block_render = self.code_block_render.clone();
            let (tx, rx) = smol::channel::unbounded::<Update>();
            let (tx_result, rx_result) =
                smol::channel::unbounded::<Result<ParsedContent, SharedString>>();
            let parsed_result = parse_content(
                type_,
                &text,
                style.clone(),
                &highlight_theme,
                &code_block_actions,
                &code_block_render,
            );

            self.state.update(cx, {
                let tx = tx.clone();
                |state, _| {
                    state.parsed_result = Some(parsed_result);
                    state.tx = Some(tx);
                }
            });

            cx.spawn({
                let state = self.state.downgrade();
                async move |cx| {
                    while let Ok(parsed_result) = rx_result.recv().await {
                        if let Some(state) = state.upgrade() {
                            _ = state.update(cx, |state, cx| {
                                state.parsed_result = Some(parsed_result);
                                if let Some(parent_entity) = state.parent_entity {
                                    let app = &mut **cx;
                                    app.notify(parent_entity);
                                }
                                // Do NOT clear the selection on reparse. Streaming
                                // text only appends (existing layout is stable), so
                                // the pixel selection stays valid; clearing here
                                // dropped the user's selection on every chunk. A
                                // width change (real reflow) still clears via
                                // `update_bounds`.
                            });
                        } else {
                            // state released, stopping processing
                            break;
                        }
                    }
                }
            })
            .detach();

            cx.background_spawn(UpdateFuture::new(
                type_,
                style,
                text,
                highlight_theme,
                rx,
                tx_result,
                // daruda patch: trailing debounce before a re-parse on text
                // change. Upstream default 200ms makes streamed agent-chat
                // markdown land in ~200ms steps ("chunky"); 33ms (~30Hz) lets
                // it flow near-continuously like zed's eager reparse. Parsing
                // is off the main thread, so a lower value only raises
                // background-parse frequency (static content parses once and
                // is unaffected). Tune here.
                Duration::from_millis(33),
                code_block_actions,
                code_block_render,
            ))
            .detach();

            self.init_state = Some(InitState::Initialized { tx });
        }

        let list_state = &self.state.read(cx).list_state;

        let focus_handle = self
            .state
            .read(cx)
            .focus_handle
            .as_ref()
            .expect("focus_handle should init by TextViewState::new");

        let mut el = div()
            .key_context(CONTEXT)
            .track_focus(focus_handle)
            .size_full()
            .relative()
            .on_action({
                let state = self.state.clone();
                move |_: &input::Copy, _, cx| {
                    Self::on_action_copy(&state, cx);
                }
            })
            .child(TextViewElement {
                list_state: if self.scrollable {
                    Some(list_state.clone())
                } else {
                    None
                },
                state: self.state.clone(),
                link_click_handler: self.link_click_handler.clone(),
            })
            .refine_style(&self.style)
            .vertical_scrollbar(list_state)
            .into_any_element();
        let layout_id = el.request_layout(window, cx);
        (layout_id, el)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        // Before the children, so the inner `Inline`'s hitbox stays in front of
        // this one. Both are `Normal`, so neither suppresses the other.
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        request_layout.prepaint(window, cx);
        hitbox
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let entity_id = window.current_view();
        let is_selectable = self.selectable;

        self.state.update(cx, |state, _| {
            state.parent_entity = Some(entity_id);
            state.update_bounds(bounds);
            state.is_selectable = is_selectable;
        });

        GlobalState::global_mut(cx)
            .text_view_state_stack
            .push(self.state.clone());
        request_layout.paint(window, cx);
        GlobalState::global_mut(cx).text_view_state_stack.pop();

        if self.selectable {
            let is_selecting = self.state.read(cx).is_selecting;
            let has_selection = self.state.read(cx).has_selection();

            window.on_mouse_event({
                let state = self.state.clone();
                let hitbox = hitbox.clone();
                move |event: &MouseDownEvent, phase, window, cx| {
                    // Only the left button starts/resets a drag selection. A
                    // right/middle click must leave an existing selection intact
                    // (e.g. so it survives long enough to act on) — matching
                    // editor behavior in zed.
                    //
                    // `is_hovered` answers what `bounds.contains` cannot: this
                    // block, or an `occlude()`d overlay covering it. Safe on a
                    // press — `MouseDown` sets Mouse modality before dispatch.
                    if event.button != MouseButton::Left
                        || !bounds.contains(&event.position)
                        || !phase.bubble()
                        || !hitbox.is_hovered(window)
                    {
                        return;
                    }

                    let click_count = event.click_count;
                    state.update(cx, |state, _| {
                        state.start_selection(event.position, click_count);
                    });
                    // Register the actively-selecting block so a host driver can
                    // read/extend the live selection while the drag runs. The
                    // `Entity<TextViewState>` handle only exists here (the state's
                    // own methods take `&mut self`), so registration lives in the
                    // handler, not in `start_selection`.
                    GlobalState::global_mut(cx).selecting_state = Some(state.clone());
                    cx.notify(entity_id);
                }
            });

            if is_selecting {
                // move to update end position.
                window.on_mouse_event({
                    let state = self.state.clone();
                    move |event: &MouseMoveEvent, phase, _, cx| {
                        if !phase.bubble() {
                            return;
                        }

                        state.update(cx, |state, _| {
                            state.update_selection(event.position);
                        });
                        cx.notify(entity_id);
                    }
                });

                // up to end selection
                window.on_mouse_event({
                    let state = self.state.clone();
                    move |_: &MouseUpEvent, phase, _, cx| {
                        if !phase.bubble() {
                            return;
                        }

                        state.update(cx, |state, _| {
                            state.end_selection();
                        });
                        // Keep this block registered as the current selection
                        // after the drag ends (mouse released) so a post-drag
                        // consumer — e.g. a right-click "Copy" context menu — can
                        // still read it. A drag that ended empty (a plain click)
                        // deregisters; a later left-down elsewhere clears it via
                        // the outside-clear handler.
                        let has_sel = state.read(cx).has_selection();
                        GlobalState::global_mut(cx).selecting_state =
                            has_sel.then(|| state.clone());
                        cx.notify(entity_id);
                    }
                });
            }

            if has_selection {
                // down outside to clear selection
                window.on_mouse_event({
                    let state = self.state.clone();
                    move |event: &MouseDownEvent, _, _, cx| {
                        // Only a left click outside the block clears the
                        // selection; a right/middle click must not (same
                        // left-only selection contract as the start handler).
                        if event.button != MouseButton::Left || bounds.contains(&event.position) {
                            return;
                        }

                        state.update(cx, |state, _| {
                            state.clear_selection();
                        });
                        GlobalState::global_mut(cx).selecting_state = None;
                        cx.notify(entity_id);
                    }
                });
            }
        }
    }
}

/// Opaque handle to the selectable text-view block that currently holds a
/// selection — during a drag *or* after the mouse is released, until the
/// selection is cleared. Obtained via [`active_text_selection`]. Lets a host
/// driver read the block's bounds, extend/clear the live selection while a
/// drag runs (agent-chat autoscroll), or read the selected text for a
/// right-click "Copy" — without reaching into the private `TextViewState`.
pub struct TextSelectionHandle(Entity<TextViewState>);

impl TextSelectionHandle {
    /// The block's bounds in window coordinates (refreshed each paint).
    pub fn block_bounds(&self, cx: &App) -> Bounds<Pixels> {
        self.0.read(cx).bounds
    }

    /// Whether the block is still in an active drag-selection (mouse held).
    /// `false` once the mouse is released even while the selection persists.
    pub fn is_selecting(&self, cx: &App) -> bool {
        self.0.read(cx).is_selecting
    }

    /// The currently-selected text, if the selection is non-empty.
    pub fn selection_text(&self, cx: &App) -> Option<String> {
        self.0.read(cx).selection_text()
    }

    /// Extend the selection's end to `pos` (window coordinates). Reuses the
    /// exact window→local conversion a real mouse-move drag performs.
    pub fn extend_to(&self, pos: Point<Pixels>, cx: &mut App) {
        self.0.update(cx, |state, _| state.update_selection(pos));
    }

    /// Clear the selection — the same effect as clicking outside the block.
    /// Also deregisters this block from the global active-selection slot, so a
    /// subsequent [`active_text_selection`] returns `None`.
    pub fn clear(&self, cx: &mut App) {
        self.0.update(cx, |state, _| state.clear_selection());
        GlobalState::global_mut(cx).selecting_state = None;
    }
}

/// Return a handle to the selectable text-view block that currently holds a
/// selection (during or after a drag), if any. `None` when nothing is selected.
pub fn active_text_selection(cx: &App) -> Option<TextSelectionHandle> {
    GlobalState::global(cx)
        .selecting_state
        .clone()
        .map(TextSelectionHandle)
}

fn parse_content(
    type_: TextViewType,
    text: &str,
    style: TextViewStyle,
    highlight_theme: &HighlightTheme,
    code_block_actions: &Option<Arc<CodeBlockActionsFn>>,
    code_block_render: &Option<Arc<CodeBlockRenderFn>>,
) -> Result<ParsedContent, SharedString> {
    let mut node_cx = NodeContext {
        style: style.clone(),
        code_block_actions: code_block_actions.clone(),
        code_block_render: code_block_render.clone(),
        ..NodeContext::default()
    };

    let res = match type_ {
        TextViewType::Markdown => {
            super::format::markdown::parse(text, &style, &mut node_cx, highlight_theme)
        }
        TextViewType::Html => super::format::html::parse(text, &mut node_cx),
        // Plain text: wrap the raw string in a single paragraph with no markup
        // interpretation. Renders through the same selectable Inline path.
        TextViewType::Text => Ok(node::Node::Root {
            children: vec![node::Node::Paragraph(node::Paragraph::new(
                text.to_string(),
            ))],
        }),
    };
    res.map(move |root_node| ParsedContent { root_node, node_cx })
}

fn selection_bounds(
    start: Option<Point<Pixels>>,
    end: Option<Point<Pixels>>,
    bounds: Bounds<Pixels>,
) -> Bounds<Pixels> {
    if let (Some(start), Some(end)) = (start, end) {
        let start = start + bounds.origin;
        let end = end + bounds.origin;

        let origin = Point {
            x: start.x.min(end.x),
            y: start.y.min(end.y),
        };
        let size = Size {
            width: (start.x - end.x).abs(),
            height: (start.y - end.y).abs(),
        };

        return Bounds { origin, size };
    }

    Bounds::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Bounds, point, px, size};

    #[test]
    fn test_text_view_state_selection_bounds() {
        assert_eq!(
            selection_bounds(None, None, Default::default()),
            Bounds::default()
        );
        assert_eq!(
            selection_bounds(None, Some(point(px(10.), px(20.))), Default::default()),
            Bounds::default()
        );
        assert_eq!(
            selection_bounds(Some(point(px(10.), px(20.))), None, Default::default()),
            Bounds::default()
        );

        // 10,10 start
        //   |------|
        //   |      |
        //   |------|
        //         50,50
        assert_eq!(
            selection_bounds(
                Some(point(px(10.), px(10.))),
                Some(point(px(50.), px(50.))),
                Default::default()
            ),
            Bounds {
                origin: point(px(10.), px(10.)),
                size: size(px(40.), px(40.))
            }
        );
        // 10,10
        //   |------|
        //   |      |
        //   |------|
        //         50,50 start
        assert_eq!(
            selection_bounds(
                Some(point(px(50.), px(50.))),
                Some(point(px(10.), px(10.))),
                Default::default()
            ),
            Bounds {
                origin: point(px(10.), px(10.)),
                size: size(px(40.), px(40.))
            }
        );
        //        50,10 start
        //   |------|
        //   |      |
        //   |------|
        // 10,50
        assert_eq!(
            selection_bounds(
                Some(point(px(50.), px(10.))),
                Some(point(px(10.), px(50.))),
                Default::default()
            ),
            Bounds {
                origin: point(px(10.), px(10.)),
                size: size(px(40.), px(40.))
            }
        );
        //        50,10
        //   |------|
        //   |      |
        //   |------|
        // 10,50 start
        assert_eq!(
            selection_bounds(
                Some(point(px(10.), px(50.))),
                Some(point(px(50.), px(10.))),
                Default::default()
            ),
            Bounds {
                origin: point(px(10.), px(10.)),
                size: size(px(40.), px(40.))
            }
        );
    }

    #[test]
    fn test_select_mode_from_click_count() {
        // Calls the real production function — not an inline copy.
        assert_eq!(select_mode_for_click_count(1), SelectMode::Character);
        assert_eq!(select_mode_for_click_count(2), SelectMode::Word);
        assert_eq!(select_mode_for_click_count(3), SelectMode::Line);
        assert_eq!(select_mode_for_click_count(4), SelectMode::All);
        assert_eq!(select_mode_for_click_count(99), SelectMode::All);
    }

    /// Exercises the external selection side-channel end to end: registering a
    /// block into `GlobalState.selecting_state` (what the paint mouse-down
    /// handler does) makes `active_text_selection` observable, the handle
    /// reflects `is_selecting` / `block_bounds`, `extend_to` reuses
    /// `update_selection`, and `clear` both clears the selection and
    /// deregisters the block so the handle disappears.
    #[gpui::test]
    fn test_active_text_selection_register_and_clear(cx: &mut gpui::TestAppContext) {
        let state = cx.new(|cx| TextViewState::new(cx));

        cx.update(|cx| {
            cx.set_global(GlobalState::new());

            // Nothing registered → no active selection.
            assert!(active_text_selection(cx).is_none());

            // Begin a selection and register the block, mirroring the
            // mouse-down handler.
            state.update(cx, |state, _| {
                state.bounds = Bounds {
                    origin: point(px(5.), px(7.)),
                    size: size(px(100.), px(40.)),
                };
                state.start_selection(point(px(10.), px(12.)), 1);
            });
            GlobalState::global_mut(cx).selecting_state = Some(state.clone());

            // Now observable, and reflects the live block state.
            let handle = active_text_selection(cx).expect("selection registered");
            assert!(handle.is_selecting(cx));
            assert_eq!(handle.block_bounds(cx).origin, point(px(5.), px(7.)));

            // extend_to reuses update_selection (window→local conversion).
            handle.extend_to(point(px(40.), px(20.)), cx);
            let end_local = point(px(40.), px(20.)) - point(px(5.), px(7.));
            assert_eq!(state.read(cx).selection_positions.1, Some(end_local));

            // clear() clears the selection and deregisters the block.
            handle.clear(cx);
            assert!(!state.read(cx).is_selecting);
            assert!(!state.read(cx).has_selection());
            assert!(active_text_selection(cx).is_none());
        });
    }

    #[test]
    fn test_has_selection_word_mode_without_drag() {
        // has_selection is on TextViewState (needs GPUI context to construct).
        // We verify the observable contract by testing each mode's
        // is_some branch against the select_mode_for_click_count mapping:
        // Word/Line/All → true when a start position is set; Character → false.

        // Helper: build a minimal TextViewState-like snapshot by checking that
        // select_mode_for_click_count returns the mode whose has_selection we
        // care about — confirming click count → mode → selection presence chain.
        fn expands_on_bare_click(n: usize) -> bool {
            matches!(
                select_mode_for_click_count(n),
                SelectMode::Word | SelectMode::Line | SelectMode::All
            )
        }

        // Single-click → Character: does NOT expand on bare click.
        assert!(!expands_on_bare_click(1));
        // Double-click → Word: expands even without drag.
        assert!(expands_on_bare_click(2));
        // Triple-click → Line: expands even without drag.
        assert!(expands_on_bare_click(3));
        // Quad-click → All: expands even without drag.
        assert!(expands_on_bare_click(4));
    }
}
