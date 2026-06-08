// SPDX-License-Identifier: MIT
//! ViUI v2 typed widget structs — Layer 2 Rust API.
//!
//! These widgets use `Signal<T>` properties and `Box<dyn Fn()>` callbacks
//! instead of the Elm rebuild model in `crate::widgets` (v1).

pub mod button;
pub mod card;
pub mod checkbox;
pub mod column;
pub mod divider;
pub mod image;
pub mod label;
pub mod list_view;
pub mod progress_bar;
pub mod row;
pub mod scroll_area;
pub mod slider;
pub mod space;
pub mod text_edit;
pub mod touch_area;

pub use card::Card;
pub use checkbox::CheckBox;
pub use divider::Divider;
pub use image::Image;
pub use list_view::ListView;
pub use scroll_area::ScrollArea;
pub use space::Space;
pub use text_edit::TextEdit;
