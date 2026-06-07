//! Fast-IPC type tokens for direct function-pointer dispatch between trusted Cells.
//!
//! In a Single Address Space OS, a trusted Cell can call a kernel service
//! function directly via a pointer — bypassing the ecall trap (~100 cycles)
//! for a single indirect branch (~3 cycles).
//!
//! `TrustedHandle<T>` is a zero-sized type (ZST) whose constructor is
//! `pub(crate)` — only the kernel crate can create one.  Cell crates that
//! hold a `TrustedHandle<VfsCell>` prove they were granted fast-path access
//! by the kernel at spawn time.

use core::marker::PhantomData;

/// Authorization token for fast-path VFS access.
///
/// Zero bytes at runtime (ZST).  `pub(crate)` constructor ensures only kernel
/// code can create one; Cell crates can receive and pass the token but cannot
/// forge it.
#[derive(Copy, Clone, Debug)]
pub struct TrustedHandle<T>(PhantomData<T>);

impl<T> TrustedHandle<T> {
    /// Create a `TrustedHandle`.  `pub(crate)` — only `libs/api` itself can
    /// construct one directly; external callers use `TrustedHandle::default()`.
    ///
    /// In SAS the unforgeability is advisory (all cells share the address space),
    /// but the restriction documents intent: only kernel-authorized paths should
    /// hold the token.  `Default` is intentionally left `pub` so callers outside
    /// this crate can use `TrustedHandle::default()`.
    pub(crate) fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T> Default for TrustedHandle<T> {
    fn default() -> Self { Self::new() }
}

// ── Marker types for each fast-IPC service ────────────────────────────────────

/// Marker: authorization token for direct VFS service calls.
pub struct VfsCell;

/// Marker: authorization token for direct network service calls.
pub struct NetCell;
