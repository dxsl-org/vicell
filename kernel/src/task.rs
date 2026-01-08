pub mod syscall;
pub mod tcb;
pub use tcb::Task;
pub mod stack;
pub mod scheduler;
pub mod drivers;
pub mod ipc_test;

#[cfg(test)]
mod tests;

use scheduler::Scheduler;
use log::info;
use crate::sync::Spinlock;
use alloc::string::String;
pub const STACK_PAGES: usize = 16;
const TRAP_FRAME_SIZE: usize = 288;
extern "C" { fn __trap_exit(); }

use alloc::vec::Vec;
use types::*;


// Global Scheduler Instance
pub(crate) static SCHEDULER: Spinlock<Option<Scheduler>> = Spinlock::new(None);

// Global Tick Counter
static TICKS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

// Helper context to save the initial boot/kernel state during first task switch
// Helper context to save the initial boot/kernel state during first task switch
static mut BOOT_CONTEXT: crate::hal::arch::Context = crate::hal::arch::Context {
    ra: 0, sp: 0, s0: 0, s1: 0, s2: 0, s3: 0, s4: 0, s5: 0, s6: 0, s7: 0, s8: 0, s9: 0, s10: 0, s11: 0, sepc: 0, sstatus: 0x102, gp: 0, tp: 0, sscratch: 0,
};

// Trampoline for Thread Spawning
// Trampoline for Thread Spawning handled by HAL

extern "C" {
    // pub fn thread_trampoline(); // In HAL
}

pub fn get_kernel_gp_tp() -> (usize, usize) {
    crate::hal::arch::get_gp_tp()
}

pub fn system_ticks() -> usize {
    TICKS.load(core::sync::atomic::Ordering::Relaxed)
}

pub fn tick() {
    TICKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

pub fn init() {
    info!("Process: Initializing Scheduler...");
    let mut sched_guard = SCHEDULER.lock();
    
    // SAFETY: Use ptr::write to overwrite the Spinlock guard's data WITHOUT dropping the old value.
    // This prevents "Freed node aliases existing hole" panic on soft reboot (where .data persists but Heap is reset).
    unsafe {
        core::ptr::write(&mut *sched_guard, Some(Scheduler::new()));
    }

    // unsafe {
    //     ostd::syscall::register_trap_handler(crate::task::syscall::handle_software_trap);
    // }
    
    // if let Some(s) = sched_guard.as_mut() {
    //     // TODO: Spawn init task via proper ELF loading
    // }
}

/// Core scheduling logic: picks next task and performs switch OUTSIDE of the lock.
pub fn yield_cpu() {
    let switch_info = if let Some(sched) = SCHEDULER.lock().as_mut() {
        sched.pick_next()
    } else {
        None
    };

    if let Some((curr, next)) = switch_info {
        unsafe {
            // use crate::arch::context::Context;
            
            let final_curr = if curr.is_null() {
                &mut BOOT_CONTEXT as *mut _
            } else {
                curr
            };
            
            // Handle null next as switch to BOOT_CONTEXT (Idle)
            let final_next = if next.is_null() {
                &BOOT_CONTEXT as *const _
            } else {
                next
            };
            
            if !next.is_null() {
                // Set sscratch for next task's kernel stack (needed for U-mode trap handling)
                let next_ref = &*next;
                crate::hal::arch::set_kernel_stack(next_ref.sp);
            }
            if next.is_null() {
                // log::info!("yield_cpu: Switching to BOOT_CONTEXT");
            }

            // switch(current, next)
            crate::hal::arch::Context::switch(final_curr, final_next);
            
            // if next.is_null() {
            //      log::info!("yield_cpu: Resumed execution (BOOT_CONTEXT)");
            // }
            
            // NOTE: Code sau Context::switch sẽ KHÔNG chạy vì nó có noreturn!
            // Khi task được switch về, nó sẽ tiếp tục từ nơi nó đã yield.
        }
    }
}

pub fn spawn(name: &str, cell_id: CellId, allowed_drivers: alloc::vec::Vec<usize>) -> usize {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        sched.spawn(name, cell_id, allowed_drivers)
    } else {
        0
    }
}


pub fn spawn_with_arg(name: &str, cell_id: CellId, allowed_drivers: alloc::vec::Vec<usize>, entry: VAddr, arg: usize) -> usize {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        sched.spawn_thread(name, cell_id, allowed_drivers, entry, arg)
    } else {
        0
    }
}

