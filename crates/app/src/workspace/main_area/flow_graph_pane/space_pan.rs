//! Space held, then drag, to move the canvas under the graph.
//!
//! The vendored canvas already pans on shift-drag and on a middle drag, so
//! this adds a third way in rather than the ability. It is here and not a
//! vendor patch because the ability is reachable from outside:
//! [`PluginContext`] hands out the offset and takes an [`Interaction`], which
//! is all a pan is.
//!
//! Three things it does differently from the vendor's, all deliberate:
//!
//! Space is a key, not a modifier, so `MouseDownEvent::modifiers` cannot
//! answer whether it is down and the plugin tracks that itself.
//!
//! Finishing runs no command, so a pan leaves the undo stack alone. Moving the
//! view is not a change to the flow, and putting it on the stack makes the
//! next undo scroll the canvas instead of taking back the last edit.
//!
//! Being armed is published back to the pane through [`PanArmed`], because the
//! cursor that says so has to be drawn outside the canvas — see that type.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{CursorStyle, MouseButton, Pixels, Point};

use crate::ui::cursor::CursorReach;
use crate::ui::flow_canvas::{
    EventResult, FlowEvent, InputEvent, Interaction, InteractionResult, Plugin, PluginContext,
};

/// The key that arms the drag, as gpui names it.
const PAN_KEY: &str = "space";

/// Above every plugin that claims a drag — port (125), node click (122), node
/// interaction (120), selection (100). Space-drag moves the view whatever it
/// starts over, which is the whole affordance: with a lower number it would
/// pan only where there is nothing to grab, and that is where dragging already
/// works.
const PRIORITY: i32 = 130;

/// The two facts the cursor is drawn from. Kept apart because they move
/// independently: releasing the key mid-drag ends the arming but not the pan,
/// and the hand has to stay closed until the button comes up.
#[derive(Clone, Copy, Default, PartialEq)]
struct PanState {
    held: bool,
    panning: bool,
    /// Whether the pointer is over the canvas. Not about the cursor's *shape* —
    /// it is what says the app's hide-on-typing policy has nothing to protect
    /// here, and it has to be true before the key is pressed rather than after.
    hovering: bool,
}

/// What the pointer should look like over the canvas, shared with the view.
///
/// The cursor is the only thing that says the canvas has changed what a drag
/// means, and it cannot be set from in here: the canvas's own root is the
/// vendor's, and a full-size overlay to hang a cursor on is what
/// [`super::render::toolbar`] already documents as making everything behind it
/// read as un-hovered. So the pane draws it on the element it owns — the one
/// the canvas is a child of — and this is how it learns.
///
/// Read rather than taken, unlike [`super::delete_key::DeleteRequest`]: a
/// pressed key is a state the cursor keeps showing, not an event answered once.
///
/// Written from two places — the plugin owns `held`, the drag owns `panning` —
/// and each through its own method, so the pair is only ever read together and
/// only [`Self::cursor`] decides what they mean.
#[derive(Clone, Default)]
pub(super) struct PanArmed(Rc<Cell<PanState>>);

impl PanArmed {
    fn held(&self) -> bool {
        self.0.get().held
    }

    fn panning(&self) -> bool {
        self.0.get().panning
    }

    /// Whether the pointer is over the canvas, and so whether the pan key can
    /// be pressed at all.
    pub(super) fn over_the_canvas(&self) -> bool {
        self.0.get().hovering
    }

    fn set_hovering(&self, hovering: bool) {
        self.0.set(PanState {
            hovering,
            ..self.0.get()
        });
    }

    fn set_held(&self, held: bool) {
        self.0.set(PanState {
            held,
            ..self.0.get()
        });
    }

    fn set_panning(&self, panning: bool) {
        self.0.set(PanState {
            panning,
            ..self.0.get()
        });
    }

    /// Let the key go on behalf of the plugin. The canvas only hears a release
    /// it has the focus for, so a pane that loses it would otherwise stay armed
    /// with no key down — and keep the app's pointer policy suspended.
    pub(super) fn let_go(&self) {
        self.set_held(false);
    }

    /// Move the pointer the way the plugin does, for a test about what the view
    /// makes of it.
    #[cfg(test)]
    pub(in crate::workspace) fn set_hovering_for_test(&self, over: bool) {
        self.set_hovering(over);
    }

    /// Write `held` the way the plugin does, for a test about what the view
    /// makes of it. A headless window has no canvas to press a key on.
    #[cfg(test)]
    pub(in crate::workspace) fn set_held_for_test(&self, held: bool) {
        self.set_held(held);
    }

