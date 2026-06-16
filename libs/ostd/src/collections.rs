// SPDX-License-Identifier: MPL-2.0

//! `no_std`-compatible collections for ViCell cells.
//!
//! Re-exports [`hashbrown`] types under a stable `ostd::collections` namespace so
//! cells can use hash maps without depending on `hashbrown` directly.

pub use hashbrown::HashMap;
pub use hashbrown::HashSet;
