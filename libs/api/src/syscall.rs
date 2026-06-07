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
    /// Force-terminate another task by TID.  Non-blocking: caller continues.
    /// Requires `SpawnCap` on caller; rejected on system cells (block_io/network_cap).
    /// ABI: a0 = target_tid → 0 on success, usize::MAX on error.
    ForceExit = 61,
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
    /// Serialize all allocated physical frames to the snapshot sector range.
    /// Triggers a warm-boot-capable snapshot.  Returns frame count on success.
    Snapshot = 420,
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
    /// Register the caller as a watcher of `watched_tid`. When that task exits OR
    /// faults, the kernel delivers a death notification to the caller's pending
    /// `Recv` (`sender_id` = the dead tid, payload = the exit/fault reason), so a
    /// supervisor can wait-any across many children with a single recv loop.
    /// Requires `SpawnCap`. ABI: a0 = watched_tid → 0 on success, usize::MAX on error.
    NotifyOnExit = 204,
    /// Register `tid` as the current provider of well-known `service_id`. SpawnCap-gated:
    /// the supervisor owns the service namespace (prevents a cell from hijacking, say, the
    /// VFS endpoint). On respawn the supervisor re-registers the new tid, so clients
    /// reconnect transparently. ABI: a0 = service_id (u16), a1 = tid → 0 / usize::MAX.
    RegisterService = 205,
    /// Resolve a well-known `service_id` to its current provider tid. Open to all cells so
    /// any client can reconnect after a service restart. ABI: a0 = service_id (u16) →
    /// provider tid (> 0), or 0 if no live provider is currently registered.
    LookupService = 206,
    /// Liveness heartbeat: the caller asserts it is alive and will call again within
    /// `interval_ticks` 10 ms scheduler ticks. If it misses that deadline the kernel
    /// terminates it as HUNG — catching a silent hang (deadlock / stuck loop) that the
    /// CPU-monopoly watchdog cannot see — so the supervisor restarts it. `interval_ticks
    /// = 0` disables the heartbeat. Open to all cells. ABI: a0 = interval_ticks → 0.
    Heartbeat = 207,

    // === Zero-Copy Grant (Storage 2.0, Phase 01) ===
    /// Allocate a kernel-managed Grant region of up to 16 pages (64 KB).
    /// ABI: a0 = size (rounded up to pages, max 65536) → base_paddr (> 0) on success, 0 on OOM.
    /// Requires GrantCap (allowlist bit 39).
    GrantAlloc = 208,
    /// Share Grant access with `target_task`.
    /// ABI: a0 = grant_id, a1 = target_task_id, a2 = GrantPerm (0=RO, 1=WO, 2=RW) → 0 on success.
    /// Requires GrantCap (bit 39).
    GrantShare = 209,
    /// Return the raw pointer to a Grant region the caller has access to.
    /// ABI: a0 = grant_id → ptr (usize, same as grant_id for identity-mapped SAS) on success.
    /// Returns usize::MAX on permission denied or not found.
    GrantSlice = 210,
    /// Release a Grant region: unmaps its pages and frees its frames.
    /// ABI: a0 = grant_id → 0 on success.
    GrantFree = 211,
    /// Start an async sector read into a Grant buffer (poll-based stub; Phase 04 for true async).
    /// ABI: a0 = sector, a1 = grant_id → async_id (1 = immediately complete) on success.
    /// Requires BlockIoCap (bit 36) — same gate as raw block-I/O opcodes 500/501.
    BlkReadAsync = 212,
    /// Request exclusive MMIO ownership for a peripheral Driver Cell.
    /// ABI: a0 = base (physical MMIO address), a1 = len → 0 on success.
    /// Requires the corresponding manifest flag (GPIO or UART) to be set.
    /// The kernel checks the Resource Registry allowlist; rejects unknown ranges.
    RequestMmio = 213,
    /// Fill caller buffer with VirtIO-RNG entropy (true hardware randomness).
    /// Required for TLS key generation — mtime-seeded PRNG is cryptographically broken.
    /// ABI: a0 = buf_ptr, a1 = len (max 64 per call, one VirtIO descriptor) → bytes written.
    /// Returns 0 if no VirtIO-RNG device is present.
    GetRandom = 214,
    /// Allocate a persistent, pre-pinned Grant buffer for the cell's lifetime.
    /// Unlike GrantAlloc, the buffer is not freed until GrantUnregister or cell exit.
    /// ABI: a0 = size → reg_id (physical base, > 0) on success, 0 on OOM.
    /// Requires GrantCap (allowlist bit 39).
    GrantRegister = 215,
    /// Release a registered buffer allocated via GrantRegister.
    /// ABI: a0 = reg_id → 0 on success.
    /// Requires GrantCap (allowlist bit 39).
    GrantUnregister = 216,

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

