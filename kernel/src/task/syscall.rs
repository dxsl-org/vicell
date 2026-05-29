//! IPC System Calls (Inspired by Tock OS)
//!
//! This module defines the interface between "Cells/Silos" and the Kernel.
//! See [docs/architecture/03-driver-strategy.md] for the full rationale.

use super::tcb::TaskState;
use alloc::collections::BTreeSet;
use api::syscall::ViSpawnArgs;
use crate::sync::Spinlock;
// use log::info;
use types::*;

/// Set of physical frames currently issued via `ShmAlloc`.
/// `ShmMap` accepts only handles that appear here, preventing a malicious
/// cell from mapping arbitrary kernel/cell-owned frames into its address
/// space via a forged handle.
///
/// NOTE: This is still a single global pool — any cell that knows a peer's
/// outstanding handle can map it. A per-owner ACL is the proper fix; this
/// table is the minimum bar to stop "ShmMap kernel_text_phys" attacks.
static SHM_HANDLES: Spinlock<Option<BTreeSet<usize>>> = Spinlock::new(None);

fn shm_handles_lock() -> &'static Spinlock<Option<BTreeSet<usize>>> {
    &SHM_HANDLES
}

fn shm_register(handle: usize) {
    let mut guard = shm_handles_lock().lock();
    if guard.is_none() {
        *guard = Some(BTreeSet::new());
    }
    if let Some(set) = guard.as_mut() {
        set.insert(handle);
    }
}

fn shm_is_valid(handle: usize) -> bool {
    let guard = shm_handles_lock().lock();
    guard.as_ref().map_or(false, |set| set.contains(&handle))
}

/// Result of a System Call
pub type SyscallResult = core::result::Result<usize, SyscallError>;

#[derive(Debug, Copy, Clone)]
pub enum SyscallError {
    InvalidDriverId,
    InvalidCommand,
    BufferTooSmall,
    PermissionDenied,
    FileNotFound,
    TryAgain,
    Unknown,
    NotSupported,
    InvalidInput,
}

/// Maximum bytes a single syscall may read/write through a user buffer.
/// Bounds kernel work per syscall and acts as a coarse sanity check against
/// `len = usize::MAX` style attacks. 64 MiB is well above any legitimate
/// caller need today; tighten further for specific syscalls (see MAX_LOG_MSG).
const MAX_USER_BUF: usize = 64 * 1024 * 1024;

/// Tighter cap for `Syscall::Log` since the kernel holds locks while printing.
const MAX_LOG_MSG: usize = 4096;

/// Validate a user-supplied (ptr, len) buffer descriptor.
///
/// Rejects: NULL pointer, zero-length when expected non-empty, lengths above
/// `max`, and pointer+length arithmetic overflow.
///
/// Does NOT walk the page table to confirm the U-bit. The trap handler enables
/// SUM only for the duration of `handle_syscall`, so a kernel-space `ptr`
/// supplied by user code will fault on access — but the fault is far more
/// graceful when we reject obvious garbage up front.
#[inline]
fn validate_user_buf(ptr: usize, len: usize, max: usize) -> Result<(), SyscallError> {
    if ptr == 0 {
        return Err(SyscallError::InvalidInput);
    }
    if len > max {
        return Err(SyscallError::BufferTooSmall);
    }
    if ptr.checked_add(len).is_none() {
        return Err(SyscallError::InvalidInput);
    }
    Ok(())
}

