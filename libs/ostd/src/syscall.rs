#![allow(unsafe_code)]

use api::syscall::{ViSpawnArgs, ViSyscall};
use core::arch::asm;

#[derive(Debug, Copy, Clone)]
pub enum SyscallResult {
    Ok(usize),
    Err(SyscallError),
}

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

#[inline(always)]
unsafe fn syscall(id: ViSyscall, a0: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let mut ret: isize;
    asm!(
        "ecall",
        inlateout("a0") a0 => ret,
        in("a1") a1,
        in("a2") a2,
        in("a3") a3,
        in("a7") (id as usize),
        options(nostack, preserves_flags)
    );
    ret
}

pub fn sys_log(msg: &str) -> SyscallResult {
    unsafe {
        syscall(ViSyscall::Log, msg.as_ptr() as usize, msg.len(), 0, 0);
        SyscallResult::Ok(0)
    }
}

pub fn sys_yield() {
    unsafe {
        syscall(ViSyscall::Yield, 0, 0, 0, 0);
    }
}

pub fn sys_exit(code: usize) -> ! {
    unsafe {
        syscall(ViSyscall::Exit, code, 0, 0, 0);
    }
    loop {
        sys_yield();
    }
}

pub fn sys_exec(path: &str) -> SyscallResult {
    unsafe {
        let ret = syscall(ViSyscall::Exec, path.as_ptr() as usize, path.len(), 0, 0);
        if ret != -1 {
            SyscallResult::Ok(ret as usize)
        } else {
            SyscallResult::Err(SyscallError::Unknown)
        }
    }
}

pub fn sys_spawn(entry: usize, arg: usize) -> SyscallResult {
    unsafe {
        let ret = syscall(ViSyscall::Spawn, entry, arg, 0, 0);
        if ret > 0 {
            SyscallResult::Ok(ret as usize)
        } else {
            SyscallResult::Err(SyscallError::Unknown)
        }
    }
}

pub fn sys_spawn_from_mem(data: &[u8], name: &str, args: &str) -> SyscallResult {
    unsafe {
        let spawn_args = ViSpawnArgs {
            buffer_addr: data.as_ptr() as usize,
            buffer_size: data.len(),
            name_ptr: name.as_ptr() as usize,
            name_len: name.len(),
            args_ptr: args.as_ptr() as usize,
            args_len: args.len(),
        };

        let ret = syscall(
            ViSyscall::SpawnFromMem,
            &spawn_args as *const _ as usize,
            0,
            0,
            0,
        );
        if ret > 0 {
            SyscallResult::Ok(ret as usize)
        } else {
            SyscallResult::Err(SyscallError::Unknown)
        }
    }
}

/// Spawn a cell by loading its ELF from a VFS path (e.g. `/bin/shell`).
///
/// The kernel reads the ELF from disk or the bootstrap table, parses it,
/// and spawns a new task.  Returns the new cell's task ID on success.
///
/// # Errors
/// Returns `SyscallError::Unknown` if the path is not found or the ELF is invalid.
pub fn sys_spawn_from_path(path: &str) -> SyscallResult {
    // SAFETY: path is a valid UTF-8 str; kernel copies it out before returning.
    unsafe {
        let ret = syscall(
            ViSyscall::SpawnFromPath,
            path.as_ptr() as usize,
            path.len(),
            0,
            0,
        );
        if ret > 0 {
            SyscallResult::Ok(ret as usize)
        } else {
            SyscallResult::Err(SyscallError::Unknown)
        }
    }
}

pub fn sys_wait(pid: usize) -> SyscallResult {
    unsafe {
        let ret = syscall(ViSyscall::Wait, pid, 0, 0, 0);
        if ret >= 0 {
            SyscallResult::Ok(ret as usize)
        } else {
            SyscallResult::Err(SyscallError::Unknown)
        }
    }
}