    /// The hand closes while the view is actually moving, and opens while a
    /// drag merely would move it. `None` leaves the pointer to whatever the
    /// canvas would draw — a card, a port, or the arrow.
    ///
    /// The closed one reaches the whole window: a pan re-renders the canvas
    /// every frame, and a cursor bound to a hitbox flickers under that. See
    /// [`crate::ui::cursor`].
    pub(super) fn cursor(&self) -> Option<CursorReach> {
        let state = self.0.get();
        if state.panning {
            Some(CursorReach::Dragging(CursorStyle::ClosedHand))
        } else if state.held {
            Some(CursorReach::Hovered(CursorStyle::OpenHand))
        } else {
            None
        }
    }
}

/// What one event means to the plugin. Carries the press position so the
/// caller has nothing left to match on — reading the event a second time to
/// pull it out would put "this verdict only comes from a mouse press" in two
/// places, and a later edit could break the pair without failing anything.
enum Verdict {
    /// Nothing to do.
    Ignored,
    /// Armed or disarmed. The pane draws a different cursor now.
    Rearmed,
    /// Start panning from here.
    PanFrom(Point<Pixels>),
}

pub(super) struct SpacePanPlugin {
    /// Whether [`PAN_KEY`] is down — kept in the shared state rather than
    /// beside it, so the view can let the key go when the canvas loses focus
    /// and the plugin reads the same answer afterwards.
    armed: PanArmed,
}

impl SpacePanPlugin {
    pub(super) fn new(armed: PanArmed) -> Self {
        Self { armed }
    }

    /// Fold one event into what the plugin knows and say what follows.
    ///
    /// `dragging` is whether the canvas is in the middle of an interaction —
    /// see the `Hover` arm. Taken as an argument so the whole state machine
    /// can be tested without a canvas to drag on.
    fn absorb(&mut self, event: &InputEvent, dragging: bool) -> Verdict {
        let was = self.armed.0.get();
        // A pan cannot outlive the interaction that is doing it. Nothing else
        // clears `panning` — only [`SpacePan::on_mouse_up`] does — so an
        // interaction ended by any other means (cancelled by another plugin, or
        // a canvas rebuilt under it while the button was down) would leave the
        // closed hand drawn over a canvas nobody is dragging, for good. The
        // hand is put right on the next event instead.
        if !dragging && self.armed.panning() {
            self.armed.set_panning(false);
        }
        match event {
            InputEvent::KeyDown(ev) if ev.keystroke.key == PAN_KEY => {
                // Held down, so this repeats; idempotent on purpose.
                self.armed.set_held(true);
            }
            InputEvent::KeyUp(ev) if ev.keystroke.key == PAN_KEY => self.armed.set_held(false),
            // The pointer left the canvas. Forgetting here is what keeps a
            // release that happened somewhere else from arming every later
            // click: the key-up goes to whoever took the focus, and without
            // this the next plain click would pan instead of select.
            //
            // Not while a drag is running, though. The canvas hands the
            // interaction only the moves and the release, so this arrives even
            // mid-pan — and panning a graph as far as the pane's edge is
            // ordinary. Disarming there would leave a still-pressed key
            // needing a release and a fresh press before the next pan.
            InputEvent::Hover(false) if !dragging => {
                self.armed.set_held(false);
                self.armed.set_hovering(false);
            }
            // Left mid-drag: the key is kept (the pan runs on), but the pointer
            // is elsewhere and the policy is no longer ours to hold.
            InputEvent::Hover(false) => self.armed.set_hovering(false),
            InputEvent::Hover(true) => self.armed.set_hovering(true),
            InputEvent::MouseDown(ev) if self.armed.held() && ev.button == MouseButton::Left => {
                return Verdict::PanFrom(ev.position);
            }
            _ => {}
        }
        // Compared whole: the view is told about any of the three, and the two
        // it does not draw a cursor from still decide what it holds open.
        if self.armed.0.get() == was {
            Verdict::Ignored
        } else {
            Verdict::Rearmed
        }
    }
}

impl Plugin for SpacePanPlugin {
    fn name(&self) -> &'static str {
        "daruda_space_pan"
    }

    fn priority(&self) -> i32 {
        PRIORITY
    }

    fn on_event(&mut self, event: &FlowEvent, ctx: &mut PluginContext) -> EventResult {
        let FlowEvent::Input(input) = event else {
            return EventResult::Continue;
        };
        match self.absorb(input, ctx.has_interaction()) {
            Verdict::Ignored => EventResult::Continue,
            Verdict::Rearmed => {
                // Or the cursor changes on whatever repaints the pane next,
                // which for a key nobody else answers is nothing at all.
                ctx.notify();
                EventResult::Continue
            }
            Verdict::PanFrom(start_mouse) => {
                self.armed.set_panning(true);
                ctx.start_interaction(SpacePan {
                    start_mouse,
                    start_offset: ctx.offset(),
                    armed: self.armed.clone(),
                });
                EventResult::Stop
            }
        }
    }
}

