//! Per-widget transient state, keyed by `WidgetId`.
//!
//! `WidgetStateStore` mirrors egui's `Memory` pattern:
//! state persists across `WidgetTree::rebuild()` calls because WidgetIds are hash-stable.
//! `gc()` removes stale entries after each view-rebuild cycle.

extern crate alloc;
use alloc::collections::BTreeMap;
use crate::widget::WidgetId;
use crate::layout::Point;

// ─── WidgetFlags ─────────────────────────────────────────────────────────────

/// Bitfield for interactive widget visual state.
///
/// OrbTK WidgetFlags pattern: state is combinatorial (hovered + focused simultaneously valid).
/// Widgets read flags from `WidgetStateStore` instead of storing booleans in their struct.
#[derive(Copy, Clone, Default, Debug)]
pub struct WidgetFlags(pub u8);

impl WidgetFlags {
    pub const HOVERED: u8 = 0b001;
    pub const PRESSED: u8 = 0b010;
    pub const FOCUSED: u8 = 0b100;

    pub fn has(self, flag: u8) -> bool { self.0 & flag != 0 }
    pub fn set(&mut self, flag: u8) { self.0 |= flag; }
    pub fn clear(&mut self, flag: u8) { self.0 &= !flag; }
}

// ─── WidgetState ─────────────────────────────────────────────────────────────

/// Transient interaction state for one widget, persisted across frame rebuilds.
#[derive(Default, Debug)]
pub struct WidgetState {
    /// Hover / press / focus flags.
    pub flags: WidgetFlags,
    /// Active drag start position (set on MousePress, cleared on release).
    pub drag_pos: Option<Point>,
    /// Vertical scroll offset — used by `ScrollArea` (P04).
    pub scroll_y: f32,
    /// Inline widget-specific payload (e.g. TextEdit cursor byte offset at `custom[0..8]`).
    pub custom: [u8; 32],
}

impl WidgetState {
    pub fn hovered(self) -> bool  { self.flags.has(WidgetFlags::HOVERED) }
    pub fn pressed(self) -> bool  { self.flags.has(WidgetFlags::PRESSED) }
    pub fn focused(self) -> bool  { self.flags.has(WidgetFlags::FOCUSED) }
}

// ─── WidgetStateStore ────────────────────────────────────────────────────────

/// Maps `WidgetId → WidgetState` across frame rebuilds.
#[derive(Default)]
pub struct WidgetStateStore {
    inner: BTreeMap<WidgetId, WidgetState>,
}

impl WidgetStateStore {
    pub fn new() -> Self { Self::default() }

    /// Get or create default state for `id`.
    pub fn entry(&mut self, id: WidgetId) -> &mut WidgetState {
        self.inner.entry(id).or_insert_with(WidgetState::default)
    }

    pub fn get(&self, id: WidgetId) -> Option<&WidgetState> {
        self.inner.get(&id)
    }

    pub fn get_mut(&mut self, id: WidgetId) -> Option<&mut WidgetState> {
        self.inner.get_mut(&id)
    }

    /// Remove state for widgets no longer present in the live tree.
    ///
    /// Should be called at the end of each `view()` rebuild cycle.
    pub fn gc(&mut self, live_ids: &[WidgetId]) {
        self.inner.retain(|id, _| live_ids.contains(id));
    }
}

// ─── FocusManager ────────────────────────────────────────────────────────────

/// Keyboard focus owner — tracks which widget receives `KeyPress`/`Char` events.
#[derive(Default)]
pub struct FocusManager {
    focused: Option<WidgetId>,
    tab_order: alloc::vec::Vec<WidgetId>,
}

impl FocusManager {
    pub fn new() -> Self { Self::default() }

    pub fn focused(&self) -> Option<WidgetId> { self.focused }

    pub fn set_focus(&mut self, id: WidgetId) {
        self.focused = Some(id);
    }

    pub fn clear_focus(&mut self) { self.focused = None; }

    /// Register a widget in tab order.
    pub fn register_tab(&mut self, id: WidgetId) {
        if !self.tab_order.contains(&id) { self.tab_order.push(id); }
    }

    /// Advance focus to the next widget in tab order.
    pub fn tab_next(&mut self) {
        if self.tab_order.is_empty() { return; }
        let next = match self.focused {
            None => 0,
            Some(cur) => {
                let pos = self.tab_order.iter().position(|&id| id == cur).unwrap_or(0);
                (pos + 1) % self.tab_order.len()
            }
        };
        self.focused = Some(self.tab_order[next]);
    }
}