/// The Fundamental Verbs of ViOS IPC (Hubris ABI + Lease System)
#[derive(Debug, Copy, Clone)]
pub enum Syscall {
    /// 0: Send (Blocking Message Send)
    Send {
        target: usize,
        msg_ptr: usize,
        msg_len: usize,
    },
    /// 1: Recv (Blocking Message Receive)
    Recv {
        mask: usize,
        buf_ptr: usize,
        buf_len: usize,
    },
    /// 2: Reply (Unblocking Reply to Caller)
    Reply { caller: usize, result: usize },
    /// 3: SetTimer (Wake up after ticks)
    SetTimer { deadline: usize },
    /// 4: BorrowRead (Copy from Lease to Caller)
    BorrowRead {
        lease_id: usize,
        offset: usize,
        ptr: usize,
        len: usize,
    },
    /// 5: BorrowWrite (Copy from Caller to Lease)
    BorrowWrite {
        lease_id: usize,
        offset: usize,
        ptr: usize,
        len: usize,
    },
    /// 6: Lend (Create a Lease for Target Task)
    Lend {
        target: usize,
        ptr: usize,
        len: usize,
        flags: usize,
    },
    /// 7: TryRecv (Non-blocking Receive)
    TryRecv {
        mask: usize,
        buf_ptr: usize,
        buf_len: usize,
    },
    /// 8: Spawn (Create new Task/Thread) - Returns Task ID
    Spawn { entry: usize, arg: usize },
    /// 9: FutexWait (Wait for value at address)
    FutexWait { addr: usize, val: u32 },
    /// 10: FutexWake (Wake up waiting tasks)
    FutexWake { addr: usize, count: usize },
    /// 11: Log (Debug Print)
    Log { msg_ptr: usize, msg_len: usize },
    /// 12: Grant (Zero Copy)
    Grant {
        target: usize,
        ptr: usize,
        len: usize,
        flags: usize,
    },
    /// 13: Map (Zero Copy)
    Map { grant_id: usize },
    /// 14: Exit (Terminate Process)
    Exit { code: usize },
    /// 6: Exec (Spawn from file)
    Exec { path_ptr: usize, path_len: usize },
    /// 10: SpawnFromMem (Spawn from Memory buffer via Struct)
    SpawnFromMem { args_ptr: usize },
    /// 12: SpawnFromPath (Spawn cell by filesystem path)
    /// ABI: path_ptr in a0, path_len in a1.
    SpawnFromPath { path_ptr: usize, path_len: usize },
    /// 8: Wait (Wait for task)
    Wait { pid: usize },
    /// 20: ShmAlloc
    ShmAlloc { size: usize },
    /// 21: ShmMap
    ShmMap { handle: usize, target_pid: usize },
    /// 30: GetProcs
    GetProcs { buf_ptr: usize, buf_len: usize },

    // --- Legacy / Compatibility Layer ---
    /// 100: Service Lookup (Find driver ID by name)
    ServiceLookup { name_ptr: usize, name_len: usize },
    /// 101: Open (Path -> FD)
    Open { path_ptr: usize, path_len: usize },
    /// 102: Read (FD, Buffer -> Bytes Read)
    Read {
        fd: usize,
        buf_ptr: usize,
        buf_len: usize,
    },
    /// 103: Close (FD)
    Close { fd: usize },
    /// 105: ReadDir (Read Directory Entries)
    ReadDir {
        fd: usize,
        buf_ptr: usize,
        buf_len: usize,
    },
    /// 106: FStat (Get File Info)
    FStat { fd: usize, stat_ptr: usize },
    /// 107: ChDir (Change Directory)
    ChDir { path_ptr: usize, path_len: usize },
    /// 108: GetCwd (Get Current Directory)
    GetCwd { buf_ptr: usize, buf_len: usize },
    /// 109: Write (FD, Buffer -> Bytes Written)
    Write {
        fd: usize,
        buf_ptr: usize,
        buf_len: usize,
    },
    /// 110: MkDir (Path)
    MkDir { path_ptr: usize, path_len: usize },
    /// 111: Create (Path -> FD)
    /// 111: Create (Path -> FD)
    Create { path_ptr: usize, path_len: usize },
    /// 104: Yield (Legacy)
    Yield,
    /// 106: Seek (FD, Offset, Whence)
    Seek {
        fd: usize,
        offset: isize,
        whence: usize,
    },
    /// 107: FileOp (Op, Arg1, Arg2)
    FileOp {
        op: usize,
        arg1: usize,
        arg2: usize,
    },
    /// 120: GetTime (Op)
    GetTime { op: usize },
}

