// SPDX-License-Identifier: MPL-2.0
//! Runtime capability delegation handles.
//!
//! A `CapHandle` represents a revocable capability delegation from a supervisor
//! to a child cell.  The supervisor (e.g. `init`) constructs a `CapHandle` after
//! spawning a child and noting which caps were granted.  Calling [`CapHandle::revoke`]
//! issues `sys_cap_revoke`, stripping those caps from the live target in-kernel.
//!
//! The target cell continues running but any subsequent syscall that requires a
//! revoked cap is denied with `PermissionDenied`.  For system cells (`block_io` /
//! `network` holders) revocation is blocked — use `sys_hotswap` instead.

use api::syscall::cap_mask;
use crate::syscall::{sys_cap_revoke, SyscallError};

/// A handle representing a revocable capability delegation to another cell.
///
/// # Monotonic downgrade invariant
/// `CapHandle` does not grant new capabilities; it only tracks what was already
/// granted at spawn time.  A cell can never hand a child a `CapHandle` whose
/// `cap_mask` exceeds the caps the supervisor itself holds.
///
/// # Revocation semantics
/// Revocation is best-effort and immediate: the kernel clears the TCB field
/// atomically under the scheduler lock.  The target cell may have already
/// consumed the cap (e.g. opened an MMIO range via `sys_request_mmio`); those
/// live effects are not undone.  Revocation prevents future use only.
///
/// For full teardown (revoke + kill + respawn) combine with `sys_force_exit`
/// and a subsequent `sys_spawn_from_path`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapHandle {
    target_tid: usize,
    cap_mask:   u32,
}

impl CapHandle {
    /// Create a handle representing the given `cap_mask` granted to `target_tid`.
    ///
    /// No syscall is issued; this only records the association so [`revoke`]
    /// knows what to strip.
    ///
    /// [`revoke`]: Self::revoke
    pub const fn new(target_tid: usize, cap_mask: u32) -> Self {
        Self { target_tid, cap_mask }
    }

    /// Revoke all capabilities this handle represents from the live target cell.
    ///
    /// Equivalent to calling `sys_cap_revoke(target_tid, cap_mask)`.
    ///
    /// # Errors
    /// See [`sys_cap_revoke`] — `PermissionDenied` if caller lacks `SpawnCap`,
    /// target is a system cell, or `InvalidCommand` if the target no longer exists.
    pub fn revoke(&self) -> Result<(), SyscallError> {
        sys_cap_revoke(self.target_tid, self.cap_mask)
    }

    /// Revoke only the specified subset of capabilities (must be a subset of
    /// `self.cap_mask`; extra bits are silently ignored by the kernel).
    pub fn revoke_partial(&self, mask: u32) -> Result<(), SyscallError> {
        sys_cap_revoke(self.target_tid, self.cap_mask & mask)
    }

    /// Task ID of the cell this handle controls.
    pub const fn target_tid(&self) -> usize { self.target_tid }

    /// The full capability mask this handle can revoke.
    pub const fn cap_mask(&self) -> u32 { self.cap_mask }

    /// Return `true` if this handle controls the network capability.
    pub const fn has_network(&self) -> bool { self.cap_mask & cap_mask::NETWORK != 0 }

    /// Return `true` if this handle controls the spawn capability.
    pub const fn has_spawn(&self) -> bool { self.cap_mask & cap_mask::SPAWN != 0 }
}