pub fn spawn_from_file(path: &str, name: &str, cell_id: CellId, allowed_drivers: alloc::vec::Vec<usize>) -> core::result::Result<usize, ViError> {
    use crate::loader::{ElfLoader, ElfParser};
    use api::fs::{OpenMode, SeekFrom};
    
    // 1. Open File
    let mut file = {
         let fs_lock = crate::fs::VIFS1.lock();
         let fs = fs_lock.as_ref().ok_or(ViError::NotFound)?;
         fs.open(path, OpenMode::Read).map_err(|_| ViError::NotFound)?
    };

    // 2. Read entire content
    log::info!("Spawn: seeking end of {}", path);
    let size = file.seek(SeekFrom::End(0)).map_err(|e| { log::error!("Seek End failed: {:?}", e); ViError::IO })?;
    log::info!("Spawn: file size: {}", size);
    
    file.seek(SeekFrom::Start(0)).map_err(|e| { log::error!("Seek Start failed: {:?}", e); ViError::IO })?;
    
    let mut data = alloc::vec![0u8; size as usize];
    log::info!("Spawn: reading {} bytes", size);
    
    let mut offset = 0;
    while offset < size as usize {
        let chunk = &mut data[offset..];
        let n = file.read(chunk).map_err(|e| { log::error!("Read failed at offset {}: {:?}", offset, e); ViError::IO })?;
        if n == 0 {
            break; // EOF
        }
        offset += n;
    }

    if offset != size as usize {
        log::error!("Spawn: Short read. Expected {}, got {}", size, offset);
        return Err(ViError::IO);
    }
    
    // 3. Load ELF
    log::info!("Spawn: parsing elf");
    let loader = ElfLoader;
    let header = loader.parse_header(&data)?;
    
    // 4. Load Segments
    {
         let mut frame_guard = crate::memory::frame::FRAME_ALLOCATOR.lock();
         let frame_allocator = frame_guard.as_mut().ok_or(ViError::OutOfMemory)?;
         loader.load_segments(&data, frame_allocator)?;
    }
    
    // 5. Spawn Task
    let tid = spawn(name, cell_id, allowed_drivers);
    if tid == 0 { return Err(ViError::Unknown); }
    


    // 6. Update Task Context
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(task) = sched.tasks.get_mut(&tid) {
            // Setup Trap Frame for return to User Mode
            // We DO NOT set context.ra to user entry, because that runs in S-mode!
            // Instead we return to __trap_exit which restores context and uses sret.
            
            task.trap_frame.sepc = header.entry;
            task.trap_frame.sstatus = 0x20; // SPIE=1, SPP=0 (User)
            
            // Allocate Kernel Stack
            let kstack = match crate::task::stack::Stack::new_kernel(STACK_PAGES) {
                Ok(s) => s,
                Err(_) => return Err(ViError::OutOfMemory),
            };

            // Allocate User Stack
            let ustack = match crate::task::stack::Stack::new_user(STACK_PAGES) {
                Ok(s) => s,
                Err(_) => return Err(ViError::OutOfMemory),
            };

            // Use Stacks
            let kstack_top = kstack.top;
            let user_stack_top = ustack.top;

            task.kernel_stack = Some(kstack);
            task.user_stack = Some(ustack);

            // 3. Setup TrapFrame on KERNEL Stack
            // Top of Kernel Stack - TrapFrame
            let tf_ptr = kstack_top - TRAP_FRAME_SIZE;

            // Set User SP in TrapFrame
            task.trap_frame.regs[2] = user_stack_top;

            // CRITICAL: Set User Mode Status in TrapFrame!
            // SPP=0 (User Mode)
            // SPIE=1 (Interrupts enabled after sret)
            // FS=Dirty (11) => 0x6000 (Bits 13/14)
            // SUM=0 (User doesn't use SUM)
            // Total: 0x6020
            task.trap_frame.sstatus = 0x6020;

            // Copy TrapFrame to Kernel Stack
            unsafe {
                let tf_dest = &mut *(tf_ptr as *mut crate::hal::arch::ViTrapFrame);
                *tf_dest = task.trap_frame;
            }
            
            // 4. Point Context to Kernel Stack
            task.context.sp = tf_ptr;
            task.context.ra = __trap_exit as usize;

            // Enable SUM (Bit 18) so Kernel can access User Pointers (e.g. Syscalls)
            // SPP=1 (S-mode ret), SPIE=1, SUM=1
            task.context.sstatus = 0x42120; // SUM=1, FS=1 (Initial), SPP=1, SPIE=1

            info!("Spawned ELF task '{}' (ID {}) at entry 0x{:X}", name, tid, header.entry);
        }
    }
    Ok(tid)
}

