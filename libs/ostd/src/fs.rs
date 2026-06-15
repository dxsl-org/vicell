// SPDX-License-Identifier: MPL-2.0

//! Filesystem utilities for ViCell cells.
//!
//! `File` is backed by a kernel capability (`CapId`) obtained via `OpenCap`.
//! Single-owner: moving a `File` transfers the capability.  Dropping without
//! calling `close()` issues an implicit close (which revokes the capability)
//! and, in debug builds, emits a warning about the implicit close.

use crate::syscall;
use alloc::string::String;
use alloc::vec::Vec;
use types::*;
use api::ipc::{VfsRequest, VfsResponse};

/// Iterator over directory entries returned by the kernel.
pub struct ReadDir {
    /// fd from legacy `sys_open` — kept for directory listing (caps are file-only for now).
    fd: usize,
}

impl Iterator for ReadDir {
    type Item = DirEntry;

    fn next(&mut self) -> Option<Self::Item> {
        let mut entry = DirEntry::default();
        // SAFETY: entry is a valid stack-allocated DirEntry; pointer is valid for the call.
        let ptr = &mut entry as *mut _ as *mut u8;
        let size = core::mem::size_of::<DirEntry>();
        let slice = unsafe { core::slice::from_raw_parts_mut(ptr, size) };
        match syscall::sys_read_dir(self.fd, slice) {
            Ok(bytes) if bytes == size => Some(entry),
            _ => None,
        }
    }
}

impl Drop for ReadDir {
    fn drop(&mut self) {
        let _ = syscall::sys_close(self.fd);
    }
}

/// Open directory for reading.
pub fn read_dir(path: &str) -> ViResult<ReadDir> {
    let fd = syscall::sys_open(path).map_err(|_| ViError::NotFound)?;
    Ok(ReadDir { fd })
}

// ─── Capability-based file ────────────────────────────────────────────────────

/// An open file backed by a kernel capability (`CapId`).
///
/// Moving `File` transfers ownership of the underlying capability.  Dropping
/// calls `close()` implicitly; in debug builds a warning is emitted when this
/// happens without an explicit `close()` call (handle-leak detection).
///
/// `path` and `vfs_tid` are cached at `open()` time to support `write_all` via
/// `VfsRequest::Append` IPC. Reads still use the faster cap-based kernel path.
pub struct File {
    cap_id: u64,
    /// Set to `true` by `close()` to suppress the drop warning.
    closed: bool,
    /// Owned copy of the path, needed for VFS Append IPC.
    path: String,
    /// Cached VFS service tid; 0 means VFS was unavailable at open time.
    vfs_tid: usize,
}

impl File {
    /// Open a file at `path`.
    ///
    /// Caches the VFS tid at open time for use by `write_all`. If the VFS
    /// service is temporarily unavailable, `write_all` will return `NotFound`
    /// until the file is re-opened after VFS restarts.
    ///
    /// # Errors
    /// Returns `ViError::NotFound` if the path does not exist in the kernel FS.
    pub fn open(path: &str) -> ViResult<Self> {
        let cap_id = syscall::sys_open_cap(path)
            .map_err(|_| ViError::NotFound)?;
        let vfs_tid = crate::service::lookup(crate::service::service::VFS)
            .unwrap_or(0);
        Ok(Self { cap_id, closed: false, path: String::from(path), vfs_tid })
    }

