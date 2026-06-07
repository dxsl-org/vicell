//! Core ViWidget trait, WidgetId, PaintCx, and WidgetTree.

extern crate alloc;
use alloc::boxed::Box;

use crate::canvas::ViCanvas;
use crate::event::{Event, EventCx, EventStatus};
use crate::layout::{Constraints, LayoutNode, LayoutView, Point, Rect, Size};
use crate::state_store::{FocusManager, WidgetStateStore};
use crate::theme::{ViTheme, DARK_THEME};

// ─── WidgetId ────────────────────────────────────────────────────────────────

/// Stable hash-based widget identity.
///
/// FNV-1a 64-bit — no alloc, no_std, O(1) per construction.
/// egui `Id` pattern: hash-based IDs survive tree structural changes.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub struct WidgetId(pub u64);

impl WidgetId {
    /// Sentinel root ID for WidgetTree dispatch.
    pub const ROOT: Self = Self(0);

    /// Create from a stable string salt (module path, user-provided label, etc.).
    pub fn new(salt: &str) -> Self {
        Self(fnv1a(salt.as_bytes()))
    }

    /// Derive a child ID from this one by appending an index.
    /// Used by container widgets to give stable IDs to indexed children.
    pub fn child(self, index: usize) -> Self {
        let mut h = self.0;
        for b in index.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
        Self(h)
    }

    /// Derive a named child ID.
    pub fn with(self, salt: &str) -> Self {
        Self(self.0.wrapping_add(fnv1a(salt.as_bytes())))
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

// ─── PaintCx ─────────────────────────────────────────────────────────────────

/// Context passed to `ViWidget::paint()`.
pub struct PaintCx<'a> {
    pub canvas: &'a mut dyn ViCanvas,
    /// Screen-space origin of the current widget's (0, 0) corner.
    pub origin: Point,
    /// Active theme. Widgets read colours/spacing from here instead of hardcoding.
    pub theme: &'a dyn ViTheme,
}

impl<'a> PaintCx<'a> {
    /// Root PaintCx with `DarkTheme` default and origin at (0, 0).
    pub fn root(canvas: &'a mut dyn ViCanvas) -> Self {
        Self { canvas, origin: Point::ZERO, theme: &DARK_THEME }
    }

    /// Root PaintCx with an explicit theme.
    pub fn with_theme(canvas: &'a mut dyn ViCanvas, theme: &'a dyn ViTheme) -> Self {
        Self { canvas, origin: Point::ZERO, theme }
    }

    /// Create a child PaintCx at `child_origin`, inheriting canvas and theme.
    pub fn child_at(&mut self, child_origin: Point) -> PaintCx<'_> {
        PaintCx { canvas: self.canvas, origin: child_origin, theme: self.theme }
    }

    /// Translate a widget-local rect to screen-space.
    pub fn screen_rect(&self, local: Rect) -> Rect {
        local.translate(self.origin.x, self.origin.y)
    }
}

// ─── ViWidget ────────────────────────────────────────────────────────────────

/// Core widget trait.
///
/// Every UI element implements this. Container widgets (Column, Row in P04)
/// hold `Vec<Box<dyn ViWidget>>` and recurse into children in `layout()`,
/// `paint()`, and `event()`.
///
/// Lifecycle (OrbTK State ref):
///   `on_mount()` → once when inserted into tree.
///   `layout()` → each frame (or when dirty).
///   `post_layout()` → after layout, before paint. Widget knows its final bounds.
///   `paint()` → render.
///   `event()` → input routing.
///   `on_unmount()` → once when removed from tree (Law 8: RAII).
pub trait ViWidget: 'static {
    /// Compute layout for this widget + all children given `constraints`.
    ///
    /// Returns screen-space `LayoutNode`. `constraints.origin` is the top-left
    /// of the allocated slot; the returned `bounds` must fit within `constraints.max`.
    fn layout(&self, constraints: Constraints) -> LayoutNode;

    /// Paint this widget (and its children) into `cx.canvas`.
    fn paint(&self, cx: &mut PaintCx);

    /// Process one input event. Return `Consumed` to stop bubbling.
    fn event(&mut self, cx: &mut EventCx, e: &Event) -> EventStatus;

    /// Called after `layout()`, before `paint()`. Widget knows its final `bounds`.
    ///
    /// Use cases: `ScrollArea` clamps scroll_y after knowing visible height;
    /// `TextEdit` computes cursor pixel position after knowing text box width.
    #[allow(unused_variables)]
    fn post_layout(&mut self, bounds: Rect) {}

    /// Called once when this widget is inserted into the live tree.
    fn on_mount(&mut self) {}

    /// Called once when this widget is removed from the live tree (RAII cleanup, Law 8).
    fn on_unmount(&mut self) {}
}

// ─── WidgetTree ──────────────────────────────────────────────────────────────

/// Retained widget tree with layout cache, state store, and focus manager.
///
/// Elm rebuild strategy: `WidgetTree::rebuild(root)` is called after each `app.update()`.
/// The new `root` replaces the old one cheaply (just a pointer swap).
/// `WidgetStateStore` (egui Memory) persists across rebuilds via hash-stable WidgetIds.
pub struct WidgetTree {
    root: Box<dyn ViWidget>,
    layout_cache: LayoutNode,
    pub state: WidgetStateStore,
    dirty: Option<Rect>,
    pub focus: FocusManager,
}

impl WidgetTree {
    /// Create a new tree from `root`. Call `layout()` before painting.
    pub fn rebuild(root: Box<dyn ViWidget>) -> Self {
        Self {
            root,
            layout_cache: LayoutNode::leaf(Rect::ZERO),
            state: WidgetStateStore::new(),
            dirty: None,
            focus: FocusManager::new(),
        }
    }

    /// Replace the root widget while preserving state and focus.
    pub fn update_root(&mut self, root: Box<dyn ViWidget>) {
        self.root = root;
    }

    /// Run the layout pass with the available screen area.
    ///
    /// Must be called before `paint()` or `dispatch_event()`.
    pub fn layout(&mut self, available: Size) {
        let constraints = Constraints::root(available);
        self.layout_cache = self.root.layout(constraints);
        // post_layout pass — widget receives its final bounds
        self.root.post_layout(self.layout_cache.bounds);
    }

    /// Paint the widget tree.
    pub fn paint(&self, cx: &mut PaintCx) {
        self.root.paint(cx);
    }

    /// Dispatch one input event to the widget tree.
    ///
    /// Returns `true` if any widget requested a repaint.
    pub fn dispatch_event(&mut self, e: &Event) -> bool {
        // Struct destructuring lets the borrow checker see separate field borrows.
        let Self { root, layout_cache, state, dirty, focus } = self;
        let mut cx = EventCx {
            state,
            focus,
            widget_id: WidgetId::ROOT,
            layout: LayoutView(layout_cache),
            needs_repaint: false,
        };
        root.event(&mut cx, e);
        let needs_repaint = cx.needs_repaint;
        if needs_repaint {
            let bounds = layout_cache.bounds;
            *dirty = Some(match dirty.take() { Some(acc) => acc.union(bounds), None => bounds });
        }
        needs_repaint
    }

    /// Take and clear the accumulated dirty rect.
    pub fn take_dirty(&mut self) -> Option<Rect> { self.dirty.take() }

    /// Mark a screen-space rect as needing repaint.
    pub fn mark_dirty(&mut self, rect: Rect) {
        self.dirty = Some(match self.dirty { Some(acc) => acc.union(rect), None => rect });
    }

    /// Root bounds (valid after calling `layout()`).
    pub fn root_bounds(&self) -> Rect { self.layout_cache.bounds }
}
