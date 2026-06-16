// SPDX-License-Identifier: MPL-2.0

//! Service discovery helpers for ViCell cells.
//!
//! Two levels of abstraction:
//! - Free functions [`lookup`] / [`register`] — thin wrappers for one-shot calls.
//! - [`ServiceRef<const ID>`] — a caching handle with a typed [`ServiceRef::call`] method
//!   for cells that talk to the same service repeatedly.

use crate::{syscall, ViError, ViResult};
use api::ipc::IPC_BUF_SIZE;

pub use api::syscall::service;

/// Resolve the live provider tid of a well-known service.
///
/// Returns `Some(tid)` when a live provider is registered. Returns `None` during
/// the brief death→respawn window — the caller should retry with a small delay.
///
/// # Example
/// ```no_run
/// let vfs = ostd::service::lookup(ostd::service::service::VFS).expect("VFS not ready");
/// ```
pub fn lookup(service_id: u16) -> Option<usize> {
    syscall::sys_lookup_service(service_id)
}

/// Register the calling cell as the live provider of `service_id`.
///
/// Requires `SpawnCap` — intended for the supervisor (`init`), which registers
/// each service after spawning it. Normal service cells do NOT call this directly;
/// `init` registers them on their behalf.
///
/// # Errors
/// Returns `ViError::PermissionDenied` when the caller lacks `SpawnCap`, or
/// `ViError::Unknown` if the registry is full.
pub fn register(service_id: u16, tid: usize) -> ViResult<()> {
    match syscall::sys_register_service(service_id, tid) {
        syscall::SyscallResult::Ok(_) => Ok(()),
        syscall::SyscallResult::Err(_) => Err(ViError::PermissionDenied),
    }
}

// ─── ServiceRef — caching typed IPC handle ───────────────────────────────────

/// A caching handle to a well-known service.
///
/// Resolves the live provider TID on first use and caches it. When the service
/// restarts (new TID), call [`invalidate`][Self::invalidate] so the next
/// [`call`][Self::call] re-resolves.
///
/// # Usage
/// ```no_run
/// use ostd::service::{ServiceRef, service};
/// use api::ipc::{IPC_BUF_SIZE, VfsRequest, VfsResponse};
///
/// let mut vfs: ServiceRef<{service::VFS}> = ServiceRef::new();
/// let mut resp_buf = [0u8; IPC_BUF_SIZE];
/// let resp: VfsResponse = vfs.call(&VfsRequest::Stat("/tmp"), &mut resp_buf)?;
/// ```
pub struct ServiceRef<const ID: u16> {
    cached_tid: Option<usize>,
}

impl<const ID: u16> ServiceRef<ID> {
    /// Create a new, unresolved handle. Resolution happens lazily on the first [`call`][Self::call].
    pub const fn new() -> Self {
        Self { cached_tid: None }
    }

    /// Resolve the live provider TID, retrying up to 8 times with a scheduler yield between
    /// attempts (mirrors the [`ConfigClient::endpoint`] pattern).
    ///
    /// Returns `None` during the brief death→respawn window.
    pub fn resolve(&mut self) -> Option<usize> {
        if let Some(tid) = self.cached_tid {
            return Some(tid);
        }
        for _ in 0..8 {
            if let Some(tid) = syscall::sys_lookup_service(ID) {
                self.cached_tid = Some(tid);
                return Some(tid);
            }
            crate::task::yield_now();
        }
        None
    }

    /// Clear the cached TID. Call this after a send error to force re-resolution on the
    /// next [`call`][Self::call] (the service may have restarted with a new TID).
    pub fn invalidate(&mut self) {
        self.cached_tid = None;
    }

    /// Send a postcard-encoded `req` to the service and decode the response into `resp_buf`.
    ///
    /// The decoded `Resp` may borrow bytes from `resp_buf` (e.g. `VfsResponse::Data(&[u8])`).
    /// Keep `resp_buf` alive as long as you use the returned value.
    ///
    /// On send failure the TID cache is invalidated automatically.
    ///
    /// # Errors
    /// - `ViError::NotFound` — service not registered after 8 retries.
    /// - `ViError::InvalidArgument` — `req` could not be encoded (message too large).
    /// - `ViError::IO` — send or receive syscall failed, or response decoding failed.
    pub fn call<'b, Req, Resp>(
        &mut self,
        req: &Req,
        resp_buf: &'b mut [u8; IPC_BUF_SIZE],
    ) -> ViResult<Resp>
    where
        Req: serde::Serialize,
        Resp: serde::Deserialize<'b>,
    {
        let tid = self.resolve().ok_or(ViError::NotFound)?;
        let mut req_buf = [0u8; IPC_BUF_SIZE];
        let encoded = api::ipc::encode(req, &mut req_buf)
            .map_err(|_| ViError::InvalidArgument)?;
        if let syscall::SyscallResult::Err(_) = syscall::sys_send(tid, encoded) {
            self.invalidate();
            return Err(ViError::IO);
        }
        match syscall::sys_recv(0, resp_buf) {
            syscall::SyscallResult::Ok(sender) if sender > 0 => {
                api::ipc::decode::<Resp>(resp_buf).map_err(|_| ViError::IO)
            }
            _ => Err(ViError::IO),
        }
    }
}

impl<const ID: u16> Default for ServiceRef<ID> {
    fn default() -> Self { Self::new() }
}

/// Convenience type alias — VFS service handle.
pub type VfsRef     = ServiceRef<{ service::VFS }>;
/// Convenience type alias — net service handle.
pub type NetRef     = ServiceRef<{ service::NET }>;
/// Convenience type alias — input service handle.
pub type InputRef   = ServiceRef<{ service::INPUT }>;
/// Convenience type alias — config service handle.
pub type ConfigRef  = ServiceRef<{ service::CONFIG }>;
/// Convenience type alias — compositor service handle.
pub type CompositorRef = ServiceRef<{ service::COMPOSITOR }>;