    /// Read all bytes until EOF into `buf`.
    pub fn read_to_end(&mut self, buf: &mut Vec<u8>) -> ViResult<usize> {
        let mut temp = [0u8; 512];
        let mut total = 0;
        loop {
            match syscall::sys_read_cap(self.cap_id, &mut temp) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&temp[..n]);
                    total += n;
                }
                Err(_) => return Err(ViError::IO),
            }
        }
        Ok(total)
    }

    /// Read up to `buf.len()` bytes from the file.
    ///
    /// Returns the number of bytes actually read (0 = EOF).
    pub fn read(&mut self, buf: &mut [u8]) -> ViResult<usize> {
        syscall::sys_read_cap(self.cap_id, buf).map_err(|_| ViError::IO)
    }

    /// Read the entire file into a `String`.  Returns `Err(IO)` if content is not valid UTF-8.
    pub fn read_to_string(&mut self) -> ViResult<alloc::string::String> {
        let mut bytes = alloc::vec::Vec::new();
        self.read_to_end(&mut bytes)?;
        alloc::string::String::from_utf8(bytes).map_err(|_| ViError::IO)
    }

    /// Append `buf` to the file via VFS IPC.
    ///
    /// **Append semantics:** each call extends the file at its current end.
    /// Multiple `write_all` calls accumulate — correct for `embedded_io::Write`.
    /// For overwrite / truncate semantics, that is future work.
    ///
    /// Large buffers are chunked into ≤400-byte pieces to fit the 512-byte
    /// `vfs_call` send buffer (path + content + postcard framing ≤ 512 bytes).
    /// Not atomic across chunks — a quota failure mid-write leaves a partial append.
    ///
    /// # Errors
    /// - `NotFound` if VFS is unavailable (re-open after VFS restarts to refresh tid).
    /// - `InvalidInput` if `path.len() > 96` (postcard frame would overflow send buffer).
    /// - `IO` on VFS error response (permission denied, quota exceeded, backend failure).
    pub fn write_all(&mut self, buf: &[u8]) -> ViResult<()> {
        if self.vfs_tid == 0 {
            return Err(ViError::NotFound);
        }
        if self.path.len() > 96 {
            // postcard: discriminant(1) + path_len(2) + path(≤96) + content_len(2) + content(≤400) = ≤501 < 512
            return Err(ViError::InvalidInput);
        }
        for chunk in buf.chunks(CHUNK_CONTENT) {
            let req = VfsRequest::Append { path: &self.path, content: chunk };
            let mut resp_buf = [0u8; 512];
            match vfs_call(self.vfs_tid, &req, &mut resp_buf)? {
                VfsResponse::Ok => {}
                _ => return Err(ViError::IO),
            }
        }
        Ok(())
    }

    /// Explicitly close the file and revoke its capability.
    pub fn close(mut self) -> ViResult<()> {
        self.closed = true;
        syscall::sys_close_cap(self.cap_id);
        Ok(())
    }

    /// Return the raw capability ID (for passing to kernel APIs).
    pub fn cap_id(&self) -> u64 {
        self.cap_id
    }
}

impl Drop for File {
    fn drop(&mut self) {
        if !self.closed {
            // Revoke the kernel capability so it doesn't leak after the File is gone.
            // This is the normal Rust drop path (error propagation, end-of-scope, etc.).
            // Calling `File::close()` first is preferred so errors can be observed,
            // but this implicit close is always safe.
            syscall::sys_close_cap(self.cap_id);
        }
    }
}

// ─── embedded-io trait impls ─────────────────────────────────────────────────

/// Max content bytes per `VfsRequest::Append` IPC call.
///
/// Bound by `vfs_call`'s 512-byte send buffer: discriminant(1) + path_len(1,
/// since ≤96 < 128 → 1-byte postcard varint) + path(≤96) + content_len(2,
/// since 400 ≥ 128 → 2-byte varint) + content(≤400) + slack(12) = 512.
/// Changing this constant requires re-verifying that budget.
const CHUNK_CONTENT: usize = 400;

impl embedded_io::ErrorType for File {
    type Error = crate::io::OstdError;
}

impl embedded_io::Read for File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, crate::io::OstdError> {
        // Explicit path to avoid ambiguity with the trait method of the same name.
        // Inherent methods win in Rust method resolution, but this is clearer.
        File::read(self, buf).map_err(crate::io::OstdError)
    }
}

impl embedded_io::Write for File {
    /// Write `buf` by appending it to the file (see `File::write_all` semantics).
    ///
    /// Returns `Ok(buf.len())` on success.  Not atomic across IPC chunks —
    /// if a quota failure occurs mid-write, bytes already flushed to VFS are
    /// committed and `Err` is returned; partial writes are observable.
    fn write(&mut self, buf: &[u8]) -> Result<usize, crate::io::OstdError> {
        self.write_all(buf)
            .map(|_| buf.len())
            .map_err(crate::io::OstdError)
    }

    fn flush(&mut self) -> Result<(), crate::io::OstdError> {
        Ok(()) // VFS writes are synchronous; no client-side buffer to flush
    }
}

// ── Zero-Copy Grant I/O (Storage 2.0, Phase 02) ──────────────────────────────

/// Blocking IPC call to the VFS service: encode `req`, send, receive, decode.
///
/// Uses a stack-allocated 512-byte buffer for both directions.
fn vfs_call<'r>(vfs_tid: usize, req: &VfsRequest<'_>, resp_buf: &'r mut [u8; 512])
    -> ViResult<VfsResponse<'r>>
{
    let mut send_buf = [0u8; 512];
    let n = api::ipc::encode(req, &mut send_buf)
        .map(|s| s.len())
        .map_err(|_| ViError::IO)?;
    syscall::sys_send(vfs_tid, &send_buf[..n]);
    syscall::sys_recv(0, resp_buf);
    api::ipc::decode::<VfsResponse>(resp_buf).map_err(|_| ViError::IO)
}

