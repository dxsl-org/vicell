#![no_std]
#![no_main]

extern crate alloc;
extern crate driver_disk;
// redox_syscall's [lib] name is "syscall"; alias so our code can use redox_syscall:: paths.
extern crate syscall as redox_syscall;

mod access;
mod backend;
mod backend_bootfs;
mod backend_fat;
mod backend_littlefs;
mod backend_ramfs;
mod backend_redoxfs;
mod block_stream;
mod disk_redoxfs;
mod lfs_disk;
mod dispatch;
mod handle_table;
mod manager;
mod mount;
mod page_cache;
mod pending;
mod quota;

use manager::VfsManager;
use ostd::io::println;
use ostd::prelude::*;

// Declares block-I/O capability; the kernel grants BlockIoCap at spawn.
// part_data/part_lfs scope the raw block syscalls to P1 (FAT32) + P4
// (littlefs) — P2 cell-table and P3 snapshot stay kernel-only (P03 design).
api::declare_manifest!(block_io = true, network = false, spawn = false,
                       part_data = true, part_lfs = true);

// Narrow syscall allowlist — kernel enforces this at dispatch (Phase 27).
// BootFS proxy (/bin via the kernel initramfs VIFS1): Open/Close/ReadDir for
// listing (all synchronous), OpenCap/ReadCap/CloseCap for file reads — the FD
// `Read` syscall is deliberately ABSENT: it is an async transformation that
// requires the caller to park immediately, which a service dispatch loop
// cannot do (see backend_bootfs.rs::read_to_vec).
api::declare_syscalls![
    Send, Recv, TryRecv, Reply, Log, Heartbeat, LookupService,
    GrantAlloc, GrantShare, GrantSlice, GrantFree, BlkReadAsync,
    GrantRegister, GrantUnregister,
    StateStash, StateRestore,
    Open, Close, ReadDir, OpenCap, ReadCap, CloseCap,
];

// Global VFS manager for the fast-IPC handler (which runs outside the main recv loop).
// Protected by a spinlock; on single-hart there is no actual contention.
static GLOBAL_VFS: Mutex<Option<VfsManager>> = Mutex::new(None);

/// Fast-IPC handler: serves VfsRequest::GetFile without ecall overhead.
///
/// # Safety
/// Called with S-mode interrupts disabled (guaranteed by `ostd::fast_ipc::call_vfs`).
unsafe fn vfs_fast_handler(
    req: &api::ipc::VfsRequest<'_>,
    out: &mut [u8; api::ipc::IPC_BUF_SIZE],
) -> usize {
    let resp = match req {
        api::ipc::VfsRequest::GetFile(path) => {
            if let Some(vfs) = GLOBAL_VFS.lock().as_ref() {
                if let Some((ptr, len)) = vfs.get_file_ptr(path) {
                    api::ipc::VfsResponse::DataPtr { ptr: ptr as u64, len: len as u64 }
                } else {
                    api::ipc::VfsResponse::Err(1)
                }
            } else {
                api::ipc::VfsResponse::Err(0xFF)
            }
        }
        _ => api::ipc::VfsResponse::Err(0xFE), // other ops must use ecall path
    };
    api::ipc::encode(&resp, out).map(|s| s.len()).unwrap_or(0)
}

#[no_mangle]
pub fn main() {
    println("VFS Service v0.2: RamFS + mkdir/rmdir/unlink IPC (typed postcard)");
    // VfsManager::new() mounts all backends, including the FAT volume on the
    // VirtIO disk (which logs its own success/fallback status).
    let vfs = VfsManager::new();
    *GLOBAL_VFS.lock() = Some(vfs);

    // Register the fast-IPC handler so trusted Cells can bypass ecall for VFS reads.
    // The kernel records the VFS cell's ID at spawn time so it can clear this
    // pointer if VFS crashes — see loader.rs fast_ipc::set_vfs_handler_cell call.
    ostd::fast_ipc::register_vfs(vfs_fast_handler);
    let mut buf = [0u8; api::ipc::IPC_BUF_SIZE];

    loop {
        match ostd::syscall::sys_recv(0, &mut buf) {
            ostd::syscall::SyscallResult::Ok(sender) if sender > 0 => {
                // Encode the response into a local buffer while holding the VFS lock,
                // then DROP the lock before sys_send.  If ipc_send blocks (client not
                // yet in Recv), yield_cpu switches to another cell.  That cell may call
                // call_vfs which also acquires GLOBAL_VFS — a deadlock if we still hold
                // the lock during the send.
                let mut encoded = [0u8; api::ipc::IPC_BUF_SIZE];
                let encoded_len: usize;
                {
                    let mut resp_buf = [0u8; api::ipc::IPC_BUF_SIZE];
                    // Acquire VFS state; released at end of this block, before sys_send.
                    let mut gvfs = GLOBAL_VFS.lock();
                    let vfs = gvfs.as_mut().expect("VFS initialized before serving requests");
                    let resp = dispatch::handle_request(vfs, &buf, sender, &mut resp_buf);
                    // Encode while holding the lock (safe: no sys_send yet).
                    encoded_len = api::ipc::encode(&resp, &mut encoded).map(|s| s.len()).unwrap_or(0);
                } // GLOBAL_VFS lock released here — before sys_send

                // Send after releasing the lock so a blocked ipc_send + yield_cpu
                // cannot switch to a cell that deadlocks on GLOBAL_VFS.
                ostd::syscall::sys_send(sender, &encoded[..encoded_len]);
                buf = [0u8; api::ipc::IPC_BUF_SIZE];
            }
            _ => {
                ostd::task::yield_now();
            }
        }
    }
}
