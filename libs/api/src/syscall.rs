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
    /// Non-blocking receive: returns 0 immediately when no message is queued
    /// (vs `Recv`, which parks the task until a message arrives).
    TryRecv = 7,

    // === Process Management (10-49) ===
    Exit = 60, // Linux compat usually, but we define our own space
    Spawn = 5,
    Exec = 6,          // Deprecated/Legacy
    SpawnFromMem = 10, // New Spawn from Memory (Struct based)
    /// Spawn a cell by reading its ELF from a VFS path.
    /// ABI: a0 = path_ptr, a1 = path_len; returns cell id or error code.
    SpawnFromPath = 12,
    /// Open a file by path, returning a kernel capability ID.
    /// ABI: a0 = path_ptr, a1 = path_len → CapId (> 0) on success.
    OpenCap = 13,
    /// Read bytes from a cap-backed file.
    /// ABI: a0 = cap_id, a1 = buf_ptr, a2 = buf_len → bytes_read.
    ReadCap = 14,
    /// Revoke a capability (close).
    /// ABI: a0 = cap_id → 0 on success.
    CloseCap = 15,
    /// Spawn a cell pinned to a specific hardware core.
    /// ABI: a0 = path_ptr, a1 = path_len, a2 = priority: u8, a3 = core_id: usize.
    /// On single-core systems core_id must be 0; any other value returns NotSupported.
    SpawnPinned = 16,
    Wait = 8,          // Wait for task
    Yield = 104,       // Linux sched_yield is 24, but we use 104 in current code
    SetTimer = 35,     // Added SetTimer
    ShmAlloc = 20,     // Allocate Shared Memory
    ShmMap = 21,       // Map Shared Memory to Task

    // === Logging (50-59) ===
    Log = 11, // Current implementation uses 11

    // === Filesystem (100-199) ===
    Open = 101,
    Read = 102,
    Close = 103,
    ReadDir = 105,
    Write = 109,
    Seek = 106,
    FileOp = 107, // Rename, Remove

    // === Time/System (120-129) ===
    GetTime = 120,

    // === GPU / Display (Phase 16) ===
    /// Flush a pixel rectangle from a cell-provided buffer to the GPU framebuffer.
    /// ABI: a0 = data_ptr, a1 = data_len, a2 = x, a3 = y (w+h embedded in len)
    GpuFlush = 300,

    // === Network ===
    /// Transmit one Ethernet frame through the kernel VirtIO NIC.
    /// ABI: a0 = frame_ptr, a1 = frame_len → 1 on success, 0 on failure.
    NetTx = 310,
    /// Receive one pending Ethernet frame from the kernel VirtIO NIC.
    /// ABI: a0 = buf_ptr, a1 = buf_len → bytes written (0 if none ready).
    NetRx = 311,

    // === Advanced IPC (Phase 20) ===
    /// Recv with a deadline in monotonic ticks.
    /// ABI: a0 = mask, a1 = buf_ptr, a2 = buf_len, a3 = timeout_ticks → sender_id or ViError::Timeout.
    RecvTimeout = 201,
    /// Gather-send: one IPC message assembled from up to 8 non-contiguous buffers.
    /// ABI: a0 = target, a1 = iovec_ptr ([ptr:usize, len:usize] × count), a2 = iovec_count.
    SendGather = 202,
    /// Scatter-receive: one IPC message written into up to 8 non-contiguous buffers.
    /// ABI: a0 = mask, a1 = iovec_ptr, a2 = iovec_count → sender_id.
    RecvScatter = 203,

    // === Hot-swap (Phase 20) ===
    /// Live-replace a running Cell without message loss.
    /// ABI: a0 = cell_id, a1 = path_ptr, a2 = path_len → new_task_id or error.
    HotSwap = 400,
    /// Stash a Cell's serialized state in the kernel under `key`, so a
    /// replacement instance can recover it across a hot-swap / respawn.
    /// ABI: a0 = key, a1 = buf_ptr, a2 = buf_len → bytes stored.
    StateStash = 410,
    /// Restore previously stashed state for `key` into the caller's buffer.
    /// ABI: a0 = key, a1 = buf_ptr, a2 = buf_len → bytes written (0 if none).
    StateRestore = 411,

    // === Unknown ===
    Unknown = 9999,

    // === Process Info ===
    GetProcs = 30,
}

