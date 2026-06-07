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

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod canvas;
pub mod elm;
pub mod event;
pub mod layout;
pub mod prelude;
pub mod response;
pub mod state_store;
pub mod theme;
pub mod widget;
pub mod widgets;
pub mod window;
