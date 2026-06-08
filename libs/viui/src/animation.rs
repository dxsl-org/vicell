// SPDX-License-Identifier: MIT
//! Animation engine — easing, tweens, and animated signals for ViUI v2.
//!
//! # Design
//!
//! Animations are driven by the caller supplying elapsed milliseconds via
//! `ViApp::tick_with_dt(events, dt_ms)`. Each registered `Animatable` is
//! advanced by `dt_ms` per tick. Animated values flow back through `Signal<f32>`
//! which trigger dirty-rect marks and partial repaints — no special wiring needed.
//!
//! # Usage
//!
//! ```rust,ignore
//! let mut battery = AnimatedSignal::new(0.0_f32);
//! let bar = ProgressBar::new(battery.signal());
//!
//! let mut app = ViApp::new(Box::new(bar), renderer);
//! app.add_animation(Box::new(battery));
//!
//! // Later, animate to a new value over 400ms:
//! // battery is moved into app, so drive it from a shared Signal<f32> instead
//! ```

extern crate alloc;

use crate::signal::Signal;

// ─── Animatable ──────────────────────────────────────────────────────────────

/// Object-safe interface for time-driven animations.
///
/// `ViApp` stores `Box<dyn Animatable>` and calls `tick(dt_ms)` each frame.
pub trait Animatable {
    /// Advance by `dt_ms` milliseconds. Returns `true` if a signal was updated
    /// (i.e. a repaint may be needed). Returning `false` means the animation is
    /// idle — no repaints are triggered.
    fn tick(&mut self, dt_ms: u32) -> bool;

    /// True while an animation is actively running.
    fn is_active(&self) -> bool;
}

// ─── Easing ──────────────────────────────────────────────────────────────────

/// Standard easing functions. Input `t ∈ [0.0, 1.0]`, output `∈ [0.0, 1.0]`.
pub mod easing {
    #[inline] pub fn linear(t: f32)      -> f32 { t }
    #[inline] pub fn ease_in(t: f32)     -> f32 { t * t }
    #[inline] pub fn ease_out(t: f32)    -> f32 { t * (2.0 - t) }
    #[inline] pub fn ease_in_out(t: f32) -> f32 {
        if t < 0.5 { 2.0 * t * t } else { -1.0 + (4.0 - 2.0 * t) * t }
    }
}

// ─── Tween ───────────────────────────────────────────────────────────────────

/// Single-value interpolator with configurable easing.
///
/// Advance with `tick(dt_ms)` — returns the current interpolated value.
/// Once `is_done()` is true, `tick()` returns `end` and is a no-op.
pub struct Tween {
    start:       f32,
    end:         f32,
    duration_ms: u32,
    elapsed_ms:  u32,
    easing:      fn(f32) -> f32,
}

impl Tween {
    pub fn new(start: f32, end: f32, duration_ms: u32) -> Self {
        Self { start, end, duration_ms: duration_ms.max(1), elapsed_ms: 0, easing: easing::ease_in_out }
    }

    /// Override the default `ease_in_out` easing.
    pub fn with_easing(mut self, f: fn(f32) -> f32) -> Self { self.easing = f; self }

    /// Advance by `dt_ms` and return the current interpolated value.
    pub fn tick(&mut self, dt_ms: u32) -> f32 {
        self.elapsed_ms = (self.elapsed_ms + dt_ms).min(self.duration_ms);
        let t = self.elapsed_ms as f32 / self.duration_ms as f32;
        let t_eased = (self.easing)(t.clamp(0.0, 1.0));
        self.start + (self.end - self.start) * t_eased
    }

    /// True when the animation has reached its end value.
    pub fn is_done(&self) -> bool { self.elapsed_ms >= self.duration_ms }

    /// Current value without advancing time.
    pub fn current(&self) -> f32 {
        let t = (self.elapsed_ms as f32 / self.duration_ms as f32).clamp(0.0, 1.0);
        self.start + (self.end - self.start) * (self.easing)(t)
    }
}

// ─── AnimatedSignal ──────────────────────────────────────────────────────────

/// Combines a `Signal<f32>` with an active `Tween` to produce smooth value transitions.
///
/// Register with `ViApp::add_animation()`. Animate to new values via
/// `animate_to()`. Pass `signal()` to widgets — they observe the signal
/// normally, unaware of the underlying animation.
///
/// # Example
///
/// ```rust,ignore
/// let mut battery_anim = AnimatedSignal::new(0.0_f32);
/// let bar = ProgressBar::new(battery_anim.signal());
/// app.add_animation(Box::new(battery_anim));
/// // Signal ref is in ProgressBar; drive updates through a shared Signal clone.
/// ```
pub struct AnimatedSignal {
    signal: Signal<f32>,
    tween:  Option<Tween>,
}

impl AnimatedSignal {
    pub fn new(initial: f32) -> Self {
        Self { signal: Signal::new(initial), tween: None }
    }

    /// Clone the inner signal handle for passing to widgets.
    pub fn signal(&self) -> Signal<f32> { self.signal.clone() }

    /// Set the value immediately (no animation).
    pub fn set(&self, value: f32) { self.signal.set(value); }

    /// Start a smooth transition to `target` over `duration_ms` milliseconds.
    ///
    /// Cancels any in-progress animation and starts from the current value.
    pub fn animate_to(&mut self, target: f32, duration_ms: u32) {
        let start = *self.signal.get();
        self.tween = Some(Tween::new(start, target, duration_ms));
    }

    /// Start animation with explicit start value (useful when resetting mid-animation).
    pub fn animate_from_to(&mut self, from: f32, to: f32, duration_ms: u32) {
        self.signal.set(from);
        self.tween = Some(Tween::new(from, to, duration_ms));
    }

    /// True when an animation is currently running.
    pub fn animating(&self) -> bool { self.tween.is_some() }
}

impl Animatable for AnimatedSignal {
    fn tick(&mut self, dt_ms: u32) -> bool {
        if let Some(tween) = &mut self.tween {
            let v = tween.tick(dt_ms);
            self.signal.set(v);
            if tween.is_done() { self.tween = None; }
            true
        } else {
            false
        }
    }

    fn is_active(&self) -> bool { self.tween.is_some() }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    #[test]
    fn tween_reaches_end() {
        let mut tw = Tween::new(0.0, 1.0, 200);
        let v = tw.tick(200);
        assert!((v - 1.0).abs() < 1e-6, "expected 1.0, got {v}");
        assert!(tw.is_done());
    }

    #[test]
    fn tween_clamps_overshoot() {
        let mut tw = Tween::new(0.0, 1.0, 100);
        tw.tick(300);   // overshoot by 200ms
        assert!(tw.is_done());
        assert!((tw.current() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn animated_signal_drives_value() {
        let mut anim = AnimatedSignal::new(0.0_f32);
        anim.animate_to(1.0, 100);
        anim.tick(50);
        let mid = *anim.signal().get();
        assert!(mid > 0.0 && mid < 1.0, "mid-animation value out of range: {mid}");
        anim.tick(50);
        let end = *anim.signal().get();
        assert!((end - 1.0).abs() < 1e-5, "expected 1.0, got {end}");
    }

    #[test]
    fn easing_linear_identity() {
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            assert!((easing::linear(t) - t).abs() < 1e-6);
        }
    }
}