/// Compact bitset of permitted syscalls, stored as a `u64`.
///
/// Used to build the allowlist embedded in the ELF `__ViCell_syscalls` section
/// via [`declare_syscalls!`].  All methods are `const` so the value can be
/// computed at compile time.
///
/// ```ignore
/// const MY_ALLOWED: SyscallSet = SyscallSet::EMPTY
///     .with(ViSyscall::Send)
///     .with(ViSyscall::Recv)
///     .with(ViSyscall::Log);
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SyscallSet(pub u64);

impl SyscallSet {
    /// Empty set — all syscalls denied (not useful on its own; combine with `with`).
    pub const EMPTY: Self = Self(0);
    /// Permit-all sentinel — the default for cells without a `__ViCell_syscalls` section.
    pub const ALL: Self = Self(u64::MAX);

    /// Return a new `SyscallSet` with `syscall` added.
    ///
    /// If `syscall` has no allowlist bit (always-permitted syscalls like `Yield`
    /// or `Exit`) the set is returned unchanged.
    pub const fn with(self, syscall: ViSyscall) -> Self {
        match syscall.allowlist_bit() {
            Some(bit) => Self(self.0 | (1u64 << bit)),
            None => self,
        }
    }

    /// Raw bit-mask value to embed in `__ViCell_syscalls`.
    pub const fn bits(self) -> u64 { self.0 }

    /// Returns `true` if `syscall` is permitted by this set.
    ///
    /// Always-permitted syscalls (`Yield`, `Exit`, …) return `true` regardless
    /// of the stored bits.
    pub const fn permits(self, syscall: ViSyscall) -> bool {
        match syscall.allowlist_bit() {
            Some(bit) => (self.0 >> bit) & 1 == 1,
            None => true,
        }
    }
}

/// Embed a syscall allowlist into the current Cell's ELF binary.
///
/// Places a `u64` bitset into the `__ViCell_syscalls` ELF section.  The kernel
/// reads it at spawn time and enforces it on every syscall — any call whose bit
/// is not set is rejected with `PermissionDenied` before the handler runs.
///
/// Cells that do **not** call this macro default to permit-all (backwards
/// compatible).  Cells that call it are restricted to exactly the listed
/// syscalls plus always-permitted ones (`Yield`, `Exit`, `ForceExit`, …).
///
/// # Usage
/// ```ignore
/// api::declare_syscalls![Send, Recv, TryRecv, Log, Heartbeat, LookupService];
/// ```
#[macro_export]
macro_rules! declare_syscalls {
    ($($syscall:ident),* $(,)?) => {
        #[used]
        #[link_section = "__ViCell_syscalls"]
        pub static VICELL_SYSCALLS: u64 = const {
            let mut mask: u64 = 0u64;
            $(
                if let Some(bit) = $crate::syscall::ViSyscall::$syscall.allowlist_bit() {
                    mask |= 1u64 << bit;
                }
            )*
            mask
        };
    };
}

