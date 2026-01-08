//! IPC System Calls (Inspired by Tock OS)
//!
//! This module defines the interface between "Cells/Silos" and the Kernel.
//! See [docs/architecture/03-driver-strategy.md] for the full rationale.


use super::tcb::TaskState;
use alloc::string::String;
use alloc::vec::Vec;
use log::info;
use types::*;
use api::syscall::ViSpawnArgs;


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
}

/// The Fundamental Verbs of ViOS IPC (Hubris ABI + Lease System)
#[derive(Debug, Copy, Clone)]
pub enum Syscall {
    /// 0: Send (Blocking Message Send)
    Send { target: usize, msg_ptr: usize, msg_len: usize },
    /// 1: Recv (Blocking Message Receive)
    Recv { mask: usize, buf_ptr: usize, buf_len: usize },
    /// 2: Reply (Unblocking Reply to Caller)
    Reply { caller: usize, result: usize },
    /// 3: SetTimer (Wake up after ticks)
    SetTimer { deadline: usize },
    /// 4: BorrowRead (Copy from Lease to Caller)
    BorrowRead { lease_id: usize, offset: usize, ptr: usize, len: usize },
    /// 5: BorrowWrite (Copy from Caller to Lease)
    BorrowWrite { lease_id: usize, offset: usize, ptr: usize, len: usize },
    /// 6: Lend (Create a Lease for Target Task)
    Lend { target: usize, ptr: usize, len: usize, flags: usize },
    /// 7: TryRecv (Non-blocking Receive)
    TryRecv { mask: usize, buf_ptr: usize, buf_len: usize },
    /// 8: Spawn (Create new Task/Thread) - Returns Task ID
    Spawn { entry: usize, arg: usize },
    /// 9: FutexWait (Wait for value at address)
    FutexWait { addr: usize, val: u32 },
    /// 10: FutexWake (Wake up waiting tasks)
    FutexWake { addr: usize, count: usize },
    /// 11: Log (Debug Print)
    Log { msg_ptr: usize, msg_len: usize },
    /// 12: Grant (Zero Copy)
    Grant { target: usize, ptr: usize, len: usize, flags: usize },
    /// 13: Map (Zero Copy)
    Map { grant_id: usize },
    /// 14: Exit (Terminate Process)
    Exit { code: usize },
    /// 6: Exec (Spawn from file)
    Exec { path_ptr: usize, path_len: usize },
    /// 10: SpawnFromMem (Spawn from Memory buffer via Struct)
    SpawnFromMem { args_ptr: usize },
    /// 8: Wait (Wait for task)
    Wait { pid: usize },
    /// 20: ShmAlloc
    ShmAlloc { size: usize },
    /// 21: ShmMap
    ShmMap { handle: usize, target_pid: usize },
    
    // --- Legacy / Compatibility Layer ---
    /// 100: Service Lookup (Find driver ID by name)
    ServiceLookup { name_ptr: usize, name_len: usize },
    /// 101: Open (Path -> FD)
    Open { path_ptr: usize, path_len: usize },
    /// 102: Read (FD, Buffer -> Bytes Read)
    Read { fd: usize, buf_ptr: usize, buf_len: usize },
    /// 103: Close (FD)
    Close { fd: usize },
    /// 105: ReadDir (Read Directory Entries)
    ReadDir { fd: usize, buf_ptr: usize, buf_len: usize },
    /// 106: FStat (Get File Info)
    FStat { fd: usize, stat_ptr: usize },
    /// 107: ChDir (Change Directory)
    ChDir { path_ptr: usize, path_len: usize },
    /// 108: GetCwd (Get Current Directory)
    GetCwd { buf_ptr: usize, buf_len: usize },
    /// 109: Write (FD, Buffer -> Bytes Written)
    Write { fd: usize, buf_ptr: usize, buf_len: usize },
    /// 110: MkDir (Path)
    MkDir { path_ptr: usize, path_len: usize },
    /// 111: Create (Path -> FD)
    Create { path_ptr: usize, path_len: usize },
    /// 104: Yield (Legacy)
    Yield,
}

