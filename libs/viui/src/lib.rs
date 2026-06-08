//! ViUI — ViCell native UI toolkit.
//!
//! no_std Elm-architecture UI toolkit with direct pixel rendering.
//! API shape compatible with iced (iced-compatible Elm model).
//!
//! ## Crate layout
//! - `layout`      — geometry (Size/Point/Rect), Length, Constraints, LayoutNode
//! - `canvas`      — ViCanvas trait, Color, TextStyle
//! - `state_store` — WidgetFlags, WidgetState, WidgetStateStore, FocusManager
//! - `event`       — Event enum, EventStatus, EventCx
//! - `widget`      — ViWidget trait, WidgetId (FNV-1a), PaintCx, WidgetTree
//! - `response`    — Response
//! - `elm`         — ViApp trait, Element<Msg>
//! - `prelude`     — common re-exports
//!
//! ## Phase additions
//! - P02: FramebufferCanvas in canvas.rs (software rasterizer)
//! - P03: PaintCx gains font/atlas fields; GlyphAtlas in libs/ostd
//! - P04: pub mod widgets (Label, Button, TextEdit, Checkbox, ScrollArea, Image)
//! - P05: pub mod theme (ViTheme trait, Dark/Light/Kiosk)
//! - P06: run_app + free-function builders in elm.rs
//! - P07: pub mod window (WindowChrome, WindowManager)
//!
//! ## ViUI v2 additions (Reactive Signal Tree)
//! - signal       — Signal<T>, Computed<T>, SubscriptionHandle
//! - dirty        — DirtyRect partial-repaint accumulator
//! - renderer     — ViRenderer trait, FramebufferRenderer
//! - node         — ViNode trait (v2 widget interface)
//! - node_widgets — Label, Button, Column, Row (typed Signal-driven widgets)
//! - app_runner   — ViApp tick-based runner

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod animation;
pub mod app_runner;
pub mod canvas;
pub mod dirty;
pub mod elm;
pub mod executor;
pub mod event;
pub mod font_context;
pub mod gpu_canvas;
pub mod gpu_cmd;
pub mod gpu_renderer;
pub mod layout;
pub mod node;
pub mod node_widgets;
pub mod prelude;
pub mod render_ctx;
pub mod renderer;
pub mod response;
pub mod signal;
pub mod state_store;
pub mod theme;
pub mod widget;
pub mod widgets;
pub mod window;

// ─── GPU renderer convenience re-exports ─────────────────────────────────────
pub use executor::{CommandExecutor, CpuExecutor};
pub use gpu_renderer::GpuRenderer;

// ─── ViUI v2 macros ───────────────────────────────────────────────────────────
//
// vi_design! proc macro: inline .vi DSL component declaration.
// Re-exported from viui-macros so callers need only depend on viui.
pub use viui_macros::vi_design;


/// Create a `Column` (vertical stack) from widget expressions.
///
/// ```rust,ignore
/// let ui = vstack!(label, button, label2);
/// ```
#[macro_export]
macro_rules! vstack {
    ($($child:expr),* $(,)?) => {
        $crate::node_widgets::column::Column::new(
            alloc::vec![
                $( alloc::boxed::Box::new($child)
                   as alloc::boxed::Box<dyn $crate::node::ViNode> ),*
            ]
        )
    };
}

/// Create a `Row` (horizontal stack) from widget expressions.
///
/// ```rust,ignore
/// let toolbar = hstack!(back_btn, title_label, menu_btn);
/// ```
#[macro_export]
macro_rules! hstack {
    ($($child:expr),* $(,)?) => {
        $crate::node_widgets::row::Row::new(
            alloc::vec![
                $( alloc::boxed::Box::new($child)
                   as alloc::boxed::Box<dyn $crate::node::ViNode> ),*
            ]
        )
    };
}
