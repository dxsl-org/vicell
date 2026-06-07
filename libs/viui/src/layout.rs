//! Geometry primitives, Length, Constraints, and LayoutNode.
//!
//! Two-pass layout (iced + OrbTK ref):
//!   `ViWidget::layout(constraints)` — builds full LayoutNode tree top-down.
//!   Container widgets compute children sizes + origins; leaf widgets return their desired size.
//!
//! All bounds in LayoutNode are **screen-space** (absolute coords).
//! `constraints.origin` carries the top-left of the allocated slot.

extern crate alloc;
use alloc::vec::Vec;

// ─── Geometry ────────────────────────────────────────────────────────────────

#[derive(Copy, Clone, Default, PartialEq, Debug)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

impl Size {
    pub const ZERO: Self = Self { w: 0.0, h: 0.0 };
    pub const fn new(w: f32, h: f32) -> Self { Self { w, h } }
}

#[derive(Copy, Clone, Default, PartialEq, Debug)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };
    pub const fn new(x: f32, y: f32) -> Self { Self { x, y } }
    pub fn offset(self, dx: f32, dy: f32) -> Self { Self { x: self.x + dx, y: self.y + dy } }
}

/// Axis-aligned rectangle with float coordinates for layout precision.
#[derive(Copy, Clone, Default, PartialEq, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self { Self { x, y, w, h } }
    pub fn from_origin_size(origin: Point, size: Size) -> Self {
        Self { x: origin.x, y: origin.y, w: size.w, h: size.h }
    }
    pub fn origin(self) -> Point { Point { x: self.x, y: self.y } }
    pub fn size(self) -> Size { Size { w: self.w, h: self.h } }

    pub fn contains(self, p: Point) -> bool {
        p.x >= self.x && p.x < self.x + self.w && p.y >= self.y && p.y < self.y + self.h
    }

    pub fn intersects(self, other: Rect) -> bool {
        self.x < other.x + other.w
            && self.x + self.w > other.x
            && self.y < other.y + other.h
            && self.y + self.h > other.y
    }

    /// Intersection of two rects, or `None` if they do not overlap.
    pub fn intersect(self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let x2 = (self.x + self.w).min(other.x + other.w);
        let y2 = (self.y + self.h).min(other.y + other.h);
        if x2 > x && y2 > y {
            Some(Rect { x, y, w: x2 - x, h: y2 - y })
        } else {
            None
        }
    }

    /// Smallest axis-aligned rectangle enclosing both rects.
    pub fn union(self, other: Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let x2 = (self.x + self.w).max(other.x + other.w);
        let y2 = (self.y + self.h).max(other.y + other.h);
        Rect { x, y, w: (x2 - x).max(0.0), h: (y2 - y).max(0.0) }
    }

    pub fn translate(self, dx: f32, dy: f32) -> Rect {
        Rect { x: self.x + dx, y: self.y + dy, ..self }
    }

    /// Shrink rect by uniform inset on all sides.
    pub fn inset(self, v: f32) -> Rect {
        Rect { x: self.x + v, y: self.y + v, w: (self.w - 2.0 * v).max(0.0), h: (self.h - 2.0 * v).max(0.0) }
    }
}

// ─── Length ──────────────────────────────────────────────────────────────────

/// How a widget declares its preferred size along one axis.
/// Mirrors `iced_core::Length` for API compatibility.
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum Length {
    /// Expand to fill all remaining available space.
    Fill,
    /// Shrink to minimum content size.
    Shrink,
    /// Exact pixel size.
    Fixed(f32),
    /// Proportional fill: `FillPortion(n)` takes `n×` the unit share.
    FillPortion(u16),
}

impl Default for Length { fn default() -> Self { Self::Shrink } }

// ─── Constraints ─────────────────────────────────────────────────────────────

/// Space available to a widget during the layout pass.
///
/// `origin` is the top-left screen-space position assigned by the parent.
/// `min`/`max` bound the widget's allowed size.
#[derive(Copy, Clone, Debug)]
pub struct Constraints {
    /// Screen-space top-left corner for this widget's slot.
    pub origin: Point,
    pub min: Size,
    pub max: Size,
}

impl Constraints {
    pub fn new(origin: Point, max: Size) -> Self {
        Self { origin, min: Size::ZERO, max }
    }

    pub fn root(screen: Size) -> Self {
        Self { origin: Point::ZERO, min: Size::ZERO, max: screen }
    }

    /// Clamp `desired` to fit within these constraints.
    pub fn constrain(self, desired: Size) -> Size {
        Size {
            w: desired.w.clamp(self.min.w, self.max.w),
            h: desired.h.clamp(self.min.h, self.max.h),
        }
    }

    /// Return inner constraints after stripping horizontal + vertical padding.
    pub fn shrink(self, dw: f32, dh: f32) -> Self {
        Self {
            origin: self.origin,
            min: Size::ZERO,
            max: Size { w: (self.max.w - dw).max(0.0), h: (self.max.h - dh).max(0.0) },
        }
    }

    /// Constraints for a child at `offset` within this slot.
    pub fn child(self, offset: Point, max: Size) -> Self {
        Self {
            origin: Point { x: self.origin.x + offset.x, y: self.origin.y + offset.y },
            min: Size::ZERO,
            max,
        }
    }
}

// ─── LayoutNode ──────────────────────────────────────────────────────────────

/// Output of `ViWidget::layout()` — screen-space bounds + children recursively.
/// Mirrors `iced_core::layout::Node`.
#[derive(Debug)]
pub struct LayoutNode {
    /// Absolute screen-space bounding box of this widget.
    pub bounds: Rect,
    pub children: Vec<LayoutNode>,
}

impl LayoutNode {
    pub fn leaf(bounds: Rect) -> Self { Self { bounds, children: Vec::new() } }
    pub fn with_children(bounds: Rect, children: Vec<LayoutNode>) -> Self {
        Self { bounds, children }
    }
}

/// Read-only view into a LayoutNode sub-tree — used in EventCx for hit testing.
#[derive(Copy, Clone)]
pub struct LayoutView<'a>(pub &'a LayoutNode);

impl<'a> LayoutView<'a> {
    pub fn bounds(self) -> Rect { self.0.bounds }
    pub fn child(self, i: usize) -> Option<LayoutView<'a>> {
        self.0.children.get(i).map(LayoutView)
    }
    pub fn children_count(self) -> usize { self.0.children.len() }
}

// ─── Padding / Axis ──────────────────────────────────────────────────────────

#[derive(Copy, Clone, Default, Debug)]
pub struct Padding {
    pub top: f32, pub right: f32, pub bottom: f32, pub left: f32,
}

impl Padding {
    pub const ZERO: Self = Self { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 };
    pub fn all(v: f32) -> Self { Self { top: v, right: v, bottom: v, left: v } }
    pub fn h_total(self) -> f32 { self.left + self.right }
    pub fn v_total(self) -> f32 { self.top + self.bottom }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Axis { Horizontal, Vertical }
