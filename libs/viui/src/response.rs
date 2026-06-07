//! `Response` — result of a widget's event processing, returned to the Elm runner.
//!
//! In the Elm facade (P06), the runner checks `Response` after `event()` to decide
//! whether to call `app.update(msg)` and request a repaint.

use crate::layout::Rect;
use crate::widget::WidgetId;

/// Result of processing pointer/keyboard events on an interactive widget.
#[derive(Clone, Debug, Default)]
pub struct Response {
    /// True on the frame a click is registered (press + release on same widget).
    pub clicked: bool,
    /// True while the pointer is within the widget's bounds.
    pub hovered: bool,
    /// True when the widget's value changed this frame (TextEdit, Checkbox, Slider).
    pub changed: bool,
    /// The widget's screen-space bounds (useful for positioning pop-overs).
    pub rect: Rect,
    /// Widget that produced this response.
    pub id: WidgetId,
}

impl Response {
    /// True if the user triggered an interaction: click, change, or any input.
    pub fn interacted(&self) -> bool { self.clicked || self.changed }
}
