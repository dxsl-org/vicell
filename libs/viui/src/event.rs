//! Event types and dispatch context.
//!
//! Routing strategies (OrbTK ref):
//!   BottomUp — pointer events: deepest widget first, bubbles to root on Ignored.
//!   Direct   — keyboard events: dispatched only to the focused widget.
//!   Global   — MouseRelease also fired globally so Button can clear PRESSED state
//!              even when the pointer moved outside during drag.

use crate::layout::{LayoutView, Point, Rect};
use crate::state_store::{FocusManager, WidgetStateStore};
use crate::widget::WidgetId;

// ─── Input types ─────────────────────────────────────────────────────────────

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MouseButton { Left, Right, Middle }

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KeyCode {
    Backspace, Delete, Enter, Tab, Escape,
    Left, Right, Up, Down,
    Home, End, PageUp, PageDown,
    F(u8),
}

#[derive(Copy, Clone, Default, Debug)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

// ─── Event ───────────────────────────────────────────────────────────────────

/// All input events delivered to widgets.
#[derive(Clone, Debug)]
pub enum Event {
    // BottomUp — dispatched by hit-testing, bubbles from leaf to root
    MouseMove   { pos: Point },
    MousePress  { pos: Point, button: MouseButton },
    /// Also dispatched globally (GlobalRelease strategy) to clear PRESSED state.
    MouseRelease { pos: Point, button: MouseButton },
    Scroll      { pos: Point, delta_y: f32 },

    // Direct — dispatched only to the focused widget
    KeyPress   { key: KeyCode, modifiers: Modifiers },
    KeyRelease { key: KeyCode },
    Char(char),

    // Lifecycle events (Direct, fired on focus change)
    Focus,
    Blur,
}

impl Event {
    /// Returns the pointer position for pointer events, `None` for keyboard/lifecycle.
    pub fn pointer_pos(&self) -> Option<Point> {
        match self {
            Self::MouseMove { pos }
            | Self::MousePress { pos, .. }
            | Self::MouseRelease { pos, .. }
            | Self::Scroll { pos, .. } => Some(*pos),
            _ => None,
        }
    }

    /// True if this event should be dispatched globally (not just to the hit-target).
    pub fn is_global_release(&self) -> bool {
        matches!(self, Self::MouseRelease { .. })
    }
}

// ─── EventStatus ─────────────────────────────────────────────────────────────

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum EventStatus {
    /// Event was handled — stop bubbling.
    Consumed,
    /// Event was not handled — continue bubbling toward the root.
    Ignored,
}

// ─── EventCx ─────────────────────────────────────────────────────────────────

/// Context passed to `ViWidget::event()`.
///
/// Container widgets create a child EventCx via `cx.for_child()` when recursing.
pub struct EventCx<'a> {
    pub state: &'a mut WidgetStateStore,
    pub focus: &'a mut FocusManager,
    /// Stable hash ID of the widget currently being dispatched to.
    pub widget_id: WidgetId,
    /// Screen-space bounds of the current widget (from the layout tree).
    pub layout: LayoutView<'a>,
    /// Set to `true` by any widget that needs a repaint this frame.
    pub needs_repaint: bool,
}

impl<'a> EventCx<'a> {
    /// Screen-space bounds of the current widget.
    pub fn bounds(&self) -> Rect { self.layout.bounds() }

    /// Request a repaint this frame.
    pub fn mark_dirty(&mut self) { self.needs_repaint = true; }

    /// True if `id` currently holds keyboard focus.
    pub fn is_focused(&self, id: WidgetId) -> bool {
        self.focus.focused() == Some(id)
    }

    /// Give keyboard focus to `id`.
    pub fn set_focus(&mut self, id: WidgetId) { self.focus.set_focus(id); }

    /// Release keyboard focus.
    pub fn release_focus(&mut self) { self.focus.clear_focus(); }

    /// True if the pointer event position lies within this widget's bounds.
    pub fn hit_test(&self, pos: Point) -> bool {
        self.layout.bounds().contains(pos)
    }
}