pub fn current_task_id() -> usize {
    if let Some(sched) = SCHEDULER.lock().as_ref() {
        sched.current_task_id.unwrap_or(0) 
    } else {
        0
    }
}

pub fn has_ready_tasks() -> bool {
    if let Some(sched) = SCHEDULER.lock().as_ref() {
        sched.has_ready_tasks()
    } else {
        false
    }
}

// Helper to resolve path relative to CWD
fn resolve_path(cwd: &str, path: &str) -> alloc::string::String {
    if path.starts_with('/') {
        alloc::string::String::from(path)
    } else {
        // Simple path joining
        let mut p = alloc::string::String::from(cwd);
        if !p.ends_with('/') {
            p.push('/');
        }
        p.push_str(path);
        // TODO: Handle ".." and "." canonicalization
        p
    }
}

// --- File System Syscall Handlers ---

#[allow(clippy::result_unit_err)]
pub fn file_open(path: &str) -> core::result::Result<usize, ()> {
    // 1. Resolve path
    let full_path = if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(task) = sched.current_task_mut() {
            resolve_path(&task.cwd, path)
        } else {
             // Should not happen
             String::from(path)
        }
    } else {
        return Err(());
    };

    // 2. Open file via VIFS
    // We loop to acquire FS lock to avoid deadlock with scheduler lock? 
    // No, here we don't hold scheduler lock while calling FS.
    
    // Check if VIFS1 is initialized
    use crate::fs::VIFS1;
    let file = {
        let fs_lock = VIFS1.lock();
        if let Some(fs) = fs_lock.as_ref() {
            fs.open(&full_path, api::fs::OpenMode::Read).map_err(|_| ())?
        } else {
            return Err(());
        }
    };

    // 3. Store in Task
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(task) = sched.current_task_mut() {
            let fd = task.open_files.keys().max().map(|k| k + 1).unwrap_or(3); // Start FD at 3 (0,1,2 reserved)
            task.open_files.insert(fd, crate::task::tcb::FileHandle(file));
            return Ok(fd);
        }
    }
    
    // Task terminated concurrently?
    Err(())
}

pub fn file_read(fd: usize, buf: &mut [u8]) -> usize {
    if fd == 0 {
        // Stdin (Keyboard)
        if buf.is_empty() { return 0; }
        
        loop {
            // Poll console
            let mut cons = crate::task::drivers::console_drv::CONSOLE.lock();
            cons.poll();
            let b = cons.read_byte();
            if let Some(byte) = b {
                buf[0] = byte;
                // Echo back to stdout
                 crate::task::print_user_log(core::str::from_utf8(&[byte]).unwrap_or("?"));
                return 1;
            }
            drop(cons);
            // Yield if no input
            yield_cpu();
        }
    }
    
    // File Read
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(task) = sched.current_task_mut() {
            if let Some(handle) = task.open_files.get_mut(&fd) {
                // handle.0 is Box<dyn ViFile>
                return handle.0.read(buf).unwrap_or(0);
            }
        }
    }
    0
}

pub fn file_write(fd: usize, buf: &[u8]) -> usize {
     if fd == 1 || fd == 2 {
         // Stdout/Stderr
         if let Ok(s) = core::str::from_utf8(buf) {
             crate::task::print_user_log(s);
             return buf.len();
         }
         return 0;
     }
     0
}

pub fn file_close(fd: usize) {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(task) = sched.current_task_mut() {
             task.open_files.remove(&fd);
        }
    }
}

pub fn file_readdir(fd: usize, buf: &mut [u8]) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(task) = sched.current_task_mut() {
            if let Some(handle) = task.open_files.get_mut(&fd) {
                 match handle.0.read_dir() {
                     Ok(Some(entry)) => {
                         // Serialize DirEntry to buf
                         // Entry size is 64 + 1 + 8 + padding = 73+ ? sizeof(DirEntry)
                         // types::DirEntry is repr(C).
                         // We copy bytes directly.
                         let ptr = &entry as *const _ as *const u8;
                         let size = core::mem::size_of::<types::DirEntry>();
                         if buf.len() < size { return Err(()); }
                         
                         unsafe {
                             core::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), size);
                         }
                         return Ok(size);
                     },
                     Ok(None) => return Ok(0), // EOF
                     Err(_) => return Err(()),
                 }
            }
        }
    }
    Err(())
}