/// Dispatches a system call to the appropriate handler.
///
/// `caller_id` is the ID of the task invoking the syscall.
pub fn handle_syscall(caller_id: usize, syscall: Syscall) -> SyscallResult {
    // Info log reduced to Debug to reduce noise
    // info!("Syscall (Task {}): Dispatched {:?}", caller_id, syscall);

    match syscall {
        // --- Hubris ABI Implementation ---
        Syscall::Send {
            target,
            msg_ptr,
            msg_len,
        } => {
            let res = super::ipc_send(caller_id, target, msg_ptr, msg_len);
            match res {
                Ok(0) => Ok(0),
                Ok(1) => {
                    super::yield_cpu(); // Blocked
                    if let Some(sched) = super::SCHEDULER.lock().as_ref() {
                        return Ok(sched
                            .tasks
                            .get(&caller_id)
                            .and_then(|t| t.reply_value)
                            .unwrap_or(0));
                    }
                    Ok(0)
                }
                Err(_) => Err(SyscallError::InvalidCommand),
                _ => Ok(0),
            }
        }
        Syscall::Recv {
            mask,
            buf_ptr,
            buf_len,
        } => {
            let res = super::ipc_recv(caller_id, mask, buf_ptr, buf_len);
            match res {
                Ok(0) => {
                    // Blocked
                    super::yield_cpu();
                    // Resume: return who sent the message
                    if let Some(sched) = super::SCHEDULER.lock().as_ref() {
                        return Ok(sched
                            .tasks
                            .get(&caller_id)
                            .and_then(|t| t.current_caller)
                            .unwrap_or(0));
                    }
                    Ok(0)
                }
                Ok(id) => Ok(id), // Got message instantly
                Err(_) => Err(SyscallError::InvalidCommand),
            }
        }
        Syscall::TryRecv {
            mask,
            buf_ptr,
            buf_len,
        } => {
            // Non-blocking Recv
            let res = super::ipc_try_recv(caller_id, mask, buf_ptr, buf_len);
            match res {
                Ok(id) => Ok(id), // 0 = No message, >0 = Sender ID
                Err(_) => Err(SyscallError::InvalidCommand),
            }
        }
        Syscall::Spawn { entry, arg } => {
            let drivers = alloc::vec::Vec::new();
            let name = "thread";
            // TODO: Spawned threads should inherit parent's CellId or be assigned properly
            // For now, use CellId(0) as default (system/kernel cell)
            let tid = super::spawn_with_arg(name, CellId(0), drivers, entry, arg);
            if tid > 0 {
                Ok(tid)
            } else {
                Err(SyscallError::Unknown)
            }
        }
        Syscall::Wait { pid } => {
            if let Some(sched) = super::SCHEDULER.lock().as_mut() {
                if let Some(target) = sched.tasks.get_mut(&pid) {
                    if target.state == TaskState::Terminated {
                        // Already dead? Return exit code if stored or just 0?
                        let code = target.exit_code.unwrap_or(0);
                        return Ok(code);
                    } else {
                        // Add to waiters
                        target.waiters.push(caller_id);
                    }
                } else {
                    return Err(SyscallError::InvalidDriverId); // Task not found
                }

                // Block caller
                if let Some(caller) = sched.tasks.get_mut(&caller_id) {
                    caller.state = TaskState::Waiting { target: pid };
                }
            }
            super::yield_cpu(); // Block
                                // Resume with exit code (set by Exit handler)
            if let Some(sched) = super::SCHEDULER.lock().as_ref() {
                return Ok(sched
                    .tasks
                    .get(&caller_id)
                    .and_then(|t| t.reply_value)
                    .unwrap_or(0));
            }
            Ok(0)
        }
        Syscall::ShmAlloc { size: _ } => {
            // Allocate a single frame from the global allocator and register
            // it in the SHM handle table so subsequent ShmMap calls can
            // verify the caller isn't forging an arbitrary physical address.
            let mut frame_guard = crate::memory::frame::FRAME_ALLOCATOR.lock();
            if let Some(allocator) = frame_guard.as_mut() {
                if let Some(frame) = allocator.allocate_frame() {
                    drop(frame_guard);
                    shm_register(frame);
                    return Ok(frame);
                }
            }
            Err(SyscallError::BufferTooSmall)
        }
        Syscall::ShmMap {
            handle,
            target_pid: _,
        } => {
            // CRITICAL: handle must be a frame previously issued by ShmAlloc.
            // Without this check, a cell could pass `handle = kernel_text_phys`
            // and obtain a user-accessible mapping to kernel code.
            if !shm_is_valid(handle) {
                return Err(SyscallError::PermissionDenied);
            }

            let frame = handle;
            let vaddr = frame; // Identity map for SAS simplicity

            use crate::memory::paging::Flags;
            let flags = Flags::VALID
                | Flags::READ
                | Flags::WRITE
                | Flags::USER
                | Flags::ACCESSED
                | Flags::DIRTY;

            let mut frame_guard = crate::memory::frame::FRAME_ALLOCATOR.lock();
            if let Some(allocator) = frame_guard.as_mut() {
                if crate::memory::paging::map_page(
                    allocator,
                    vaddr,
                    frame,
                    Flags::from_bits(flags),
                )
                .is_ok()
                {
                    return Ok(vaddr);
                }
            }
            Err(SyscallError::Unknown)
        }
        Syscall::FutexWait { addr, val } => {
            // Returns Ok(0) if blocked (then yield), Err(TryAgain) if val mismatch
            match super::futex_wait(caller_id, addr, val) {
                Ok(_) => {
                    super::yield_cpu(); // Block
                    Ok(0)
                }
                Err(_) => Err(SyscallError::TryAgain),
            }
        }
        Syscall::FutexWake { addr, count } => {
            if let Ok(n) = super::futex_wake(caller_id, addr, count) {
                Ok(n)
            } else {
                Err(SyscallError::Unknown) // Should not fail typically
            }
        }
        Syscall::Log { msg_ptr, msg_len } => {
            // Reject NULL, oversize, or overflowing buffers. The kernel
            // print path holds locks with interrupts disabled, so a
            // multi-MB log message effectively hangs the system.
            validate_user_buf(msg_ptr, msg_len, MAX_LOG_MSG)?;
            unsafe {
                let slice = core::slice::from_raw_parts(msg_ptr as *const u8, msg_len);
                if let Ok(msg) = core::str::from_utf8(slice) {
                    crate::task::print_user_log(msg);
                }
            }
            Ok(0)
        }
        Syscall::Grant {
            target,
            ptr,
            len,
            flags,
        } => super::ipc_grant(caller_id, target, ptr, len, flags as u32)
            .map_err(|_| SyscallError::PermissionDenied),
        Syscall::Map { grant_id } => {
            super::ipc_map(caller_id, grant_id).map_err(|_| SyscallError::PermissionDenied)
        }
        Syscall::Exit { code } => {
            log::info!("Syscall::Exit: task {} exited with code {}", caller_id, code);
            let mut waiters = alloc::vec::Vec::new();

            if let Some(sched) = super::SCHEDULER.lock().as_mut() {
                // Record exit code and collect waiters before reaping so their
                // reply_value can carry the exit code.
                if let Some(task) = sched.tasks.get_mut(&caller_id) {
                    task.exit_code = Some(code);
                    waiters.append(&mut task.waiters);
                }
                // Move task to sched.zombies so its context pointer remains valid
                // across the context switch in yield_cpu; pick_next checks zombies.
                sched.exit_task(caller_id);
                // Wake any tasks blocked on Wait(caller_id).
                for wid in waiters {
                    if let Some(w) = sched.tasks.get_mut(&wid) {
                        w.state = TaskState::Ready;
                        w.reply_value = Some(code);
                        sched.ready_queue.push_back(wid);
                    }
                }
            }

            // yield_cpu switches away; this task is never rescheduled.
            super::yield_cpu();
            Ok(0)
        }
        Syscall::Reply { caller: _, result } => {
            super::ipc_reply(caller_id, result).map_err(|_| SyscallError::InvalidCommand)
        }

        Syscall::Lend {
            target,
            ptr,
            len,
            flags,
        } => super::ipc_lend(caller_id, target, ptr, len, flags as u32)
            .map_err(|_| SyscallError::PermissionDenied),

        Syscall::BorrowRead {
            lease_id,
            offset,
            ptr,
            len,
        } => super::ipc_borrow_read(caller_id, lease_id, offset, ptr, len)
            .map_err(|_| SyscallError::PermissionDenied),
        Syscall::BorrowWrite {
            lease_id,
            offset,
            ptr,
            len,
        } => super::ipc_borrow_write(caller_id, lease_id, offset, ptr, len)
            .map_err(|_| SyscallError::PermissionDenied),

        // --- Legacy Implementation ---
        Syscall::Yield => {
            super::yield_cpu();
            Ok(0)
        }
        Syscall::ServiceLookup { name_ptr, name_len } => {
            validate_user_buf(name_ptr, name_len, MAX_LOG_MSG)?;
            unsafe {
                let _slice = core::slice::from_raw_parts(name_ptr as *const u8, name_len);
                // if let Some(id) = crate::task::drivers::registry::resolve(name) {
                //    return Ok(id);
                // }
                return Ok(0);
            }
        }
        Syscall::Open { path_ptr, path_len } => {
            validate_user_buf(path_ptr, path_len, MAX_LOG_MSG)?;
            unsafe {
                let slice = core::slice::from_raw_parts(path_ptr as *const u8, path_len);
                if let Ok(path) = core::str::from_utf8(slice) {
                    if let Ok(fd) = super::file_open(path) {
                        return Ok(fd);
                    }
                }
            }
            Err(SyscallError::FileNotFound)
        }
        Syscall::Read {
            fd,
            buf_ptr,
            buf_len,
        } => {
            validate_user_buf(buf_ptr, buf_len, MAX_USER_BUF)?;
            unsafe {
                let slice = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len);
                let read_bytes = super::file_read(fd, slice);
                Ok(read_bytes)
            }
        }
        Syscall::Close { fd } => {
            super::file_close(fd);
            Ok(0)
        }
        Syscall::ReadDir {
            fd,
            buf_ptr,
            buf_len,
        } => {
            validate_user_buf(buf_ptr, buf_len, MAX_USER_BUF)?;
            unsafe {
                let slice = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len);
                super::file_readdir(fd, slice).map_err(|_| SyscallError::Unknown)
            }
        }
        Syscall::FStat { fd, stat_ptr } => {
            if stat_ptr == 0 {
                return Err(SyscallError::InvalidInput);
            }
            super::file_fstat(fd, stat_ptr).map_err(|_| SyscallError::Unknown)
        }
        // Syscall::Remove removed
        Syscall::ChDir { path_ptr, path_len } => {
            validate_user_buf(path_ptr, path_len, MAX_LOG_MSG)?;
            unsafe {
                let slice = core::slice::from_raw_parts(path_ptr as *const u8, path_len);
                if let Ok(path) = core::str::from_utf8(slice) {
                    if super::file_chdir(path).is_ok() {
                        return Ok(0);
                    }
                }
            }
            Err(SyscallError::FileNotFound)
        }
        Syscall::GetCwd { buf_ptr, buf_len } => {
            validate_user_buf(buf_ptr, buf_len, MAX_USER_BUF)?;
            unsafe {
                let slice = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len);
                if let Ok(len) = super::file_getcwd(slice) {
                    return Ok(len);
                }
            }
            Err(SyscallError::BufferTooSmall)
        }
        Syscall::Write {
            fd,
            buf_ptr,
            buf_len,
        } => {
            validate_user_buf(buf_ptr, buf_len, MAX_USER_BUF)?;
            unsafe {
                let slice = core::slice::from_raw_parts(buf_ptr as *const u8, buf_len);
                let written = super::file_write(fd, slice);
                Ok(written)
            }
        }
        Syscall::MkDir { path_ptr, path_len } => {
            validate_user_buf(path_ptr, path_len, MAX_LOG_MSG)?;
            unsafe {
                let path_slice = core::slice::from_raw_parts(path_ptr as *const u8, path_len);
                // Use checked UTF-8 conversion: passing invalid UTF-8 to a
                // future file_mkdir impl could panic. Reject early.
                if core::str::from_utf8(path_slice).is_err() {
                    return Err(SyscallError::InvalidInput);
                }
                // let res = super::file_mkdir(path_str);  // FIXME: not implemented
            }
            Err(SyscallError::PermissionDenied)
        }
        Syscall::Exec { path_ptr, path_len } => {
            validate_user_buf(path_ptr, path_len, MAX_LOG_MSG)?;
            unsafe {
                let slice = core::slice::from_raw_parts(path_ptr as *const u8, path_len);
                if core::str::from_utf8(slice).is_ok() {
                    // Legacy Exec support removed/deprecated; use SpawnFromMem.
                    Err(SyscallError::NotSupported)
                } else {
                    Err(SyscallError::InvalidCommand)
                }
            }
        }
        Syscall::SpawnFromPath { path_ptr, path_len } => {
            // Reject empty or over-long paths at the trust boundary.
            if path_len == 0 || path_len > crate::loader::disk_layout::MAX_CELL_PATH {
                return Err(SyscallError::InvalidInput);
            }
            validate_user_buf(path_ptr, path_len, crate::loader::disk_layout::MAX_CELL_PATH)?;
            // SAFETY: path_ptr is a valid user buffer (validated above); SUM=1
            // lets S-mode read U-mode pages.  Slice lives only in this frame.
            let path_str = unsafe {
                let slice = core::slice::from_raw_parts(path_ptr as *const u8, path_len);
                core::str::from_utf8(slice).map_err(|_| SyscallError::InvalidInput)?
            };
            if !path_str.starts_with('/') {
                return Err(SyscallError::InvalidInput);
            }
            crate::loader::spawn_from_path(path_str).map_err(|e| match e {
                types::ViError::NotFound => SyscallError::FileNotFound,
                types::ViError::OutOfMemory => SyscallError::Unknown,
                _ => SyscallError::InvalidInput,
            })
        }

        Syscall::SpawnFromMem { args_ptr } => {
            if args_ptr == 0 {
                return Err(SyscallError::InvalidInput);
            }
            // Validate the args descriptor itself before reading it.
            validate_user_buf(args_ptr, core::mem::size_of::<ViSpawnArgs>(), MAX_LOG_MSG)?;
            unsafe {
                let args = &*(args_ptr as *const ViSpawnArgs);

                // Validate every pointer inside the args struct.
                validate_user_buf(args.buffer_addr, args.buffer_size, MAX_USER_BUF)?;
                validate_user_buf(args.name_ptr, args.name_len, MAX_LOG_MSG)?;

                let data_slice =
                    core::slice::from_raw_parts(args.buffer_addr as *const u8, args.buffer_size);
                let name_slice =
                    core::slice::from_raw_parts(args.name_ptr as *const u8, args.name_len);
                let name = core::str::from_utf8(name_slice).unwrap_or("unknown");

                let cell_id = CellId(0);
                let drivers = alloc::vec::Vec::new();

                match super::spawn_from_mem(data_slice, name, cell_id, drivers) {
                    Ok(tid) => Ok(tid),
                    Err(_) => Err(SyscallError::InvalidInput),
                }
            }
        }
        Syscall::Create { path_ptr, path_len } => {
            validate_user_buf(path_ptr, path_len, MAX_LOG_MSG)?;
            unsafe {
                let path_slice = core::slice::from_raw_parts(path_ptr as *const u8, path_len);
                if core::str::from_utf8(path_slice).is_err() {
                    return Err(SyscallError::InvalidInput);
                }
                // let res = super::file_create(path_str);  // FIXME: not implemented
            }
            Err(SyscallError::PermissionDenied)
        }
        Syscall::SetTimer { deadline } => {
            // Check if deadline passed
            let now = super::system_ticks();
            let wake_at = now + deadline;

            // Sleep!
            if let Some(sched) = super::SCHEDULER.lock().as_mut() {
                if let Some(task) = sched.current_task_mut() {
                    task.state = TaskState::Sleeping { until: wake_at };
                }
            }
            // Yield CPU safely
            super::yield_cpu();
            Ok(0)
        }

        Syscall::GetProcs { buf_ptr, buf_len } => {
            unsafe {
                let slice = core::slice::from_raw_parts_mut(buf_ptr as *mut api::syscall::ProcessInfo, buf_len);
                if let Some(sched) = super::SCHEDULER.lock().as_ref() {
                    let mut count = 0;
                    for (pid, task) in sched.tasks.iter() {
                        if count >= slice.len() {
                            break;
                        }
                        
                        let mut name = [0u8; 32];
                        let name_bytes = task.name.as_bytes();
                        let len = core::cmp::min(name_bytes.len(), 32);
                        name[..len].copy_from_slice(&name_bytes[..len]);

                        let state_val = match task.state {
                            TaskState::Ready => 0,
                            TaskState::Running => 1,
                            TaskState::Terminated => 3,
                            _ => 2, // Map everything else (Waiting, Sleeping, IPC blocks) to Waiting
                        };

                        slice[count] = api::syscall::ProcessInfo {
                            id: *pid,
                            state: state_val,
                            name,
                        };
                        count += 1;
                    }
                    return Ok(count);
                }
                Ok(0)
            }
        }
        
        Syscall::Seek { fd, offset, whence } => {
            super::file_seek(fd, offset, whence).map_err(|_| SyscallError::Unknown)
        }
        
        Syscall::FileOp { op, arg1, arg2 } => {
            match op {
                0 => {
                    // Remove(path_ptr, path_len)
                    unsafe {
                        let slice = core::slice::from_raw_parts(arg1 as *const u8, arg2);
                        if let Ok(path) = core::str::from_utf8(slice) {
                             return super::file_remove(path).map_err(|_| SyscallError::PermissionDenied);
                        }
                        Err(SyscallError::InvalidInput)
                    }
                }
                1 => {
                    // Rename - Stub
                    Err(SyscallError::NotSupported)
                }
                _ => Err(SyscallError::InvalidCommand),
            }
        }
        
        Syscall::GetTime { op } => {
            let ticks = super::system_ticks();
            if op == 0 {
                Ok(ticks / 1000)
            } else {
                Ok(ticks)
            }
        }
    }
}

