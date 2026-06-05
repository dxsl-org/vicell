//! C-callable Rust bridge for typed VFS IPC.
//!
//! `modvfs.c` declares `extern` prototypes for these symbols and calls them
//! directly — the same pattern as `net_bridge.rs` for network IPC.
//!
//! Internal helpers (`vfs_ok`, `vfs_get_file_into`, `vfs_write_chunked`) mirror
//! the logic in `cells/runtimes/lua/src/bindings_vfs.rs` but have no Lua dependency.

use core::ffi::c_int;

const VFS_ENDPOINT: usize = 3;
/// Maximum payload per chunked write IPC (stays well inside the 512-byte frame).
const MAX_CHUNK: usize = 400;

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Send a typed VfsRequest and return `true` when the reply is `VfsResponse::Ok`.
pub(crate) fn vfs_ok(req: &api::ipc::VfsRequest<'_>) -> bool {
    let mut buf = [0u8; 512];
    let n = match api::ipc::encode(req, &mut buf) {
        Ok(s) => s.len(),
        Err(_) => return false,
    };
    ostd::syscall::sys_send(VFS_ENDPOINT, &buf[..n]);
    let mut reply = [0u8; 64];
    match ostd::syscall::sys_recv(0, &mut reply) {
        ostd::syscall::SyscallResult::Ok(_) => {
            matches!(
                api::ipc::decode::<api::ipc::VfsResponse>(&reply),
                Ok(api::ipc::VfsResponse::Ok)
            )
        }
        _ => false,
    }
}

/// Read file content from VFS into `out` via `GetFile`. Returns bytes copied.
///
/// `GetFile` returns a `DataPtr` (zero-copy pointer into VFS SAS memory). The
/// bytes MUST be copied before the next `sys_recv` call — VFS may reuse the
/// pointed memory once it processes the next request.
pub(crate) fn vfs_get_file_into(path: &str, out: &mut [u8]) -> usize {
    let req = api::ipc::VfsRequest::GetFile(path);
    let mut buf = [0u8; 512];
    let n = match api::ipc::encode(&req, &mut buf) {
        Ok(s) => s.len(),
        Err(_) => return 0,
    };
    ostd::syscall::sys_send(VFS_ENDPOINT, &buf[..n]);
    let mut reply = [0u8; 512];
    match ostd::syscall::sys_recv(0, &mut reply) {
        ostd::syscall::SyscallResult::Ok(_) => {
            match api::ipc::decode::<api::ipc::VfsResponse>(&reply) {
                Ok(api::ipc::VfsResponse::DataPtr { ptr, len }) => {
                    let data_len = (len as usize).min(out.len());
                    // SAFETY: ptr is a valid SAS pointer from VFS; the VFS cell is
                    // blocked in its recv loop while we hold the reply, so the pointed
                    // memory is stable for the duration of this copy.
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            ptr as *const u8,
                            out.as_mut_ptr(),
                            data_len,
                        );
                    }
                    data_len
                }
                _ => 0,
            }
        }
        _ => 0,
    }
}

/// Write `data` to `path`, chunking into MAX_CHUNK-byte IPC payloads.
fn vfs_write_chunked(path: &str, data: &[u8], append: bool) -> bool {
    if data.is_empty() {
        return if append {
            true
        } else {
            vfs_ok(&api::ipc::VfsRequest::Write { path, content: &[] })
        };
    }
    let mut first = !append;
    let mut ok = true;
    for chunk in data.chunks(MAX_CHUNK) {
        let req = if first {
            first = false;
            api::ipc::VfsRequest::Write { path, content: chunk }
        } else {
            api::ipc::VfsRequest::Append { path, content: chunk }
        };
        ok &= vfs_ok(&req);
    }
    ok
}

// ── C-callable exports ────────────────────────────────────────────────────────

/// Read file content into `out[0..out_size]` via typed VFS IPC.
///
/// Returns bytes copied (0 = file not found or empty).
///
/// # Safety
/// `path[0..pl]` and `out[0..out_size]` must be valid, non-overlapping for
/// the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn ViCell_vfs_read(
    path: *const u8,
    pl: usize,
    out: *mut u8,
    out_size: usize,
) -> usize {
    // SAFETY: caller guarantees path[0..pl] is valid UTF-8-ish bytes (from Python str).
    let path_bytes = unsafe { core::slice::from_raw_parts(path, pl) };
    let path_str = core::str::from_utf8(path_bytes).unwrap_or("");
    // SAFETY: caller guarantees out[0..out_size] is a valid writable buffer.
    let buf = unsafe { core::slice::from_raw_parts_mut(out, out_size) };
    vfs_get_file_into(path_str, buf)
}

/// Write `data[0..dl]` to `path`, overwriting existing content.
///
/// Returns 1 on success, 0 on failure.
///
/// # Safety
/// `path[0..pl]` and `data[0..dl]` must be valid for the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn ViCell_vfs_write(
    path: *const u8,
    pl: usize,
    data: *const u8,
    dl: usize,
) -> c_int {
    // SAFETY: caller guarantees path[0..pl] is valid.
    let path_str = core::str::from_utf8(unsafe { core::slice::from_raw_parts(path, pl) })
        .unwrap_or("");
    // SAFETY: caller guarantees data[0..dl] is valid.
    let content = unsafe { core::slice::from_raw_parts(data, dl) };
    vfs_write_chunked(path_str, content, false) as c_int
}