/// The drag itself: the offset follows the pointer by the distance it has
/// moved since the button went down.
struct SpacePan {
    start_mouse: Point<Pixels>,
    start_offset: Point<Pixels>,
    /// Held so the closing hand outlives the press that started it: the key
    /// may be released mid-drag, and the pan carries on regardless.
    armed: PanArmed,
}

impl Interaction for SpacePan {
    fn on_mouse_move(
        &mut self,
        ev: &gpui::MouseMoveEvent,
        ctx: &mut PluginContext,
    ) -> InteractionResult {
        ctx.set_offset(Point::new(
            self.start_offset.x + (ev.position.x - self.start_mouse.x),
            self.start_offset.y + (ev.position.y - self.start_mouse.y),
        ));
        ctx.notify();
        InteractionResult::Continue
    }

    fn on_mouse_up(
        &mut self,
        _event: &gpui::MouseUpEvent,
        ctx: &mut PluginContext,
    ) -> InteractionResult {
        // Back to whatever the key alone says — an open hand if it is still
        // down, nothing if it was let go while the drag ran.
        self.armed.set_panning(false);
        // No command: see the module doc — a pan is not an edit to undo.
        ctx.cancel_interaction();
        ctx.notify();
        InteractionResult::End
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Keystroke, Modifiers, MouseDownEvent, px};

    /// Not dragging — what most of these are about.
    const IDLE: bool = false;

    fn key(name: &str) -> Keystroke {
        Keystroke {
            modifiers: Modifiers::default(),
            key: name.to_string(),
            key_char: None,
        }
    }

    fn press(button: MouseButton) -> InputEvent {
        InputEvent::MouseDown(MouseDownEvent {
            button,
            position: Point::new(px(10.0), px(10.0)),
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        })
    }

    fn down(name: &str) -> InputEvent {
        InputEvent::KeyDown(gpui::KeyDownEvent {
            keystroke: key(name),
            is_held: false,
            prefer_character_input: false,
        })
    }

    fn up(name: &str) -> InputEvent {
        InputEvent::KeyUp(gpui::KeyUpEvent {
            keystroke: key(name),
        })
    }

    fn plugin() -> SpacePanPlugin {
        SpacePanPlugin::new(PanArmed::default())
    }

    fn pans(v: Verdict) -> bool {
        matches!(v, Verdict::PanFrom(_))
    }

    fn rearms(v: Verdict) -> bool {
        matches!(v, Verdict::Rearmed)
    }

    #[test]
    fn a_drag_pans_only_while_the_key_is_down() {
        let mut p = plugin();
        assert!(!pans(p.absorb(&press(MouseButton::Left), IDLE)), "not held");

        assert!(rearms(p.absorb(&down(PAN_KEY), IDLE)), "the cursor changes");
        assert!(pans(p.absorb(&press(MouseButton::Left), IDLE)));

        assert!(rearms(p.absorb(&up(PAN_KEY), IDLE)));
        assert!(!pans(p.absorb(&press(MouseButton::Left), IDLE)), "released");
    }

    /// Holding a key repeats its key-down. Only the first changes anything, or
    /// the pane would be told to repaint on every repeat.
    #[test]
    fn holding_the_key_down_rearms_once() {
        let mut p = plugin();
        assert!(rearms(p.absorb(&down(PAN_KEY), IDLE)));
        for _ in 0..3 {
            assert!(
                matches!(p.absorb(&down(PAN_KEY), IDLE), Verdict::Ignored),
                "already armed"
            );
        }
        assert!(pans(p.absorb(&press(MouseButton::Left), IDLE)));
    }

    /// Another key must not arm it, or every keystroke over the canvas would
    /// turn the next click into a pan.
    #[test]
    fn another_key_arms_nothing() {
        let mut p = plugin();
        p.absorb(&down("a"), IDLE);
        assert!(!pans(p.absorb(&press(MouseButton::Left), IDLE)));
    }

    /// A key-up for something else must not disarm it either — the two keys
    /// are tracked apart, and space is still down.
    #[test]
    fn another_keys_release_leaves_it_armed() {
        let mut p = plugin();
        p.absorb(&down(PAN_KEY), IDLE);
        p.absorb(&up("a"), IDLE);
        assert!(pans(p.absorb(&press(MouseButton::Left), IDLE)));
    }

    /// **The stuck-key guard.** Leaving the canvas with the key down sends its
    /// release to whoever takes the focus, so the plugin would never hear it
    /// and every later click would pan instead of select.
    #[test]
    fn leaving_the_canvas_forgets_the_key() {
        let mut p = plugin();
        p.absorb(&down(PAN_KEY), IDLE);
        assert!(rearms(p.absorb(&InputEvent::Hover(false), IDLE)));
        assert!(!pans(p.absorb(&press(MouseButton::Left), IDLE)));
    }

