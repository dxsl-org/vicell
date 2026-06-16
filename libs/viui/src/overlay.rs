// SPDX-License-Identifier: MIT
//! Overlay layer — shared action queue + entry types for Dialog, DropDown, Toast.
//!
//! # Design
//!
//! Widgets cannot mutate `ViApp` directly (borrow conflict: `root.event()` holds
//! `&mut self.root` while `tick_with_dt` holds `&mut self`). Instead, widgets
//! close over an `OverlayActionQueue` clone and push `OverlayAction` items into
//! it during their `event()` call. After all events are processed, `ViApp`
//! drains the queue and executes the deferred actions.

extern crate alloc;
use alloc::{boxed::Box, vec::Vec};
use alloc::rc::Rc;
use core::cell::RefCell;

use crate::layout::Rect;
use crate::node::ViNode;

// ─── OverlayEntry ────────────────────────────────────────────────────────────

/// One overlay widget pushed onto the layer stack.
pub struct OverlayEntry {
    /// The widget drawn on top of the base UI.
    pub widget: Box<dyn ViNode>,

    /// When `true` this overlay intercepts ALL input — nothing below it receives
    /// events. Use for modal dialogs.
    pub blocking: bool,

    /// When `true` a click outside `widget.bounds()` automatically pops this
    /// overlay. Use for drop-down menus and non-modal panels.
    pub dismiss_outside: bool,

    /// If `Some`, the widget's layout origin is constrained relative to this
    /// anchor rect (e.g. position a drop-down below its trigger button).
    /// When `None` the widget lays out against the full screen constraints.
    pub anchor_bounds: Option<Rect>,
}

// ─── OverlayAction ───────────────────────────────────────────────────────────

/// Deferred action pushed by a widget during `event()` and executed by `ViApp`
/// after the event loop completes. Avoids borrow conflicts on `ViApp`.
pub enum OverlayAction {
    /// Push a new overlay on top of the stack.
    Push(OverlayEntry),

    /// Pop the topmost overlay.
    Pop,

    /// Pop every overlay at once (e.g. Escape key handler).
    PopAll,

    /// Show a transient toast notification.
    ShowToast(crate::node_widgets::toast::ToastConfig),
}

// ─── OverlayActionQueue ──────────────────────────────────────────────────────

/// Shared, interior-mutable queue of deferred overlay actions.
///
/// Clone this to hand a reference to any widget that needs overlay access.
/// `ViApp` drains it once per frame after the event loop.
pub type OverlayActionQueue = Rc<RefCell<Vec<OverlayAction>>>;

/// Create a new, empty action queue.
pub fn new_action_queue() -> OverlayActionQueue {
    Rc::new(RefCell::new(Vec::new()))
}