/// Append `data[0..dl]` to `path`.
///
/// Returns 1 on success, 0 on failure.
///
/// # Safety
/// `path[0..pl]` and `data[0..dl]` must be valid for the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn ViCell_vfs_append(
    path: *const u8,
    pl: usize,
    data: *const u8,
    dl: usize,
) -> c_int {
    // SAFETY: caller guarantees path[0..pl] and data[0..dl] are valid.
    let path_str = core::str::from_utf8(unsafe { core::slice::from_raw_parts(path, pl) })
        .unwrap_or("");
    let content = unsafe { core::slice::from_raw_parts(data, dl) };
    vfs_write_chunked(path_str, content, true) as c_int
}

/// Create a directory at `path`.
///
/// Returns 1 on success, 0 on failure (directory already exists or VFS error).
///
/// # Safety
/// `path[0..pl]` must be valid for the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn ViCell_vfs_mkdir(path: *const u8, pl: usize) -> c_int {
    // SAFETY: caller guarantees path[0..pl] is valid.
    let path_str = core::str::from_utf8(unsafe { core::slice::from_raw_parts(path, pl) })
        .unwrap_or("");
    vfs_ok(&api::ipc::VfsRequest::Mkdir(path_str)) as c_int
}

/// Stat a path. On success writes `*size_out` and `*is_dir_out` (0=file, 1=dir).
///
/// Returns 1 on success, 0 if the path does not exist.
///
/// # Safety
/// `path[0..pl]`, `size_out`, and `is_dir_out` must be valid, writable for
/// the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn ViCell_vfs_stat(
    path: *const u8,
    pl: usize,
    size_out: *mut u64,
    is_dir_out: *mut c_int,
) -> c_int {
    // SAFETY: caller guarantees path[0..pl] is valid.
    let path_str = core::str::from_utf8(unsafe { core::slice::from_raw_parts(path, pl) })
        .unwrap_or("");
    let mut buf = [0u8; 512];
    let n = match api::ipc::encode(&api::ipc::VfsRequest::Stat(path_str), &mut buf) {
        Ok(s) => s.len(),
        Err(_) => return 0,
    };
    ostd::syscall::sys_send(VFS_ENDPOINT, &buf[..n]);
    let mut reply = [0u8; 64];
    match ostd::syscall::sys_recv(0, &mut reply) {
        ostd::syscall::SyscallResult::Ok(_) => {
            match api::ipc::decode::<api::ipc::VfsResponse>(&reply) {
                Ok(api::ipc::VfsResponse::Stat { size, is_dir }) => {
                    // SAFETY: caller guarantees size_out and is_dir_out are valid pointers.
                    unsafe {
                        *size_out = size;
                        *is_dir_out = is_dir as c_int;
                    }
                    1
                }
                _ => 0,
            }
        }
        _ => 0,
    }
}

/// List directory entries into `out[0..out_size]` as `"d:name\nf:name\n"` text.
///
/// Returns bytes written (0 = path not a directory or VFS error).
///
/// # Safety
/// `path[0..pl]` and `out[0..out_size]` must be valid for the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn ViCell_vfs_listdir(
    path: *const u8,
    pl: usize,
    out: *mut u8,
    out_size: usize,
) -> usize {
    // SAFETY: caller guarantees path[0..pl] is valid.
    let path_str = core::str::from_utf8(unsafe { core::slice::from_raw_parts(path, pl) })
        .unwrap_or("");
    let mut buf = [0u8; 512];
    let n = match api::ipc::encode(&api::ipc::VfsRequest::ListDir(path_str), &mut buf) {
        Ok(s) => s.len(),
        Err(_) => return 0,
    };
    ostd::syscall::sys_send(VFS_ENDPOINT, &buf[..n]);
    let mut reply = [0u8; 512];
    match ostd::syscall::sys_recv(0, &mut reply) {
        ostd::syscall::SyscallResult::Ok(_) => {
            match api::ipc::decode::<api::ipc::VfsResponse>(&reply) {
                Ok(api::ipc::VfsResponse::Data(entries)) => {
                    let copy_len = entries.len().min(out_size);
                    // SAFETY: out[0..out_size] is a valid writable buffer from caller.
                    unsafe { core::ptr::copy_nonoverlapping(entries.as_ptr(), out, copy_len); }
                    copy_len
                }
                _ => 0,
            }
        }
        _ => 0,
    }
}

/// Delete (unlink) a file at `path`.
///
/// Returns 1 on success, 0 if the file does not exist or VFS returns an error.
///
/// # Safety
/// `path[0..pl]` must be valid for the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn ViCell_vfs_remove(path: *const u8, pl: usize) -> c_int {
    // SAFETY: caller guarantees path[0..pl] is valid.
    let path_str = core::str::from_utf8(unsafe { core::slice::from_raw_parts(path, pl) })
        .unwrap_or("");
    vfs_ok(&api::ipc::VfsRequest::Unlink(path_str)) as c_int
}