pub fn file_fstat(_fd: usize, _stat_ptr: usize) -> core::result::Result<usize, ()> {
    Err(())
}

pub fn file_chdir(_path: &str) -> core::result::Result<usize, ()> {
     // TODO: Implement chdir
     Ok(0)
}

pub fn file_getcwd(_buf: &mut [u8]) -> core::result::Result<usize, ()> {
    Err(())
}
use log::warn;
use crate::task::tcb::{TaskState, LeaseAttributes};

pub fn ipc_lend(_lender_id: usize, target_id: usize, ptr: VAddr, len: usize, flags: u32) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(target_task) = sched.tasks.get_mut(&target_id) {
            let lease_id = target_task.add_lease(ptr, len, LeaseAttributes(flags));
            return Ok(lease_id);
        }
    }
    Err(())
}

pub fn ipc_send(caller_id: usize, target_id: usize, msg_ptr: VAddr, msg_len: usize) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if !sched.tasks.contains_key(&target_id) {
            warn!("IPC: Target Task {} not found!", target_id);
            return Err(());
        }

        let target_ready = if let Some(target) = sched.tasks.get(&target_id) {
             match target.state {
                 TaskState::Recv { mask: _, buf_ptr, buf_len } => Some((buf_ptr, buf_len)),
                 _ => None
             }
        } else { None };

        if let Some((dest_ptr, dest_len)) = target_ready {
            let app_src = msg_ptr as *const u8;
            let app_dst = dest_ptr as *mut u8;
            let copy_len = core::cmp::min(msg_len, dest_len);
            unsafe { core::ptr::copy_nonoverlapping(app_src, app_dst, copy_len); }
            
            if let Some(target) = sched.tasks.get_mut(&target_id) {
                target.state = TaskState::Ready;
                target.current_caller = Some(caller_id); 
                sched.ready_queue.push_back(target_id);
            }
            
            if let Some(caller) = sched.tasks.get_mut(&caller_id) {
                 caller.state = TaskState::Sending { target: target_id, msg_ptr, msg_len }; 
            }
            return Ok(0);
        } else {
             if let Some(caller) = sched.tasks.get_mut(&caller_id) {
                 caller.state = TaskState::Sending { target: target_id, msg_ptr, msg_len };
             }
             return Ok(1);
        }
    }
    Err(())
}

pub fn ipc_recv(caller_id: usize, mask: usize, buf_ptr: VAddr, buf_len: usize) -> core::result::Result<usize, ()> {
     if let Some(sched) = SCHEDULER.lock().as_mut() {
         let mut found_sender = None;
         for (tid, task) in sched.tasks.iter() {
             if let TaskState::Sending { target, msg_ptr, msg_len } = task.state {
                 if target == caller_id {
                     found_sender = Some((*tid, msg_ptr, msg_len));
                     break;
                 }
             }
         }
         
         if let Some((sender_id, src_ptr, src_len)) = found_sender {
            let app_src = src_ptr as *const u8;
            let app_dst = buf_ptr as *mut u8;
            let copy_len = core::cmp::min(src_len, buf_len);
            unsafe { core::ptr::copy_nonoverlapping(app_src, app_dst, copy_len); }
            
            if let Some(caller) = sched.tasks.get_mut(&caller_id) {
                caller.current_caller = Some(sender_id);
            }
            return Ok(sender_id);
         } else {
             if let Some(caller) = sched.tasks.get_mut(&caller_id) {
                 caller.state = TaskState::Recv { mask, buf_ptr, buf_len };
             }
             return Ok(0);
         }
     }
     Err(())
}

pub fn ipc_try_recv(caller_id: usize, _mask: usize, buf_ptr: VAddr, buf_len: usize) -> core::result::Result<usize, ()> {
     if let Some(sched) = SCHEDULER.lock().as_mut() {
         let mut found_sender = None;
         for (tid, task) in sched.tasks.iter() {
             if let TaskState::Sending { target, msg_ptr, msg_len } = task.state {
                 if target == caller_id {
                     found_sender = Some((*tid, msg_ptr, msg_len));
                     break;
                 }
             }
         }
         
         if let Some((sender_id, src_ptr, src_len)) = found_sender {
            let app_src = src_ptr as *const u8;
            let app_dst = buf_ptr as *mut u8;
            let copy_len = core::cmp::min(src_len, buf_len);
            unsafe { core::ptr::copy_nonoverlapping(app_src, app_dst, copy_len); }
            
            if let Some(caller) = sched.tasks.get_mut(&caller_id) {
                caller.current_caller = Some(sender_id);
            }
            return Ok(sender_id);
         } else {
             return Ok(0);
         }
     }
     Err(())
}