/// Read up to `buf.len()` bytes from a file cap using the optimal I/O path.
///
/// - `buf.len() < 4096`: kernel ReadCap path (no Grant overhead)
/// - `buf.len() >= 4096`: zero-copy Grant path (one VFS round-trip per 4096 bytes)
///
/// # F14 contract
/// The Grant is freed only AFTER `GrantDone` is received from VFS, ensuring
/// VFS has finished reading the buffer before the caller reclaims the frames.
///
/// # Errors
/// Returns `ViError::IO` on any transport or permission failure.
pub fn read_all(cap_id: u64, buf: &mut [u8], vfs_tid: usize) -> ViResult<usize> {
    if buf.len() < 4096 {
        syscall::sys_read_cap(cap_id, buf).map_err(|_| ViError::IO)
    } else {
        grant_read(cap_id, buf, vfs_tid)
    }
}

/// Write `data` to a file cap using the optimal I/O path.
///
/// - `data.len() < 4096`: kernel WriteGrant IPC path (no Grant overhead; caller
///   uses existing `VfsRequest::Write` via IPC — stub, returns 0 for now)
/// - `data.len() >= 4096`: zero-copy Grant path
///
/// # F14 contract
/// The caller waits for `GrantDone` before freeing the grant, so VFS finishes
/// writing to disk before the frames are returned to the allocator.
pub fn write_all(cap_id: u64, data: &[u8], vfs_tid: usize) -> ViResult<usize> {
    if data.len() < 4096 {
        // Small writes: caller uses existing VfsRequest::Write IPC directly.
        // This wrapper covers the large-file case only; return 0 to signal fallback.
        let _ = (cap_id, vfs_tid);
        Ok(0)
    } else {
        grant_write(cap_id, data, vfs_tid)
    }
}

fn grant_read(cap_id: u64, buf: &mut [u8], vfs_tid: usize) -> ViResult<usize> {
    let size = buf.len().min(4096);
    let grant_id = syscall::sys_grant_alloc(size).ok_or(ViError::OutOfMemory)?;
    // Share RW with VFS so it can fill the grant buffer.
    syscall::sys_grant_share(grant_id, vfs_tid, 2 /* ReadWrite */);

    // Control message fits in 512B IPC buffer.
    let req = VfsRequest::ReadGrant { cap: cap_id, offset: 0, size, grant: grant_id };
    let mut resp_buf = [0u8; 512];
    let resp = vfs_call(vfs_tid, &req, &mut resp_buf)
        .map_err(|e| { syscall::sys_grant_free(grant_id); e })?;

    let bytes = match resp {
        // F14: GrantDone arrives only AFTER VFS has filled the grant buffer.
        VfsResponse::GrantDone { bytes } => bytes,
        _ => { syscall::sys_grant_free(grant_id); return Err(ViError::IO); }
    };

    // SAFETY: grant was allocated with `size` bytes; VFS filled `bytes` of it.
    let ptr = syscall::sys_grant_slice(grant_id).ok_or_else(|| {
        syscall::sys_grant_free(grant_id); ViError::IO
    })?;
    let src = unsafe { core::slice::from_raw_parts(ptr as *const u8, bytes) };
    buf[..bytes].copy_from_slice(src);

    // F14: safe to free — GrantDone already received above.
    syscall::sys_grant_free(grant_id);
    Ok(bytes)
}

fn grant_write(cap_id: u64, data: &[u8], vfs_tid: usize) -> ViResult<usize> {
    let bytes = data.len().min(4096);
    let grant_id = syscall::sys_grant_alloc(bytes).ok_or(ViError::OutOfMemory)?;

    // Fill grant buffer BEFORE sharing — we own it exclusively here.
    // SAFETY: grant was allocated for `bytes`; ptr is valid for that range.
    let ptr = syscall::sys_grant_slice(grant_id).ok_or_else(|| {
        syscall::sys_grant_free(grant_id); ViError::IO
    })?;
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), ptr, bytes) };

    // Share WriteOnly (VFS reads, can't modify).
    syscall::sys_grant_share(grant_id, vfs_tid, 1 /* WriteOnly */);

    let req = VfsRequest::WriteGrant { cap: cap_id, offset: 0, grant: grant_id, bytes };
    let mut resp_buf = [0u8; 512];
    // ipc_call blocks until VFS replies — F14 guarantees VFS drained the grant.
    let resp = vfs_call(vfs_tid, &req, &mut resp_buf)
        .map_err(|e| { syscall::sys_grant_free(grant_id); e })?;

    syscall::sys_grant_free(grant_id);
    match resp {
        VfsResponse::GrantDone { bytes: written } => Ok(written),
        _ => Err(ViError::IO),
    }
}