impl ViSyscall {
    /// Stable bit index (0-38) for the per-Cell syscall allowlist stored in
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
            // Snapshot: privileged warm-boot operation; reuses HotSwap bit (SpawnCap required).
            Self::Snapshot      => Some(32),
            // LookupService is an open syscall (any client resolves a service endpoint).
            Self::LookupService => Some(37),
            // Heartbeat is an open syscall (any cell asserts its own liveness).
            Self::Heartbeat     => Some(38),
            // GrantCap (bit 39): cells that need zero-copy large-file I/O via Grant API.
            Self::GrantAlloc | Self::GrantShare | Self::GrantSlice | Self::GrantFree
            | Self::GrantRegister | Self::GrantUnregister => Some(39),
            // BlkReadAsync reuses BlockIoCap (bit 36) — same authority as raw block I/O.
            Self::BlkReadAsync  => Some(36),
            // RequestMmio: gated by GPIO (bit 40) or UART (bit 41) manifest flags.
            // The kernel re-checks the manifest and allowlist at dispatch; here we
            // assign bit 40 as the generic "peripheral MMIO" allowlist bit.
            Self::RequestMmio   => Some(40),
            // GetRandom: any cell that needs entropy (TLS, crypto) declares this.
            // Bit 41 was previously allocated as a UART MMIO alias; repurposing for entropy
            // is safe because UART MMIO is enforced at dispatch by resource-registry check,
            // not by this bit alone.
            Self::GetRandom     => Some(41),
            // Yield, Exit, and ForceExit are always permitted — a Cell must be able
            // to yield the CPU, exit cleanly, and force-terminate unresponsive tasks
            // regardless of its allowlist.  SpawnCap is the authority gate for ForceExit.
            // NotifyOnExit and RegisterService are privileged (SpawnCap-gated, like
            // ForceExit), so they are always permitted past the allowlist — SpawnCap is
            // the authority gate enforced at dispatch.
            Self::Yield | Self::Exit | Self::ForceExit | Self::NotifyOnExit
            | Self::RegisterService | Self::Unknown => None,
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
            61 => ViSyscall::ForceExit,
            5 => ViSyscall::Spawn,
            6 => ViSyscall::Exec,
            10 => ViSyscall::SpawnFromMem,
            12 => ViSyscall::SpawnFromPath,
            13 => ViSyscall::OpenCap,
            14 => ViSyscall::ReadCap,
            15 => ViSyscall::CloseCap,
            16  => ViSyscall::SpawnPinned,
            420 => ViSyscall::Snapshot,
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
            204 => ViSyscall::NotifyOnExit,
            205 => ViSyscall::RegisterService,
            206 => ViSyscall::LookupService,
            207 => ViSyscall::Heartbeat,
            208 => ViSyscall::GrantAlloc,
            209 => ViSyscall::GrantShare,
            210 => ViSyscall::GrantSlice,
            211 => ViSyscall::GrantFree,
            212 => ViSyscall::BlkReadAsync,
            213 => ViSyscall::RequestMmio,
            214 => ViSyscall::GetRandom,
            215 => ViSyscall::GrantRegister,
            216 => ViSyscall::GrantUnregister,
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

/// Well-known service IDs for the kernel Service Registry (`RegisterService` /
/// `LookupService`). Stable ABI — values must not change once shipped. `0` is reserved
/// to mean "no provider" (the value `LookupService` returns when nothing is registered).
///
/// A client resolves the live provider tid of a service by `LookupService(service::VFS)`
/// instead of hard-coding a tid, so it transparently reconnects when the supervisor
/// respawns that service under a new tid.
pub mod service {
    /// Virtual filesystem service (`/bin/vfs`).
    pub const VFS: u16 = 1;
    /// Network stack service (`/bin/net`).
    pub const NET: u16 = 2;
    /// Input/event service (`/bin/input`).
    pub const INPUT: u16 = 3;
    /// Configuration store service (`/bin/config`).
    pub const CONFIG: u16 = 4;
    /// Display compositor service (`/bin/compositor`).
    pub const COMPOSITOR: u16 = 5;
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
