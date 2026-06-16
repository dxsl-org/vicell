// SPDX-License-Identifier: MIT
//! ViUI v2 typed widget structs — Layer 2 Rust API.
//!
//! These widgets use `Signal<T>` properties and `Box<dyn Fn()>` callbacks
//! instead of the Elm rebuild model in `crate::widgets` (v1).

pub mod button;
pub mod card;
pub mod checkbox;
pub mod column;
pub mod dialog;
pub mod divider;
pub mod dropdown;
pub mod flex_box;
pub mod image;
pub mod label;
pub mod list_view;
pub mod progress_bar;
pub mod row;
pub mod scroll_area;
pub mod slider;
pub mod space;
pub mod text_edit;
pub mod toast;
pub mod touch_area;

pub use card::Card;
pub use checkbox::CheckBox;
pub use dialog::Dialog;
pub use divider::Divider;
pub use dropdown::DropDown;
pub use flex_box::{FlexBox, FlexDirection, FlexItem};
pub use image::Image;
pub use list_view::ListView;
pub use scroll_area::ScrollArea;
pub use space::Space;
pub use text_edit::TextEdit;
pub use toast::{Toast, ToastConfig, ToastKind};
