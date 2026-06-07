//! `ViTheme` trait and built-in theme implementations.
//!
//! Widgets read colors/spacing from `cx.theme` instead of hardcoding values.

use crate::canvas::Color;

// ─── ViTheme trait ───────────────────────────────────────────────────────────

pub trait ViTheme: 'static {
    // Surface colours
    fn bg(&self)      -> Color;
    fn surface(&self) -> Color;    // card / panel background
    fn border(&self)  -> Color;

    // Text colours
    fn text_primary(&self)   -> Color;
    fn text_secondary(&self) -> Color;

    // Interactive colours
    fn accent(&self)          -> Color;
    fn button_normal(&self)   -> Color;
    fn button_hovered(&self)  -> Color;
    fn button_pressed(&self)  -> Color;
    fn input_bg(&self)        -> Color;
    fn input_focused_bg(&self) -> Color;
    fn input_focused_border(&self) -> Color;

    // Spacing
    fn padding_sm(&self) -> f32 { 4.0 }
    fn padding_md(&self) -> f32 { 8.0 }
    fn padding_lg(&self) -> f32 { 16.0 }

    // Font sizes (px). 0 = bitmap 8×8 fallback.
    fn font_size_body(&self)    -> u16 { 0 }
    fn font_size_heading(&self) -> u16 { 0 }
}

// ─── DarkTheme ───────────────────────────────────────────────────────────────

pub struct DarkTheme;

impl ViTheme for DarkTheme {
    fn bg(&self)      -> Color { Color::rgb(18, 18, 24) }
    fn surface(&self) -> Color { Color::rgb(30, 30, 40) }
    fn border(&self)  -> Color { Color::rgb(60, 60, 80) }

    fn text_primary(&self)   -> Color { Color::WHITE }
    fn text_secondary(&self) -> Color { Color::rgb(160, 160, 180) }

    fn accent(&self)          -> Color { Color::rgb(80, 120, 220) }
    fn button_normal(&self)   -> Color { Color::rgb(45, 45, 60) }
    fn button_hovered(&self)  -> Color { Color::rgb(65, 65, 90) }
    fn button_pressed(&self)  -> Color { Color::rgb(80, 100, 180) }
    fn input_bg(&self)        -> Color { Color::rgb(20, 20, 30) }
    fn input_focused_bg(&self) -> Color { Color::rgb(15, 15, 50) }
    fn input_focused_border(&self) -> Color { Color::rgb(90, 110, 210) }
}

// ─── LightTheme ──────────────────────────────────────────────────────────────

pub struct LightTheme;

impl ViTheme for LightTheme {
    fn bg(&self)      -> Color { Color::rgb(245, 245, 248) }
    fn surface(&self) -> Color { Color::WHITE }
    fn border(&self)  -> Color { Color::rgb(200, 200, 210) }

    fn text_primary(&self)   -> Color { Color::rgb(20, 20, 30) }
    fn text_secondary(&self) -> Color { Color::rgb(100, 100, 120) }

    fn accent(&self)          -> Color { Color::rgb(60, 100, 200) }
    fn button_normal(&self)   -> Color { Color::rgb(220, 220, 230) }
    fn button_hovered(&self)  -> Color { Color::rgb(200, 205, 230) }
    fn button_pressed(&self)  -> Color { Color::rgb(170, 185, 230) }
    fn input_bg(&self)        -> Color { Color::WHITE }
    fn input_focused_bg(&self) -> Color { Color::rgb(240, 244, 255) }
    fn input_focused_border(&self) -> Color { Color::rgb(60, 100, 200) }
}

// ─── KioskTheme ──────────────────────────────────────────────────────────────
//
// High contrast, large touch targets (≥44×44px), minimum font size 16px.

pub struct KioskTheme;

impl ViTheme for KioskTheme {
    fn bg(&self)      -> Color { Color::BLACK }
    fn surface(&self) -> Color { Color::rgb(20, 20, 20) }
    fn border(&self)  -> Color { Color::WHITE }

    fn text_primary(&self)   -> Color { Color::WHITE }
    fn text_secondary(&self) -> Color { Color::GRAY }

    fn accent(&self)          -> Color { Color::YELLOW }
    fn button_normal(&self)   -> Color { Color::rgb(40, 40, 40) }
    fn button_hovered(&self)  -> Color { Color::GRAY }
    fn button_pressed(&self)  -> Color { Color::rgb(180, 160, 0) }
    fn input_bg(&self)        -> Color { Color::rgb(10, 10, 10) }
    fn input_focused_bg(&self) -> Color { Color::rgb(0, 0, 30) }
    fn input_focused_border(&self) -> Color { Color::YELLOW }

    fn padding_sm(&self) -> f32 { 8.0 }
    fn padding_md(&self) -> f32 { 16.0 }
    fn padding_lg(&self) -> f32 { 24.0 }
}

// ─── Default theme instance ──────────────────────────────────────────────────

/// Default dark theme used when no theme is provided to PaintCx.
pub static DARK_THEME: DarkTheme = DarkTheme;
