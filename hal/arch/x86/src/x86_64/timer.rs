//! LAPIC periodic timer wrapper (delegates to apic::init_lapic).
pub fn init() { super::apic::init_lapic(); }
/// No-op: LAPIC periodic timer reloads automatically.
pub fn reset() {}
