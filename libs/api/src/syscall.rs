// SPDX-License-Identifier: MPL-2.0

/// System Call Identifiers (The Contract)
///
/// These IDs must match between Kernel and User (libs/ostd).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum ViSyscall {
    // === IPC (0-9) ===
    Send = 0,
    Recv = 1,
    Call = 2,
    Reply = 3,
    
    // === Process Management (10-49) ===
    Exit = 60,  // Linux compat usually, but we define our own space
    Spawn = 5,
    Exec = 6, // Deprecated/Legacy
    SpawnFromMem = 10, // New Spawn from Memory (Struct based)
    Wait = 8, // Wait for task
    Yield = 104, // Linux sched_yield is 24, but we use 104 in current code
    SetTimer = 35, // Added SetTimer
    ShmAlloc = 20, // Allocate Shared Memory
    ShmMap = 21,   // Map Shared Memory to Task
    
    // === Logging (50-59) ===
    Log = 11,   // Current implementation uses 11
    
    // === Filesystem (100-199) ===
    Open = 101,
    Read = 102,
    Close = 103,
    ReadDir = 105,
    Write = 109,
    
    // === Unknown ===
    Unknown = 9999,
}

impl From<usize> for ViSyscall {
    fn from(id: usize) -> Self {
        match id {
            0 => ViSyscall::Send,
            1 => ViSyscall::Recv,
            2 => ViSyscall::Call,
            3 => ViSyscall::Reply,
            60 => ViSyscall::Exit,
            5 => ViSyscall::Spawn,
            6 => ViSyscall::Exec,
            10 => ViSyscall::SpawnFromMem,
            8 => ViSyscall::Wait,
            104 => ViSyscall::Yield,
            35 => ViSyscall::SetTimer,
            20 => ViSyscall::ShmAlloc,
            21 => ViSyscall::ShmMap,
            11 => ViSyscall::Log,
            101 => ViSyscall::Open,
            102 => ViSyscall::Read,
            103 => ViSyscall::Close,
            105 => ViSyscall::ReadDir,
            109 => ViSyscall::Write,
            _ => ViSyscall::Unknown,
        }
    }
}

/// Arguments for SpawnFromMem syscall.
/// Using repr(C) for ABI stability.
#[repr(C)]
pub struct ViSpawnArgs {
    /// Address of buffer containing ELF.
    pub buffer_addr: usize,
    /// Size of buffer.
    pub buffer_size: usize,
    /// Pointer to name string (utf8).
    pub name_ptr: usize,
    /// Length of name string.
    pub name_len: usize,
    /// Pointer to arguments string (utf8, space separated or null separated).
    pub args_ptr: usize,
    /// Length of arguments string.
    pub args_len: usize,
}
