// SPDX-License-Identifier: MIT
//! Reactive Signal<T> — fine-grained dependency tracking for ViUI v2.
//!
//! # Subscription lifetime
//!
//! Subscriptions stay alive as long as the returned `SubscriptionHandle` is alive.
//! Dropping the handle deactivates the callback on the next `notify()` pass.
//! Widgets must store handles in their struct fields to maintain subscriptions.
//!
//! # Re-entrancy
//!
//! Calling `Signal::set` from within a subscriber is safe but silently skipped
//! (the `notifying` guard prevents an infinite loop). To chain updates, use a
//! separate event queue or schedule the set after the current frame.

extern crate alloc;
use alloc::{rc::Rc, vec::Vec};
use core::cell::{Cell, Ref, RefCell};

// ─── SubscriptionHandle ───────────────────────────────────────────────────────

/// Keeps a subscription alive. Dropping this value unsubscribes the callback.
///
/// Store in a struct field for the subscription to remain active.
pub struct SubscriptionHandle {
    // Holds external strong ref to the Rc<dyn Fn()> inside SignalInner::subs.
    // Drop → strong_count on inner Rc falls to 1 → pruned on next notify().
    _rc: Rc<dyn Fn()>,
}

// ─── SignalInner<T> ───────────────────────────────────────────────────────────

struct SignalInner<T: 'static> {
    value:     RefCell<T>,
    subs:      RefCell<Vec<Rc<dyn Fn()>>>,
    notifying: Cell<bool>,
}

// ─── Signal<T> ───────────────────────────────────────────────────────────────

/// Reactive value container. Cloning shares the same underlying cell.
///
/// When `set()` or `update()` is called, all live subscribers are notified
/// synchronously before the method returns.
pub struct Signal<T: 'static> {
    inner: Rc<SignalInner<T>>,
}

impl<T: 'static> Clone for Signal<T> {
    fn clone(&self) -> Self { Self { inner: Rc::clone(&self.inner) } }
}

impl<T: 'static> Signal<T> {
    pub fn new(value: T) -> Self {
        Self {
            inner: Rc::new(SignalInner {
                value:     RefCell::new(value),
                subs:      RefCell::new(Vec::new()),
                notifying: Cell::new(false),
            }),
        }
    }

    /// Borrow the current value. Do not hold across a `set()` call.
    pub fn get(&self) -> Ref<'_, T> { self.inner.value.borrow() }

    /// Replace the value and notify all live subscribers.
    pub fn set(&self, value: T) {
        *self.inner.value.borrow_mut() = value;
        self.notify();
    }

    /// Mutate the value in-place and notify all live subscribers.
    pub fn update<F: FnOnce(&mut T)>(&self, f: F) {
        f(&mut self.inner.value.borrow_mut());
        self.notify();
    }

    /// Register a callback; returns a handle that keeps the subscription alive.
    pub fn subscribe<F: Fn() + 'static>(&self, f: F) -> SubscriptionHandle {
        let rc: Rc<dyn Fn()> = Rc::new(f);
        let handle_rc = Rc::clone(&rc);
        self.inner.subs.borrow_mut().push(rc);
        SubscriptionHandle { _rc: handle_rc }
    }

    /// Derive a `Computed<U>` that updates whenever this signal changes.
    pub fn map<U: 'static, F: Fn(&T) -> U + 'static>(&self, f: F) -> Computed<U> {
        let initial = f(&self.inner.value.borrow());
        let out = Signal::new(initial);
        let out_clone = out.clone();
        let self_clone = self.clone();
        let handle = self.subscribe(move || {
            let new_val = f(&self_clone.inner.value.borrow());
            out_clone.set(new_val);
        });
        Computed { signal: out, _handle: handle }
    }

    fn notify(&self) {
        if self.inner.notifying.get() { return; }
        self.inner.notifying.set(true);
        // Clone the list so subscribers may call set() on *other* signals without
        // hitting the subs RefCell borrow conflict on this signal.
        let subs: Vec<Rc<dyn Fn()>> = self.inner.subs.borrow().clone();
        for sub in &subs {
            // strong_count > 1 → external SubscriptionHandle is still alive
            if Rc::strong_count(sub) > 1 { sub(); }
        }
        // Prune dead entries (only one strong ref left = this subs vec)
        self.inner.subs.borrow_mut().retain(|rc| Rc::strong_count(rc) > 1);
        self.inner.notifying.set(false);
    }
}

// ─── Computed<T> ─────────────────────────────────────────────────────────────

/// Read-only derived signal that updates when its source signal changes.
///
/// Alive as long as this struct exists; drop to stop tracking.
pub struct Computed<T: 'static> {
    signal:  Signal<T>,
    // Keeps the source → out subscription alive for the lifetime of this Computed.
    _handle: SubscriptionHandle,
}

impl<T: 'static> Computed<T> {
    pub fn get(&self) -> Ref<'_, T> { self.signal.get() }

    /// Subscribe to changes in the computed output.
    pub fn subscribe<F: Fn() + 'static>(&self, f: F) -> SubscriptionHandle {
        self.signal.subscribe(f)
    }

    /// Decompose into the inner `Signal<T>` and the source subscription handle.
    ///
    /// The returned `SubscriptionHandle` must be stored to keep automatic
    /// updates active — dropping it stops the computed from tracking its source.
    pub fn into_parts(self) -> (Signal<T>, SubscriptionHandle) {
        (self.signal, self._handle)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    extern crate alloc;
    use alloc::rc::Rc;
    use core::cell::Cell;
    use super::*;

    #[test]
    fn set_notifies_subscriber() {
        let sig = Signal::new(0i32);
        let fired = Rc::new(Cell::new(0u32));
        let fired2 = Rc::clone(&fired);
        let _h = sig.subscribe(move || fired2.set(fired2.get() + 1));
        sig.set(1);
        assert_eq!(fired.get(), 1);
        sig.set(2);
        assert_eq!(fired.get(), 2);
    }

    #[test]
    fn dropped_handle_stops_notification() {
        let sig = Signal::new(0i32);
        let fired = Rc::new(Cell::new(false));
        let fired2 = Rc::clone(&fired);
        let h = sig.subscribe(move || fired2.set(true));
        drop(h);
        sig.set(1);
        // handle was dropped → subscriber pruned → not called
        assert!(!fired.get());
    }

    #[test]
    fn map_updates_computed() {
        let sig = Signal::new(3i32);
        let doubled = sig.map(|n| n * 2);
        assert_eq!(*doubled.get(), 6);
        sig.set(5);
        assert_eq!(*doubled.get(), 10);
    }

    #[test]
    fn reentrancy_does_not_loop() {
        let sig = Signal::<i32>::new(0);
        let sig2 = sig.clone();
        // Subscriber calls set() on the same signal → must not recurse
        let _h = sig.subscribe(move || { sig2.set(99); });
        sig.set(1); // should return without hanging
        assert_eq!(*sig.get(), 99); // second set took effect after first notify
    }

    #[test]
    fn dirty_rect_union() {
        use crate::dirty::DirtyRect;
        use crate::layout::Rect;
        let mut d = DirtyRect::new();
        assert!(!d.is_dirty());
        d.mark(Rect::new(0.0, 0.0, 10.0, 10.0));
        d.mark(Rect::new(20.0, 0.0, 10.0, 10.0));
        let r = d.take().unwrap();
        assert_eq!(r.x, 0.0);
        assert_eq!(r.w, 30.0);
        assert!(!d.is_dirty());
    }
}