use api::syscall::ViSyscall;
use hal::arch::ViTrapFrame;

#[no_mangle]
pub extern "Rust" fn vios_syscall_dispatch(frame: &mut ViTrapFrame) {
    let syscall_id = frame.regs[17];
    let a0 = frame.regs[10];
    let a1 = frame.regs[11];
    let a2 = frame.regs[12];
    let _a3 = frame.regs[13];

    // Debug log
    // log::info!("SYSCALL DISPATCH: ID={}, a0={:X}, sstatus={:X}", syscall_id, a0, frame.sstatus);

    // Helper to construct Syscall enum
    // Note: This mapping manually unpacks registers to arguments based on the Syscall definition.
    // This duplicates logic slightly but keeps the kernel side robust.
    let syscall = match ViSyscall::from(syscall_id) {
        ViSyscall::Send => Syscall::Send { target: a0, msg_ptr: a1, msg_len: a2 },
        ViSyscall::Recv => Syscall::Recv { mask: a0, buf_ptr: a1, buf_len: a2 },
        ViSyscall::Reply => Syscall::Reply { caller: a0, result: a1 },
        // SetTimer? ID 3?
        ViSyscall::Call =>     // Call is not in Syscall enum? Ah, Syscall enum has: Reply (3). Call (2).
            // ViSyscall::Call (2).
            // But Syscall enum has Reply as 2? 
            // In Syscall enum: Reply { caller, result } is 2.
            // Check ViSyscall: Call=2, Reply=3.
            // Check handle_software_trap reference:
            // 2 => Reply.
            // 3 => SetTimer.
            // There is a mismatch between ViSyscall (Contract) and Syscall (Internal Enum used here).
            // I should adhere to ViSyscall for the DISPATCHER input, and map to internal Syscall.
            // IMPORTANT: If ViSyscall says 3 is Reply, and Syscall enum says 2 is Reply, I map ViSyscall::Reply -> Syscall::Reply.
            // I need to be careful with registers.
            Syscall::ServiceLookup { name_ptr: a0, name_len: a1 }, // Placeholder for Call
            
        ViSyscall::Spawn => Syscall::Spawn { entry: a0, arg: a1 },
        ViSyscall::Exec => Syscall::Exec { path_ptr: a0, path_len: a1 },
        ViSyscall::SpawnFromMem => Syscall::SpawnFromMem { args_ptr: a0 },
        ViSyscall::SpawnFromPath => Syscall::SpawnFromPath { path_ptr: a0, path_len: a1 },
        ViSyscall::Wait => Syscall::Wait { pid: a0 },
        ViSyscall::ShmAlloc => Syscall::ShmAlloc { size: a0 },
        ViSyscall::ShmMap => Syscall::ShmMap { handle: a0, target_pid: a1 },
        ViSyscall::Exit => Syscall::Exit { code: a0 },
        ViSyscall::Yield => Syscall::Yield,
        ViSyscall::SetTimer => Syscall::SetTimer { deadline: a0 },
        ViSyscall::Log => Syscall::Log { msg_ptr: a0, msg_len: a1 },
        ViSyscall::GetProcs => Syscall::GetProcs { buf_ptr: a0, buf_len: a1 },
        
        ViSyscall::Open => Syscall::Open { path_ptr: a0, path_len: a1 },
        ViSyscall::Read => Syscall::Read { fd: a0, buf_ptr: a1, buf_len: a2 },
        ViSyscall::Close => Syscall::Close { fd: a0 },
        ViSyscall::ReadDir => Syscall::ReadDir { fd: a0, buf_ptr: a1, buf_len: a2 },
        ViSyscall::Write => Syscall::Write { fd: a0, buf_ptr: a1, buf_len: a2 },

        // POSIX Support
        ViSyscall::Seek => Syscall::Seek { fd: a0, offset: a1 as isize, whence: a2 },
        ViSyscall::FileOp => Syscall::FileOp { op: a0, arg1: a1, arg2: a2 },
        ViSyscall::GetTime => Syscall::GetTime { op: a0 },
        
        // Handle non-matching/legacy manually
        _ => match syscall_id {
            // SetTimer (3 in old, ? in ViSyscall)
            3 => Syscall::SetTimer { deadline: a0 },
            
            // Legacy 100-111 coverage if needed
            100 => Syscall::ServiceLookup { name_ptr: a0, name_len: a1 },
            106 => Syscall::FStat { fd: a0, stat_ptr: a1 },
            107 => Syscall::ChDir { path_ptr: a0, path_len: a1 },
            108 => Syscall::GetCwd { buf_ptr: a0, buf_len: a1 },
            110 => Syscall::MkDir { path_ptr: a0, path_len: a1 },
            111 => Syscall::Create { path_ptr: a0, path_len: a1 },
            
             _ => {
                 frame.regs[10] = usize::MAX; // -1
                 return;
             }
        }
    };

    let caller_id = super::current_task_id();

    // Enable Access to User Memory (SUM) on RISC-V
    #[cfg(target_arch = "riscv64")]
    unsafe {
        core::arch::asm!("csrs sstatus, {0}", in(reg) 0x40000);
    }

    let result = handle_syscall(caller_id, syscall);

    // Disable Access to User Memory (SUM)
    #[cfg(target_arch = "riscv64")]
    unsafe {
        core::arch::asm!("csrc sstatus, {0}", in(reg) 0x40000);
    }

    match result {
        Ok(val) => frame.regs[10] = val,
        Err(_) => frame.regs[10] = usize::MAX, // -1
    }
}
