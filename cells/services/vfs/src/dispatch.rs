//! Typed IPC request dispatch — decodes one `VfsRequest` and produces the
//! `VfsResponse`, routing filesystem ops through the MountTable.
//!
//! Cross-cutting policy lives here, not in backends: AccessTable authorization,
//! quota accounting (net-delta on overwrite), async-read pending table, and
//! zero-copy grant I/O.

use crate::manager::VfsManager;

/// Handle one decoded request. `resp_buf` backs `VfsResponse::Data` payloads,
/// so the response borrows it; callers encode before reusing the buffer.
pub fn handle_request<'a>(
    vfs: &mut VfsManager,
    buf: &[u8; 512],
    sender: usize,
    resp_buf: &'a mut [u8; 512],
) -> api::ipc::VfsResponse<'a> {
    // Decode typed request; `take_from_bytes` tolerates trailing zeros in the
    // 512-byte receive buffer left over from previous messages.
    let req = match api::ipc::decode::<api::ipc::VfsRequest>(buf) {
        Ok(r) => r,
        Err(_) => return api::ipc::VfsResponse::Err(0xFF), // malformed request
    };

    match req {
        api::ipc::VfsRequest::GetFile(p) => {
            if let Some((ptr, len)) = vfs.get_file_ptr(p) {
                api::ipc::VfsResponse::DataPtr { ptr: ptr as u64, len: len as u64 }
            } else {
                api::ipc::VfsResponse::Err(1)
            }
        }

        api::ipc::VfsRequest::ListDir(p) => {
            let n = vfs.list_dir(p, resp_buf);
            api::ipc::VfsResponse::Data(&resp_buf[..n])
        }

        api::ipc::VfsRequest::Stat(p) => {
            match vfs.stat(p) {
                Some((size, is_dir)) => api::ipc::VfsResponse::Stat { size, is_dir },
                None => api::ipc::VfsResponse::Err(1),
            }
        }

        api::ipc::VfsRequest::Write { path, content } => {
            let owner = types::CellId(sender as u64);
            // Access check: only authorized cells may write to this path.
            if !vfs.access.can_write(owner, path) {
                return api::ipc::VfsResponse::Err(3); // 3 = PermissionDenied
            }
            // Capture size of any existing file to release its quota share.
            // Overwriting an existing file should charge the delta, not the
            // full new size — otherwise repeated overwrites inflate usage.
            let old_size = vfs.file_size(path);
            let new_size = content.len() as u64;
            // Net quota delta: may be negative (file shrunk) or positive.
            let net_charge = new_size.saturating_sub(old_size);
            if net_charge > 0 && !vfs.quota.can_charge(owner, net_charge) {
                return api::ipc::VfsResponse::Err(2); // 2 = quota exceeded
            }
            if vfs.write(path, content) {
                // Release old bytes and charge new size.
                vfs.quota.release(owner, old_size);
                let _ = vfs.quota.charge(owner, new_size);
                api::ipc::VfsResponse::Ok
            } else {
                api::ipc::VfsResponse::Err(1)
            }
        }

        api::ipc::VfsRequest::Append { path, content } => {
            let owner = types::CellId(sender as u64);
            if !vfs.access.can_write(owner, path) {
                return api::ipc::VfsResponse::Err(3);
            }
            let append_len = content.len() as u64;
            if !vfs.quota.can_charge(owner, append_len) {
                return api::ipc::VfsResponse::Err(2); // quota exceeded
            }
            if vfs.append(path, content) {
                let _ = vfs.quota.charge(owner, append_len);
                api::ipc::VfsResponse::Ok
            } else {
                api::ipc::VfsResponse::Err(1)
            }
        }

        api::ipc::VfsRequest::Mkdir(p) => {
            let owner = types::CellId(sender as u64);
            if !vfs.access.can_write(owner, p) {
                api::ipc::VfsResponse::Err(3)
            } else if vfs.mkdir(p) {
                api::ipc::VfsResponse::Ok
            } else {
                api::ipc::VfsResponse::Err(1)
            }
        }

        api::ipc::VfsRequest::Rmdir(p) => {
            // Verifies the target IS a directory — POSIX ENOTDIR semantics.
            if vfs.rmdir(p) { api::ipc::VfsResponse::Ok } else { api::ipc::VfsResponse::Err(1) }
        }

        api::ipc::VfsRequest::Unlink(p) => {
            // Capture file size before deletion for quota release.
            let file_size = vfs.file_size(p);
            if vfs.unlink(p) {
                // Release the quota that was charged when the file was written.
                let owner = types::CellId(sender as u64);
                vfs.quota.release(owner, file_size);
                api::ipc::VfsResponse::Ok
            } else {
                api::ipc::VfsResponse::Err(1)
            }
        }

        api::ipc::VfsRequest::RmdirRecursive(p) => {
            if vfs.rmdir_recursive(p) { api::ipc::VfsResponse::Ok } else { api::ipc::VfsResponse::Err(1) }
        }

        api::ipc::VfsRequest::ReadAsync { path } => {
            // Read file data synchronously (disk is still blocking in this backend).
            // Store under a handle and return immediately — caller polls.
            let data = vfs.read_to_vec(path);
            let handle = vfs.pending.insert(data);
            api::ipc::VfsResponse::PendingHandle(handle)
        }

        api::ipc::VfsRequest::Poll { handle } => {
            // With a synchronous backend data is always ready on first poll.
            match vfs.pending.poll(handle) {
                Some(data) => {
                    // Cap at 480, not resp_buf.len(): the reply must still fit
                    // the 512-byte IPC frame AFTER the postcard envelope. A
                    // full 512-byte payload made encode fail and the client
                    // saw an empty reply (surfaced by /bin ELF reads).
                    let n = data.len().min(480);
                    resp_buf[..n].copy_from_slice(&data[..n]);
                    api::ipc::VfsResponse::Data(&resp_buf[..n])
                }
                None => api::ipc::VfsResponse::Err(4), // 4 = stale/unknown handle
            }
        }

        // ── Zero-Copy Grant I/O (Storage 2.0, Phase 02) ────────────────

        api::ipc::VfsRequest::ReadGrant { cap, offset, size, grant } => {
            // Validate: VFS must have been GrantShare'd access by the app.
            match ostd::syscall::sys_grant_slice(grant) {
                None => api::ipc::VfsResponse::Err(1), // no access
                Some(ptr) => {
                    // Look up the cap in the VFS handle table.
                    let bytes = if let Some(entry) = vfs.handles.get_mut(api::cap::CapId(cap)) {
                        let avail = entry.data_len.saturating_sub(offset as usize);
                        let n = size.min(avail).min(4096);
                        // SAFETY: data_ptr is a valid in-memory VAddr; ptr is a
                        // kernel-allocated, identity-mapped grant buffer.
                        unsafe {
                            core::ptr::copy_nonoverlapping(
                                (entry.data_ptr + offset as usize) as *const u8,
                                ptr,
                                n,
                            );
                        }
                        n
                    } else {
                        0 // unknown cap — caller must register handle first
                    };
                    // F14: reply AFTER filling the buffer.
                    api::ipc::VfsResponse::GrantDone { bytes }
                }
            }
        }

        api::ipc::VfsRequest::WriteGrant { cap, offset, grant, bytes } => {
            let _ = (cap, offset); // path routing via cap table deferred to Phase 04
            match ostd::syscall::sys_grant_slice(grant) {
                None => api::ipc::VfsResponse::Err(1),
                Some(ptr) => {
                    let n = bytes.min(4096);
                    // SAFETY: ptr is a valid identity-mapped grant buffer filled
                    // by the app before GrantShare + WriteGrant IPC.
                    let data = unsafe { core::slice::from_raw_parts(ptr as *const u8, n) };
                    // Phase 02 stub: data available in `data` slice.
                    // Full routing via cap→path lookup deferred to Phase 04.
                    let _ = data;
                    // F14: GrantDone sent only AFTER reading the grant buffer
                    // (ipc_call blocks, so app cannot free it prematurely).
                    api::ipc::VfsResponse::GrantDone { bytes: n }
                }
            }
        }
    }
}