pub fn sys_shm_alloc(size: usize) -> SyscallResult {
    unsafe {
        let ret = syscall(ViSyscall::ShmAlloc, size, 0, 0, 0);
        if ret > 0 {
            SyscallResult::Ok(ret as usize)
        } else {
            SyscallResult::Err(SyscallError::Unknown)
        }
    }
}

pub fn sys_shm_map(handle: usize, target_pid: usize) -> SyscallResult {
    unsafe {
        let ret = syscall(ViSyscall::ShmMap, handle, target_pid, 0, 0);
        if ret != 0 {
            SyscallResult::Ok(ret as usize)
        } else {
            SyscallResult::Err(SyscallError::Unknown)
        }
    }
}

pub fn sys_open(path: &str) -> Result<usize, SyscallError> {
    unsafe {
        let ret = syscall(ViSyscall::Open, path.as_ptr() as usize, path.len(), 0, 0);
        if ret >= 0 {
            Ok(ret as usize)
        } else {
            Err(SyscallError::FileNotFound)
        }
    }
}

pub fn sys_close(fd: usize) {
    unsafe {
        syscall(ViSyscall::Close, fd, 0, 0, 0);
    }
}

pub fn sys_read(fd: usize, buffer: &mut [u8]) -> Result<usize, SyscallError> {
    unsafe {
        let ret = syscall(
            ViSyscall::Read,
            fd,
            buffer.as_mut_ptr() as usize,
            buffer.len(),
            0,
        );
        if ret >= 0 {
            Ok(ret as usize)
        } else {
            Err(SyscallError::PermissionDenied)
        }
    }
}

pub fn sys_write(fd: usize, buffer: &[u8]) -> Result<usize, SyscallError> {
    unsafe {
        let ret = syscall(
            ViSyscall::Write,
            fd,
            buffer.as_ptr() as usize,
            buffer.len(),
            0,
        );
        if ret >= 0 {
            Ok(ret as usize)
        } else {
            Err(SyscallError::PermissionDenied)
        }
    }
}

// IPC Wrappers
pub fn sys_send(target: usize, msg: &[u8]) -> SyscallResult {
    unsafe {
        let ret = syscall(ViSyscall::Send, target, msg.as_ptr() as usize, msg.len(), 0);
        SyscallResult::Ok(ret as usize)
    }
}

pub fn sys_read_dir(fd: usize, buffer: &mut [u8]) -> Result<usize, SyscallError> {
    unsafe {
        let ret = syscall(
            ViSyscall::ReadDir,
            fd,
            buffer.as_mut_ptr() as usize,
            buffer.len(),
            0,
        );
        if ret >= 0 {
            Ok(ret as usize)
        } else {
            Err(SyscallError::Unknown)
        }
    }
}

pub fn sys_recv(mask: usize, buf: &mut [u8]) -> SyscallResult {
    unsafe {
        let ret = syscall(
            ViSyscall::Recv,
            mask,
            buf.as_mut_ptr() as usize,
            buf.len(),
            0,
        );
        SyscallResult::Ok(ret as usize)
    }
}

pub fn sys_set_timer(ticks: usize) -> SyscallResult {
    unsafe {
        syscall(ViSyscall::SetTimer, ticks, 0, 0, 0);
        SyscallResult::Ok(0)
    }
}

pub fn sys_grant(_target: usize, _ptr: usize, _len: usize, _flags: usize) -> SyscallResult {
    // Assume Grant mapped to ID 12
    SyscallResult::Err(SyscallError::Unknown)
}

pub fn sys_get_procs(buffer: &mut [api::syscall::ProcessInfo]) -> Result<usize, SyscallError> {
    unsafe {
        let ret = syscall(ViSyscall::GetProcs, buffer.as_mut_ptr() as usize, buffer.len(), 0, 0);
        if ret >= 0 {
            Ok(ret as usize)
        } else {
            Err(SyscallError::Unknown)
        }
    }
}