pub fn ipc_reply(caller_id: usize, result: usize) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        let target_id = sched.tasks.get(&caller_id).and_then(|t| t.current_caller);
        if let Some(tid) = target_id {
            if let Some(t) = sched.tasks.get_mut(&tid) {
                t.state = TaskState::Ready;
                t.reply_value = Some(result);
                sched.ready_queue.push_back(tid);
            }
            if let Some(task) = sched.tasks.get_mut(&caller_id) {
                task.current_caller = None;
            }
            return Ok(0);
        }
    }
    Err(())
}

pub fn ipc_borrow_read(caller_id: usize, lease_id: usize, offset: usize, dst_ptr: VAddr, len: usize) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_ref() {
        if let Some(task) = sched.tasks.get(&caller_id) {
            if let Some(lease) = task.get_lease(lease_id) {
                if !lease.attributes.contains(LeaseAttributes::READ) { return Err(()); }
                if offset + len > lease.len { return Err(()); }
                unsafe { core::ptr::copy_nonoverlapping((lease.ptr + offset) as *const u8, dst_ptr as *mut u8, len); }
                return Ok(len);
            }
        }
    }
    Err(())
}

pub fn ipc_borrow_write(caller_id: usize, lease_id: usize, offset: usize, src_ptr: VAddr, len: usize) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_ref() {
        if let Some(task) = sched.tasks.get(&caller_id) {
            if let Some(lease) = task.get_lease(lease_id) {
                if !lease.attributes.contains(LeaseAttributes::WRITE) { return Err(()); }
                if offset + len > lease.len { return Err(()); }
                unsafe { core::ptr::copy_nonoverlapping(src_ptr as *const u8, (lease.ptr + offset) as *mut u8, len); }
                return Ok(len);
            }
        }
    }
    Err(())
}

pub fn ipc_grant(caller_id: usize, target_id: usize, ptr: VAddr, len: usize, flags: u32) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(target) = sched.tasks.get_mut(&target_id) {
            let gid = target.add_grant(ptr, len, flags, caller_id);
            return Ok(gid);
        }
    }
    Err(())
}

pub fn ipc_map(caller_id: usize, grant_id: usize) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_ref() {
        if let Some(task) = sched.tasks.get(&caller_id) {
            if let Some(grant) = task.get_grant(grant_id) {
                return Ok(grant.ptr);
            }
        }
    }
    Err(())
}

/// Get scheduler statistics
pub fn scheduler_stats() -> (usize, usize) {
    if let Some(sched) = SCHEDULER.lock().as_ref() {
        (sched.tasks.len(), sched.ready_queue.len())
    } else {
        (0, 0)
    }
}

pub fn futex_wait(caller_id: usize, addr: VAddr, val: u32) -> core::result::Result<usize, ()> {
    // Check condition
    unsafe {
        let current_val = *(addr as *const u32);
        if current_val != val {
            return Err(()); // EAGAIN
        }
    }

    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(task) = sched.tasks.get_mut(&caller_id) {
            task.state = TaskState::FutexWait { addr };
            return Ok(0);
        }
    }
    Err(())
}

pub fn futex_wake(_caller_id: usize, addr: VAddr, count: usize) -> core::result::Result<usize, ()> {
    let mut woken = 0;
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        let mut to_wake = alloc::vec::Vec::new();
        
        // Scan for waiting tasks
        for (tid, task) in sched.tasks.iter() {
             // Skip self? Futex wake usually doesn't wake self (self is running).
             if let TaskState::FutexWait { addr: wa_addr } = task.state {
                 if wa_addr == addr {
                     to_wake.push(*tid);
                     if to_wake.len() >= count { break; }
                 }
             }
        }
        
        woken = to_wake.len();

        // Wake them up
        for tid in to_wake {
            if let Some(task) = sched.tasks.get_mut(&tid) {
                task.state = TaskState::Ready;
                sched.ready_queue.push_back(tid);
            }
        }
    }
    Ok(woken)
}

