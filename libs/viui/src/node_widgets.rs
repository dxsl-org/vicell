// SPDX-License-Identifier: MIT
//! ViUI v2 typed widget structs — Layer 2 Rust API.
//!
//! These widgets use `Signal<T>` properties and `Box<dyn Fn()>` callbacks
//! instead of the Elm rebuild model in `crate::widgets` (v1).

pub mod button;
pub mod column;
pub mod label;
pub mod progress_bar;
pub mod row;
pub mod slider;
pub mod touch_area;