impl ViSyscall {
    /// Stable bit index (0-36) for the per-Cell syscall allowlist stored in
    /// `Task::syscall_allowlist`.
    ///
    /// Bit indices are independent of raw opcode values so they remain stable
    /// even if opcodes are renumbered.  Returns `None` for syscalls that are
    /// always permitted (Yield, Exit) — filtering them would prevent a Cell
    /// from ever yielding or cleanly shutting down.
    ///
    /// Bit 36 is reserved for raw block-I/O syscalls (opcodes 500-503) which
    /// bypass `ViSyscall::from()` and must be checked separately at dispatch.
    pub const fn allowlist_bit(self) -> Option<u8> {
        match self {
            Self::Send          => Some(0),
            Self::Recv          => Some(1),
            Self::TryRecv       => Some(2),
            Self::Reply         => Some(3),
            Self::Call          => Some(4),
            Self::Spawn         => Some(5),
            Self::SpawnFromMem  => Some(6),
            Self::SpawnFromPath => Some(7),
            Self::SpawnPinned   => Some(8),
            Self::Wait          => Some(9),
            Self::Log           => Some(10),
            Self::SetTimer      => Some(11),
            Self::ShmAlloc      => Some(12),
            Self::ShmMap        => Some(13),
            Self::GetProcs      => Some(14),
            Self::OpenCap       => Some(15),
            Self::ReadCap       => Some(16),
            Self::CloseCap      => Some(17),
            Self::Open          => Some(18),
            Self::Read          => Some(19),
            Self::Write         => Some(20),
            Self::Close         => Some(21),
            Self::ReadDir       => Some(22),
            Self::Seek          => Some(23),
            Self::FileOp        => Some(24),
            Self::GetTime       => Some(25),
            Self::GpuFlush      => Some(26),
            Self::NetTx         => Some(27),
            Self::NetRx         => Some(28),
            Self::RecvTimeout   => Some(29),
            Self::SendGather    => Some(30),
            Self::RecvScatter   => Some(31),
            Self::HotSwap       => Some(32),
            Self::StateStash    => Some(33),
            Self::StateRestore  => Some(34),
            Self::Exec          => Some(35),
            // Yield and Exit are always permitted — a Cell must be able to
            // yield the CPU and exit cleanly regardless of its allowlist.
            Self::Yield | Self::Exit | Self::Unknown => None,
        }
    }
}

impl From<usize> for ViSyscall {
    fn from(id: usize) -> Self {
        match id {
            0 => ViSyscall::Send,
            1 => ViSyscall::Recv,
            7 => ViSyscall::TryRecv,
            2 => ViSyscall::Call,
            3 => ViSyscall::Reply,
            60 => ViSyscall::Exit,
            5 => ViSyscall::Spawn,
            6 => ViSyscall::Exec,
            10 => ViSyscall::SpawnFromMem,
            12 => ViSyscall::SpawnFromPath,
            13 => ViSyscall::OpenCap,
            14 => ViSyscall::ReadCap,
            15 => ViSyscall::CloseCap,
            16 => ViSyscall::SpawnPinned,
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
            106 => ViSyscall::Seek,
            107 => ViSyscall::FileOp,
            120 => ViSyscall::GetTime,
            30  => ViSyscall::GetProcs,
            201 => ViSyscall::RecvTimeout,
            202 => ViSyscall::SendGather,
            203 => ViSyscall::RecvScatter,
            300 => ViSyscall::GpuFlush,
            310 => ViSyscall::NetTx,
            311 => ViSyscall::NetRx,
            400 => ViSyscall::HotSwap,
            410 => ViSyscall::StateStash,
            411 => ViSyscall::StateRestore,
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

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessInfo {
    pub id: usize,
    pub state: usize, // 0=Ready, 1=Running, 2=Waiting, 3=Terminated
    pub name: [u8; 32],
}