pub fn print_user_log(msg: &str) {
    // If msg ends with newline, trim it because info! adds one.
    // Actually, userprintln! sends newline.
    // We want "USER: " prefix.
    info!("USER: {}", msg.trim_end());
}

/// Spawns a synthetic task for testing User Mode without filesystem
pub fn spawn_synthetic(name: &str, cell_id: CellId, entry: VAddr) -> core::result::Result<usize, ViError> {
    use hal::paging::PAGE_SIZE;
    
    // 1. Spawn Task (Allocates stack, etc.)
    let tid = spawn(name, cell_id, alloc::vec::Vec::new());
    if tid == 0 { return Err(ViError::Unknown); }
    
    // 2. Map Code Page at 'entry'
    {
        let mut frame_guard = crate::memory::frame::FRAME_ALLOCATOR.lock();
        let allocator = frame_guard.as_mut().ok_or(ViError::OutOfMemory)?;
        let frame = allocator.allocate_frame().ok_or(ViError::OutOfMemory)?;
        
        // Write code to frame (Physical access)
        // Code: ecall (0x00000073) + loop (j .)
        // Write code to frame (Physical access)
        unsafe {
            let base = frame as *mut u8;
            
            // 1. lui a0, 0x1      => a0 = 0x1000 (Page Base)
            *(base as *mut u32) = 0x00001537;
            
            // 2. addi a0, a0, 32  => a0 = 0x1020 (String Address)
            *(base.add(4) as *mut u32) = 0x02050513;
            
            // 3. li a1, 21        => a1 = 21 (Length)
            *(base.add(8) as *mut u32) = 0x01500593;
            
            // 4. li a7, 11        => a7 = 11 (Syscall::Log)
            *(base.add(12) as *mut u32) = 0x00b00893;
            
            // 5. ecall
            *(base.add(16) as *mut u32) = 0x00000073;
            
            // 6. j .              => Loop forever
            *(base.add(20) as *mut u32) = 0x0000006F;
            
            // Data: "Hello from Userspace!" at offset 32
            let msg = b"Hello from Userspace!";
            core::ptr::copy_nonoverlapping(msg.as_ptr(), base.add(32), msg.len());
        }
        
        // Permissions: VALID | READ | EXECUTE | USER
        // Note: Generic PageFlags bits might not match RISC-V perfectly if not verified, 
        // but we verified they DO match in hal implementation.
        // Or we use hal::PageFlags directly.
        use crate::memory::paging::Flags;
        // 1=V, 2=R, 8=X, 16=U ? No.
        // Check lib.rs: V=1, R=2, W=4, X=8, U=16
        // We want V, R, X, U. 1|2|8|16 = 27 (0x1B).
        
        let flags = Flags::from_bits(Flags::VALID | Flags::READ | Flags::WRITE | Flags::EXECUTE | Flags::USER | Flags::ACCESSED | Flags::DIRTY);
        
        crate::memory::paging::map_page(allocator, entry, frame, flags).map_err(|_| ViError::OutOfMemory)?;
    }
    
    // 3. Update Task Context (Copied from spawn_from_file)
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(task) = sched.tasks.get_mut(&tid) {
            task.trap_frame.sepc = entry;
            task.trap_frame.sstatus = 0x20; // User Mode (SPIE=1, SPP=0)
            
            // Allocate Kernel Stack
            let kstack = match crate::task::stack::Stack::new_kernel(STACK_PAGES) {
                Ok(s) => s,
                Err(_) => return Err(ViError::OutOfMemory),
            };

            // Allocate User Stack
            let ustack = match crate::task::stack::Stack::new_user(STACK_PAGES) {
                Ok(s) => s,
                Err(_) => return Err(ViError::OutOfMemory),
            };

            // Use Stacks
            let kstack_top = kstack.top;
            let user_stack_top = ustack.top;

            task.kernel_stack = Some(kstack);
            task.user_stack = Some(ustack);

            let tf_ptr = kstack_top - TRAP_FRAME_SIZE;
            task.trap_frame.regs[2] = user_stack_top; // User SP

            unsafe {
                let tf_dest = &mut *(tf_ptr as *mut crate::hal::arch::ViTrapFrame);
                *tf_dest = task.trap_frame;
            }
            
            task.context.sp = tf_ptr;
            task.context.ra = __trap_exit as usize;
            task.context.sstatus = 0x40120; // SUM=1

            info!("Spawned Synthetic task '{}' (ID {}) at entry 0x{:X}", name, tid, entry);
        }
    }
    
    Ok(tid)
}