/// Dispatches a system call to the appropriate handler.
/// 
/// `caller_id` is the ID of the task invoking the syscall.
pub fn handle_syscall(caller_id: usize, syscall: Syscall) -> SyscallResult {
    // Info log reduced to Debug to reduce noise
    // info!("Syscall (Task {}): Dispatched {:?}", caller_id, syscall);

    match syscall {
        // --- Hubris ABI Implementation ---
        Syscall::Send { target, msg_ptr, msg_len } => {
            let res = super::ipc_send(caller_id, target, msg_ptr, msg_len);
            match res {
                Ok(0) => Ok(0),
                Ok(1) => {
                    super::yield_cpu(); // Blocked
                    if let Some(sched) = super::SCHEDULER.lock().as_ref() {
                        return Ok(sched.tasks.get(&caller_id).and_then(|t| t.reply_value).unwrap_or(0));
                    }
                    Ok(0)
                }
                Err(_) => Err(SyscallError::InvalidCommand),
                _ => Ok(0)
            }
        }
        Syscall::Recv { mask, buf_ptr, buf_len } => {
            let res = super::ipc_recv(caller_id, mask, buf_ptr, buf_len);
            match res {
                Ok(0) => {
                    // Blocked
                    super::yield_cpu();
                    // Resume: return who sent the message
                    if let Some(sched) = super::SCHEDULER.lock().as_ref() {
                        return Ok(sched.tasks.get(&caller_id).and_then(|t| t.current_caller).unwrap_or(0));
                    }
                    Ok(0)
                }
                Ok(id) => Ok(id), // Got message instantly
                Err(_) => Err(SyscallError::InvalidCommand)
            }
        }
        Syscall::TryRecv { mask, buf_ptr, buf_len } => {
            // Non-blocking Recv
            let res = super::ipc_try_recv(caller_id, mask, buf_ptr, buf_len);
            match res {
                Ok(id) => Ok(id), // 0 = No message, >0 = Sender ID
                Err(_) => Err(SyscallError::InvalidCommand)
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
                return Ok(sched.tasks.get(&caller_id).and_then(|t| t.reply_value).unwrap_or(0));
            }
            Ok(0)
        }
        Syscall::ShmAlloc { size } => {
            // Allocate a global frame
            // For MVP, we just allocate from frame allocator and return physical address as handle?
            // Handle MUST be secure.
            // We use PAddr as Handle for now (Insecure but works for single user SAS logic).
            let mut frame_guard = crate::memory::frame::FRAME_ALLOCATOR.lock();
            if let Some(allocator) = frame_guard.as_mut() {
                if let Some(frame) = allocator.allocate_frame() {
                    return Ok(frame);
                }
            }
            Err(SyscallError::BufferTooSmall)
        }
        Syscall::ShmMap { handle, target_pid } => {
            // Map the frame (handle) into target_pid's address space.
            // We need to find a free VAddr in target.
            // Simplified: Map at Identity + Offset? Or hardcoded region?
            // Let's map at 0x8000_0000 + handle (if handle is small offset?).
            // Handle is PhysAddr (e.g. 0x80200000).
            // We map it to VAddr = Handle (Identity) for SAS simplicity if not already mapped.
            // Actually, in SAS, if we use Identity Mapping for User Space (which we do for now mostly),
            // then ShmAlloc returning PAddr is enough! The user can just use it?
            // NO. User runs in U-mode. They can only access mapped pages.
            // We need to ensure PAddr is mapped with U-bit.

            let frame = handle;
            let vaddr = frame; // Identity map for simplicity

            // Map it for Target
            // Permissions: R/W/U
            use crate::memory::paging::Flags;
            let flags = Flags::VALID | Flags::READ | Flags::WRITE | Flags::USER | Flags::ACCESSED | Flags::DIRTY;

            // We need access to Frame Allocator to clone the mapping?
            // Or just update the Page Table.
            // Page Table is shared? SAS means One Page Table?
            // If SAS means One Page Table, then mapping it once makes it available to ALL.
            // ViOS SAS: "Single Address Space". Yes.
            // So ShmMap just ensures it is mapped with U permission.

            let mut frame_guard = crate::memory::frame::FRAME_ALLOCATOR.lock();
            if let Some(allocator) = frame_guard.as_mut() {
                 unsafe {
                     // Check if already mapped?
                     // We just force map.
                     if crate::memory::paging::map_page(allocator, vaddr, frame, Flags::from_bits(flags.bits())).is_ok() {
                         return Ok(vaddr);
                     }
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
                },
                Err(_) => Err(SyscallError::TryAgain) 
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
             log::info!("Syscall::Log: ptr=0x{:X}, len={}", msg_ptr, msg_len);
             if msg_ptr == 0 {
                 log::error!("Syscall::Log called with NULL pointer!");
                 return Ok(0);
             }
             unsafe {
                let slice = core::slice::from_raw_parts(msg_ptr as *const u8, msg_len);
                if let Ok(msg) = core::str::from_utf8(slice) {
                    // Use crate::io::_print or similar?
                    // Kernel usually has a logger. info! is from 'log' crate.
                    crate::task::print_user_log(msg);
                }
             }
             Ok(0)
        }
        Syscall::Grant { target, ptr, len, flags } => {
             super::ipc_grant(caller_id, target, ptr, len, flags as u32).map_err(|_| SyscallError::PermissionDenied)
        }
        Syscall::Map { grant_id } => {
             super::ipc_map(caller_id, grant_id).map_err(|_| SyscallError::PermissionDenied)
        }
        Syscall::Exit { code } => {
            log::info!("Syscall::Exit handler called for {} with code {}", caller_id, code);
            let mut waiters = alloc::vec::Vec::new();

            if let Some(sched) = super::SCHEDULER.lock().as_mut() {
                // sched.exit_task(caller_id); // Don't remove yet! Mark Terminated.
                if let Some(task) = sched.tasks.get_mut(&caller_id) {
                    task.state = TaskState::Terminated;
                    task.exit_code = Some(code);
                    // Steal waiters
                    waiters.append(&mut task.waiters);
                }

                // Wake up waiters
                for wid in waiters {
                    if let Some(w) = sched.tasks.get_mut(&wid) {
                        w.state = TaskState::Ready;
                        w.reply_value = Some(code);
                        sched.ready_queue.push_back(wid);
                    }
                }
            }

            super::yield_cpu(); 
            // Should not return
            Ok(0) 
        }
        Syscall::Reply { caller: _, result } => {              super::ipc_reply(caller_id, result).map_err(|_| SyscallError::InvalidCommand)
        }
        
        Syscall::Lend { target, ptr, len, flags } => {
            super::ipc_lend(caller_id, target, ptr, len, flags as u32).map_err(|_| SyscallError::PermissionDenied)
        }
        
        Syscall::BorrowRead { lease_id, offset, ptr, len } => {
             super::ipc_borrow_read(caller_id, lease_id, offset, ptr, len).map_err(|_| SyscallError::PermissionDenied)
        }
        Syscall::BorrowWrite { lease_id, offset, ptr, len } => {
             super::ipc_borrow_write(caller_id, lease_id, offset, ptr, len).map_err(|_| SyscallError::PermissionDenied)
        }
        
        // --- Legacy Implementation ---
        Syscall::Yield => {
            super::yield_cpu();
            Ok(0)
        }
        Syscall::ServiceLookup { name_ptr, name_len } => {
            unsafe {
                let slice = core::slice::from_raw_parts(name_ptr as *const u8, name_len);
                // if let Some(id) = crate::task::drivers::registry::resolve(name) {
                //    return Ok(id);
                // }
                return Ok(0);
            }
            Err(SyscallError::InvalidDriverId)
        }
        Syscall::Open { path_ptr, path_len } => {
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
        Syscall::Read { fd, buf_ptr, buf_len } => {
            unsafe {
                 let slice = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len);
                 let read_bytes = super::file_read(fd, slice);
                 return Ok(read_bytes);
            }
        }
        Syscall::Close { fd } => {
            super::file_close(fd);
            Ok(0)
        }
        Syscall::ReadDir { fd, buf_ptr, buf_len } => {
            unsafe {
                 let slice = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len);
                 return super::file_readdir(fd, slice).map_err(|_| SyscallError::Unknown);
            }
        }
        Syscall::FStat { fd, stat_ptr } => {
            super::file_fstat(fd, stat_ptr).map_err(|_| SyscallError::Unknown)
        }
        // Syscall::Remove removed

        Syscall::ChDir { path_ptr, path_len } => {
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
             unsafe {
                 let slice = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len);
                 if let Ok(len) = super::file_getcwd(slice) {
                    return Ok(len);
                 }
                 // return Ok(0);
             }
             Err(SyscallError::BufferTooSmall)
        }
        Syscall::Write { fd, buf_ptr, buf_len } => {
             unsafe {
                 let slice = core::slice::from_raw_parts(buf_ptr as *const u8, buf_len);
                 let written = super::file_write(fd, slice);
                 Ok(written)
             }
        }
        Syscall::MkDir { path_ptr, path_len } => {
             unsafe {
                 let path_slice = core::slice::from_raw_parts(path_ptr as *const u8, path_len);
                 let path_str = core::str::from_utf8_unchecked(path_slice);
                 // let res = super::file_mkdir(path_str);  // FIXME: not implemented
                 let res: core::result::Result<usize, ()> = Err(());  // Temporary
                 if res.is_ok() {
                         return Ok(0); // MkDir returns 0 on success
                     }
                 }
             Err(SyscallError::PermissionDenied)
        }
        Syscall::Exec { path_ptr, path_len } => {
             unsafe {
                 let slice = core::slice::from_raw_parts(path_ptr as *const u8, path_len);
                 if let Ok(path) = core::str::from_utf8(slice) {
                     // Legacy Exec support removed/depreciated
                     // We should use SpawnFromMem for modern apps
                     Err(SyscallError::NotSupported)
                 } else {
                     Err(SyscallError::InvalidCommand)
                 }
             }
        }
        Syscall::SpawnFromMem { args_ptr } => {
             unsafe {
                 // Read struct from user pointer
                 let args = &*(args_ptr as *const ViSpawnArgs);

                 let data_slice = core::slice::from_raw_parts(args.buffer_addr as *const u8, args.buffer_size);
                 let name_slice = core::slice::from_raw_parts(args.name_ptr as *const u8, args.name_len);
                 let name = core::str::from_utf8(name_slice).unwrap_or("unknown");

                 // TODO: Handle args.args_ptr (Command Line Arguments)
                 // For now, ignore args or pass them to spawn_from_mem?
                 // spawn_from_mem needs update to take args?

                 let cell_id = CellId(0);
                 let drivers = alloc::vec::Vec::new();

                 match super::spawn_from_mem(data_slice, name, cell_id, drivers) {
                     Ok(tid) => Ok(tid),
                     Err(_) => Err(SyscallError::InvalidInput),
                 }
             }
        }
        Syscall::Create { path_ptr, path_len } => {
             unsafe {
                 let path_slice = core::slice::from_raw_parts(path_ptr as *const u8, path_len);
                 let path_str = core::str::from_utf8_unchecked(path_slice);
                 // let res = super::file_create(path_str);  // FIXME: not implemented
                 let res: core::result::Result<usize, ()> = Err(());  // Temporary
                 if let Ok(fd) = res { // Assuming res would be Ok(fd) on success
                         return Ok(fd);
                     }
                 }
             Err(SyscallError::PermissionDenied)
        }
        Syscall::Read { fd, buf_ptr, buf_len } => {
            if fd == 0 {
                // Console Input (Stdin)
                // Busy-loop with yield until a character is available
                loop {
                    let mut byte_opt = None;
                    {
                        let mut cons = crate::task::drivers::console_drv::CONSOLE.lock();
                        // Poll hardware
                        cons.poll();
                        // Check buffer
                        byte_opt = cons.read_byte();
                    }
                    
                     if let Some(byte) = byte_opt {
                         // log::info!("Syscall::Read: Got byte {}", byte);
                         unsafe {
                             let slice = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len);
                             if slice.len() > 0 {
                                 slice[0] = byte;
                                 return Ok(1);
                             } else {
                                 return Ok(0);
                             }
                         }
                    }
                    
                    // No input yet, yield and try again
                    super::yield_cpu();
                }
            } else {
                 // Filesystem Read not linked yet
                 Err(SyscallError::InvalidDriverId)
            }
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
    let a3 = frame.regs[13];
    
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
        ViSyscall::Wait => Syscall::Wait { pid: a0 },
        ViSyscall::ShmAlloc => Syscall::ShmAlloc { size: a0 },
        ViSyscall::ShmMap => Syscall::ShmMap { handle: a0, target_pid: a1 },
        ViSyscall::Exit => Syscall::Exit { code: a0 },
        ViSyscall::Yield => Syscall::Yield,
        ViSyscall::SetTimer => Syscall::SetTimer { deadline: a0 },
        ViSyscall::Log => Syscall::Log { msg_ptr: a0, msg_len: a1 },
        
        ViSyscall::Open => Syscall::Open { path_ptr: a0, path_len: a1 },
        ViSyscall::Read => Syscall::Read { fd: a0, buf_ptr: a1, buf_len: a2 },
        ViSyscall::Close => Syscall::Close { fd: a0 },
        ViSyscall::ReadDir => Syscall::ReadDir { fd: a0, buf_ptr: a1, buf_len: a2 },
        ViSyscall::Write => Syscall::Write { fd: a0, buf_ptr: a1, buf_len: a2 },
        
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