    /// **And the limit on that guard.** The canvas gives an interaction only
    /// the moves and the release, so the hover still arrives mid-pan — and
    /// dragging a graph past the pane's edge is ordinary. Disarming there
    /// would cost a release and a fresh press before the next pan.
    #[test]
    fn leaving_the_canvas_mid_pan_keeps_the_key() {
        let mut p = plugin();
        p.absorb(&down(PAN_KEY), IDLE);
        let dragging = true;
        assert!(matches!(
            p.absorb(&InputEvent::Hover(false), dragging),
            Verdict::Ignored
        ));
        assert!(
            pans(p.absorb(&press(MouseButton::Left), IDLE)),
            "still armed"
        );
    }

    /// The right button is the context menu's, and the middle one already
    /// pans on its own.
    #[test]
    fn only_the_left_button_pans() {
        let mut p = plugin();
        p.absorb(&down(PAN_KEY), IDLE);
        assert!(!pans(p.absorb(&press(MouseButton::Right), IDLE)));
    }

    /// **A pan must not outlive its interaction.** Only the release clears
    /// `panning`, so an interaction ended any other way — cancelled by another
    /// plugin, or a canvas rebuilt under it — would leave the closed hand drawn
    /// over a canvas nobody is dragging, with nothing left to take it off.
    #[test]
    fn a_pan_without_an_interaction_is_put_right_on_the_next_event() {
        let mut p = plugin();
        p.absorb(&down(PAN_KEY), IDLE);
        assert!(pans(p.absorb(&press(MouseButton::Left), IDLE)), "panning");
        // What `on_event` does with that verdict, and what the drag then is.
        p.armed.set_panning(true);
        let dragging = true;
        p.absorb(&up(PAN_KEY), dragging);
        assert_eq!(
            p.armed.cursor(),
            Some(CursorReach::Dragging(CursorStyle::ClosedHand)),
            "the drag outlives the key"
        );

        // The interaction is gone without a release having been seen.
        let verdict = p.absorb(&InputEvent::Hover(true), IDLE);
        assert!(
            rearms(verdict),
            "the view is told, or the hand stays drawn on a stale frame"
        );
        assert_eq!(
            p.armed.cursor(),
            None,
            "nothing is held and nothing is moving"
        );
    }

    /// And it must not cut a pan that is genuinely running.
    #[test]
    fn a_running_pan_is_left_alone() {
        let mut p = plugin();
        p.absorb(&down(PAN_KEY), IDLE);
        p.absorb(&press(MouseButton::Left), IDLE);
        p.armed.set_panning(true);
        let dragging = true;
        assert!(matches!(
            p.absorb(&InputEvent::Hover(false), dragging),
            Verdict::Ignored
        ));
        assert_eq!(
            p.armed.cursor(),
            Some(CursorReach::Dragging(CursorStyle::ClosedHand))
        );
    }

    /// The pane reads this to draw the cursor, so it has to follow the key
    /// rather than only the drag.
    #[test]
    fn the_key_alone_opens_the_hand() {
        let armed = PanArmed::default();
        assert_eq!(armed.cursor(), None, "the canvas draws its own pointer");

        armed.set_held(true);
        assert_eq!(
            armed.cursor(),
            Some(CursorReach::Hovered(CursorStyle::OpenHand))
        );

        armed.set_held(false);
        assert_eq!(armed.cursor(), None);
    }

    /// Moving the view closes it, and that beats the key: the two are read
    /// together and the drag is the more specific answer.
    #[test]
    fn a_running_pan_closes_the_hand() {
        let armed = PanArmed::default();
        armed.set_held(true);
        armed.set_panning(true);
        assert_eq!(
            armed.cursor(),
            Some(CursorReach::Dragging(CursorStyle::ClosedHand))
        );

        armed.set_panning(false);
        assert_eq!(
            armed.cursor(),
            Some(CursorReach::Hovered(CursorStyle::OpenHand)),
            "still held"
        );
    }

    /// **Why the two are kept apart.** A key-up reaches the plugin mid-drag —
    /// the canvas gives an interaction only the moves and the release — but
    /// the pan carries on to the button, so the hand has to stay closed and
    /// then leave the pointer alone rather than reopening.
    #[test]
    fn letting_the_key_go_mid_pan_keeps_the_hand_closed() {
        let armed = PanArmed::default();
        armed.set_held(true);
        armed.set_panning(true);

        armed.set_held(false);
        assert_eq!(
            armed.cursor(),
            Some(CursorReach::Dragging(CursorStyle::ClosedHand)),
            "the drag outlives the key"
        );

        armed.set_panning(false);
        assert_eq!(armed.cursor(), None, "and nothing is armed after it");
    }
}
