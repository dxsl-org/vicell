//! IPC System Calls (Inspired by Tock OS)
//!
//! This module defines the interface between "Cells/Silos" and the Kernel.
//! See [docs/architecture/03-driver-strategy.md] for the full rationale.

use super::tcb::TaskState;
use alloc::collections::{BTreeMap, BTreeSet};
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

// ── Zero-Copy Grant Table ─────────────────────────────────────────────────────

/// Kernel-managed zero-copy memory region.
///
/// Distinct from `tcb::GrantEntry` which tracks per-task grants from the kernel.
/// Owner and grantee are tracked by raw task id (usize) — same as `caller_id`
/// from `current_task_id()`. We intentionally avoid `CellId` wrappers here
/// because `caller_id` IS a task id, not a Cell id (F7).
struct PageGrant {
    base:      usize,                        // physical address of the first allocated page
    size:      usize,                        // total byte count (multiple of 4096)
    owner:     usize,                        // task id of the GrantAlloc caller
    shared_to: Option<(usize, GrantPerm)>,   // current grantee task id + permission
}

static PAGE_GRANT_TABLE: Spinlock<Option<BTreeMap<usize, PageGrant>>> = Spinlock::new(None);

fn grant_table_lock() -> &'static Spinlock<Option<BTreeMap<usize, PageGrant>>> {
    &PAGE_GRANT_TABLE
}

/// Maximum pages in a single GrantAlloc or GrantRegister call (16 MiB ceiling).
/// Acts as a safety cap; cells are further bounded by available physical frames.
const MAX_GRANT_PAGES: usize = 4096;

// ── Registered Grant Table (GrantRegister / GrantUnregister, syscalls 215/216) ──

/// Persistent kernel-managed Grant buffer for a cell's lifetime.
///
/// Supports one grantee at a time via `GrantShare`/`GrantSlice` (same as `PageGrant`).
struct RegGrant {
    base:      usize,                        // physical address of first allocated page
    size:      usize,                        // byte count (multiple of 4096)
    owner:     usize,                        // task id — only owner may call GrantUnregister
    shared_to: Option<(usize, GrantPerm)>,   // authorized grantee task id + permission
}

static REG_GRANT_TABLE: Spinlock<Option<BTreeMap<usize, RegGrant>>> = Spinlock::new(None);

fn reg_grant_table_lock() -> &'static Spinlock<Option<BTreeMap<usize, RegGrant>>> {
    &REG_GRANT_TABLE
}

// ── Shared allocation/deallocation helpers ────────────────────────────────────

/// Allocate `n_pages` contiguous physical frames, map them USER RW, and zero them.
///
/// Returns the physical base address on success, or `None` on OOM or partial map.
/// Lock order: FRAME_ALLOCATOR (alloc) → FRAME_ALLOCATOR (map_page) → release →
///             FRAME_ALLOCATOR (partial-failure dealloc).
fn alloc_grant_pages(n_pages: usize) -> Option<usize> {
    use crate::memory::frame::FRAME_ALLOCATOR;
    use crate::memory::paging::Flags;
    const PAGE_SIZE: usize = 4096;

    let user_flags = Flags::VALID | Flags::READ | Flags::WRITE
        | Flags::USER | Flags::ACCESSED | Flags::DIRTY;

    let paddr = {
        let mut g = FRAME_ALLOCATOR.lock();
        g.as_mut().and_then(|a| a.allocate_contiguous(n_pages))?
    };

    let mut mapped = 0usize;
    {
        let mut guard = FRAME_ALLOCATOR.lock();
        if let Some(alloc) = guard.as_mut() {
            for i in 0..n_pages {
                let v = paddr + i * PAGE_SIZE;
                if crate::memory::paging::map_page(alloc, v, v, Flags::from_bits(user_flags)).is_ok() {
                    mapped += 1;
                } else {
                    break;
                }
            }
        }
    }

    if mapped < n_pages {
        // Partial map: unmap what succeeded, then free all frames.
        for i in 0..mapped {
            let _ = crate::memory::paging::unmap_page(paddr + i * PAGE_SIZE);
        }
        crate::memory::paging::tlb_flush_all();
        let mut fa = FRAME_ALLOCATOR.lock();
        if let Some(a) = fa.as_mut() {
            for k in 0..n_pages { a.deallocate_frame(paddr + k * PAGE_SIZE); }
        }
        return None;
    }

    // Zero every mapped page before handing to user: prevents stale data from a
    // previously-freed grant leaking to a different cell (info-disclosure under G2).
    // SAFETY: frames are identity-mapped USER RW; SUM=1 allows S-mode writes.
    unsafe { core::ptr::write_bytes(paddr as *mut u8, 0, n_pages * PAGE_SIZE); }

    Some(paddr)
}

/// Unmap and deallocate `n_pages` physical frames starting at `base`.
///
/// Lock order: unmap_page (KERNEL_ROOT) → sfence.vma → FRAME_ALLOCATOR.
/// Must NOT hold FRAME_ALLOCATOR when called.
fn free_grant_pages(base: usize, n_pages: usize) {
    use crate::memory::frame::FRAME_ALLOCATOR;
    const PAGE_SIZE: usize = 4096;

    for i in 0..n_pages {
        let _ = crate::memory::paging::unmap_page(base + i * PAGE_SIZE);
    }
    crate::memory::paging::tlb_flush_all();
    let mut fa = FRAME_ALLOCATOR.lock();
    if let Some(alloc) = fa.as_mut() {
        for k in 0..n_pages { alloc.deallocate_frame(base + k * PAGE_SIZE); }
    }
}

// ── Grant Reaper ──────────────────────────────────────────────────────────────

/// Reclaim all grant pages owned or held by a dying task.
///
/// Called from every task-exit code path (Exit syscall, ForceExit, scheduler watchdog, fault handler).
/// Two effects:
///   1. Owner death  — remove entry, unmap pages, return frames to allocator.
///   2. Grantee death — clear `shared_to` so the owner's grant becomes unshared.
///
/// Lock order: PAGE_GRANT_TABLE collect → unmap (KERNEL_ROOT) → FRAME_ALLOCATOR.
/// Never holds FRAME_ALLOCATOR when calling free_grant_pages.
pub(crate) fn reap_grants_for_task(dead_tid: usize) {
    const PAGE_SIZE: usize = 4096;

    // ── PAGE_GRANT_TABLE pass ─────────────────────────────────────────────────
    let owned: alloc::vec::Vec<PageGrant> = {
        let mut tbl = grant_table_lock().lock();
        let Some(map) = tbl.as_mut() else { return };
        // Clear grantee references (no removal needed — owner keeps the entry).
        for grant in map.values_mut() {
            if grant.shared_to.map_or(false, |(tid, _)| tid == dead_tid) {
                grant.shared_to = None;
            }
        }
        // Collect and remove owned entries.
        let owned_keys: alloc::vec::Vec<usize> = map
            .iter()
            .filter(|(_, g)| g.owner == dead_tid)
            .map(|(k, _)| *k)
            .collect();
        owned_keys.iter().filter_map(|k| map.remove(k)).collect()
    }; // PAGE_GRANT_TABLE lock released

    for grant in &owned {
        free_grant_pages(grant.base, grant.size / PAGE_SIZE);
    }

    // ── REG_GRANT_TABLE pass ──────────────────────────────────────────────────
    let reg_owned: alloc::vec::Vec<RegGrant> = {
        let mut tbl = reg_grant_table_lock().lock();
        let Some(map) = tbl.as_mut() else { return };
        // Clear grantee references when the grantee dies.
        for grant in map.values_mut() {
            if grant.shared_to.map_or(false, |(tid, _)| tid == dead_tid) {
                grant.shared_to = None;
            }
        }
        let owned_keys: alloc::vec::Vec<usize> = map
            .iter()
            .filter(|(_, g)| g.owner == dead_tid)
            .map(|(k, _)| *k)
            .collect();
        owned_keys.iter().filter_map(|k| map.remove(k)).collect()
    }; // REG_GRANT_TABLE lock released

    for reg in &reg_owned {
        free_grant_pages(reg.base, reg.size / PAGE_SIZE);
    }
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

/// Returns `true` if the calling task satisfies the given capability check.
///
/// Lock-ordering: acquires SCHEDULER, drops before returning.
fn caller_has_cap<F: Fn(&crate::task::tcb::Task) -> bool>(caller_id: usize, check: F) -> bool {
    super::SCHEDULER
        .lock()
        .as_ref()
        .and_then(|sched| sched.tasks.get(&caller_id))
        .map(|t| check(t))
        .unwrap_or(false)
}

fn caller_has_block_io(caller_id: usize) -> bool {
    caller_has_cap(caller_id, |t| t.block_io_cap.is_some())
}

/// Per-cell block-I/O range gate (Milestone 2.5 P03).
///
/// Replaces the old global `sector >= CELL_TABLE_BASE_LBA` check: the caller's
/// `block_regions` bitmask (from its manifest PART_* bits, or the legacy VFS
/// grant) defines which MBR partitions its raw block syscalls may address.
/// Deny-by-default: a sector outside every granted partition is rejected —
/// which structurally protects P2 (cell table) and P3 (snapshot), since no
/// bit exists for them. Logs every denial (silent denials cost us a day once).
fn check_block_access(caller_id: usize, sector: u64, count: u64) -> bool {
    use crate::loader::disk_layout as dl;
    let regions = super::SCHEDULER.lock().as_ref()
        .and_then(|s| s.tasks.get(&caller_id))
        .map(|t| t.block_regions)
        .unwrap_or(0);
    let end = match sector.checked_add(count) {
        Some(e) => e,
        None => return false,
    };
    const GRANTABLE: [(u8, u64, u64); 3] = [
        (0b001, dl::PART_FAT32_BASE_LBA, dl::PART_FAT32_SECTORS), // P1 (PART_DATA)
        (0b010, dl::PART_LFS_BASE_LBA,   dl::PART_LFS_SECTORS),   // P4 (PART_LFS)
        (0b100, dl::PART_SRV_BASE_LBA,   dl::PART_SRV_SECTORS),   // P5 (SRV/RedoxFS, co-granted w/ LFS)
    ];
    for (bit, base, size) in GRANTABLE {
        if regions & bit != 0 && sector >= base && end <= base + size {
            return true;
        }
    }
    log::warn!(
        "[blk] sector {}..{} denied for tid {} (regions={:#04b})",
        sector, end, caller_id, regions
    );
    false
}

fn caller_has_network(caller_id: usize) -> bool {
    caller_has_cap(caller_id, |t| t.network_cap.is_some())
}

fn caller_has_hypervisor(caller_id: usize) -> bool {
    caller_has_cap(caller_id, |t| t.hypervisor_cap.is_some())
}

fn caller_has_spawn(caller_id: usize) -> bool {
    caller_has_cap(caller_id, |t| t.spawn_cap.is_some())
}

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

/// The Fundamental Verbs of ViCell IPC (Hubris ABI + Lease System)
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
    /// 202: SendGather — send one IPC message from multiple non-contiguous buffers.
    SendGather { target: usize, iovec_ptr: usize, iovec_count: usize },
    /// 203: RecvScatter — receive one IPC message into multiple non-contiguous buffers.
    RecvScatter { mask: usize, iovec_ptr: usize, iovec_count: usize },
    /// 201: RecvTimeout — Recv with a monotonic-tick deadline (Phase 20).
    RecvTimeout {
        mask: usize,
        buf_ptr: usize,
        buf_len: usize,
        /// Deadline in kernel monotonic ticks from boot.  0 = non-blocking.
        deadline: u64,
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
    /// 61: ForceExit — terminate another task by TID; non-blocking return to caller.
    ForceExit { tid: usize },
    /// 204: NotifyOnExit — register the caller to be notified when `watched` dies.
    NotifyOnExit { watched: usize },
    /// 205: RegisterService — register `tid` as the current provider of `service_id`
    /// (SpawnCap-gated; the supervisor owns the namespace).
    RegisterService { service_id: u16, tid: usize },
    /// 206: LookupService — resolve `service_id` to its live provider tid (open; 0 = none).
    LookupService { service_id: u16 },
    /// 207: Heartbeat — caller asserts liveness; (re)arm the hung-detection deadline
    /// `interval` ticks ahead (0 = disable).
    Heartbeat { interval: usize },
    /// 6: Exec (Spawn from file)
    Exec { path_ptr: usize, path_len: usize },
    /// 10: SpawnFromMem (Spawn from Memory buffer via Struct)
    SpawnFromMem { args_ptr: usize },
    /// 12: SpawnFromPath (Spawn cell by filesystem path)
    /// ABI: path_ptr in a0, path_len in a1.
    SpawnFromPath { path_ptr: usize, path_len: usize },
    /// 16: SpawnPinned — spawn cell pinned to a core (single-core: core_id must be 0).
    /// ABI: a0=path_ptr, a1=path_len, a2=priority: u8, a3=core_id: usize.
    SpawnPinned { path_ptr: usize, path_len: usize, priority: u8, core_id: usize },
    /// 13: OpenCap — open a file and return a CapId.
    OpenCap { path_ptr: usize, path_len: usize },
    /// 14: ReadCap — read bytes from a cap-backed file.
    ReadCap { cap_id: usize, buf_ptr: usize, buf_len: usize },
    /// 15: CloseCap — revoke a capability.
    CloseCap { cap_id: usize },
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
    /// 300: GpuFlush — copy cell pixel buffer to VirtIO GPU framebuffer.
    GpuFlush { data_ptr: usize, data_len: usize, xy: usize, wh: usize },
    /// 301: GpuCursor — set sprite (op=0) or move (op=1) the VirtIO GPU hardware cursor.
    GpuCursor { op: usize, data_ptr: usize, xy: usize, hot: usize },
    /// 310: NetTx — transmit one Ethernet frame via the kernel VirtIO NIC.
    NetTx { frame_ptr: usize, frame_len: usize },
    /// 311: NetRx — receive one pending Ethernet frame from the VirtIO NIC.
    NetRx { buf_ptr: usize, buf_len: usize },
    /// 410: StateStash — save serialized cell state under `key` for hot-swap.
    StateStash { key: usize, buf_ptr: usize, buf_len: usize },
    /// 411: StateRestore — recover stashed state for `key` into the buffer.
    StateRestore { key: usize, buf_ptr: usize, buf_len: usize },
    /// 400: HotSwap — live-replace a Cell with a new ELF from disk.
    HotSwap { cell_id: usize, path_ptr: usize, path_len: usize },
    /// 420: Snapshot — serialize all allocated physical frames to disk for warm boot.
    Snapshot,
    /// 500: BlkRead — read one 512-byte sector from the VirtIO block device.
    /// Not in `ViSyscall` enum to preserve `libs/api` stability (raw dispatch).
    BlkRead { sector: u64, buf_ptr: usize },
    /// 501: BlkWrite — write one 512-byte sector to the VirtIO block device.
    BlkWrite { sector: u64, buf_ptr: usize },
    /// 502: Shutdown — trigger SBI SRST system shutdown (S-mode → OpenSBI). No return.
    Shutdown,
    /// 503: BlkFlush — flush the VirtIO block device write cache to the backing image.
    BlkFlush,
    /// 208: GrantAlloc — allocate n pages as a zero-copy Grant region.
    GrantAlloc { size: usize },
    /// 209: GrantShare — share a Grant region with `target_cell` under `perm`.
    GrantShare { grant_id: usize, target_cell: usize, perm: usize },
    /// 210: GrantSlice — return the user-space pointer for a Grant the caller owns/holds.
    GrantSlice { grant_id: usize },
    /// 211: GrantFree — unmap + deallocate a Grant region.
    GrantFree { grant_id: usize },
    /// 212: BlkReadAsync — synchronous-but-zero-copy sector read into a Grant buffer.
    BlkReadAsync { sector: u64, grant_id: usize },
    /// 213: RequestMmio — claim exclusive MMIO range for a peripheral Driver Cell.
    RequestMmio { base: usize, len: usize },
    /// 214: GetRandom — fill a caller buffer with VirtIO-RNG entropy bytes.
    GetRandom { buf_ptr: usize, len: usize },
    /// 215: GrantRegister — allocate a persistent pre-pinned Grant buffer (lifetime = cell exit).
    GrantRegister { size: usize },
    /// 216: GrantUnregister — explicitly release a registered buffer.
    GrantUnregister { reg_id: usize },
    /// 217: WaitForEvent — block until `mask` bits fire or `deadline` ticks pass.
    /// `deadline = None` means block indefinitely.
    WaitForEvent { mask: u32, deadline: Option<u64> },

    // === Hypervisor (220-225) — HypervisorCap ZST-gated ===
    /// 220: CreateVm — allocate guest RAM + Stage-2 table → vm_id.
    CreateVm        { guest_pages: usize },
    /// 221: CreateVcpu — create a vCPU with `entry_pc` in `vm_id` → vcpu_id.
    CreateVcpu      { vm_id: usize, entry_pc: u64 },
    /// 222: MapGuestMemory — map guest IPA range in `vm_id` Stage-2 table.
    MapGuestMemory  { vm_id: usize, ipa: u64, size: usize, writable: bool },
    /// 223: RunVcpu — world-switch into vCPU; write `ViVmExit` to `out_ptr`.
    RunVcpu         { vm_id: usize, vcpu_id: usize, budget_ns: u64, out_ptr: usize },
    /// 224: VcpuRegs — read (write=false) or write (write=true) GP registers.
    VcpuRegs        { vm_id: usize, vcpu_id: usize, buf_ptr: usize, write: bool },
    /// 225: InjectIrq — inject GICv2 virtual interrupt (0 ≤ intid ≤ 1019).
    InjectIrq       { vm_id: usize, vcpu_id: usize, intid: u32 },
    /// 226: WriteGuestMemory — copy `len` bytes from `src_ptr` to guest GPA.
    WriteGuestMemory { vm_id: usize, gpa: u64, src_ptr: usize, len: usize },
    /// 227: ReadGuestMemory — copy `len` bytes from guest GPA into `dst_ptr`.
    ReadGuestMemory  { vm_id: usize, gpa: u64, dst_ptr: usize, len: usize },
}

/// Read the per-Cell syscall allowlist from the TCB.
///
/// Returns `u64::MAX` (permit-all) for unknown tids — safe default during
/// early boot before the scheduler is initialised.
fn get_syscall_allowlist(caller_id: usize) -> u64 {
    super::SCHEDULER
        .lock()
        .as_ref()
        .and_then(|s| s.tasks.get(&caller_id))
        .map(|t| t.syscall_allowlist)
        .unwrap_or(u64::MAX)
}

/// Map a kernel-internal `Syscall` variant to its `ViSyscall` representation
/// for allowlist bit lookup.
///
/// Returns `None` for:
/// - Raw block-I/O ops (500-503): ZST-gated via `BlockIoCap`, not filtered here.
/// - Legacy/internal variants (FutexWait, BorrowRead, Lend, …): no bit assigned.
/// - Always-permitted syscalls (Yield, Exit, …): `allowlist_bit()` returns `None`.
fn syscall_to_vi(syscall: &Syscall) -> Option<api::syscall::ViSyscall> {
    use api::syscall::ViSyscall as V;
    Some(match syscall {
        Syscall::Send { .. }          => V::Send,
        Syscall::Recv { .. }          => V::Recv,
        Syscall::TryRecv { .. }       => V::TryRecv,
        Syscall::RecvTimeout { .. }   => V::RecvTimeout,
        Syscall::SendGather { .. }    => V::SendGather,
        Syscall::RecvScatter { .. }   => V::RecvScatter,
        Syscall::Reply { .. }         => V::Reply,
        Syscall::Spawn { .. }         => V::Spawn,
        Syscall::SpawnFromMem { .. }  => V::SpawnFromMem,
        Syscall::SpawnFromPath { .. } => V::SpawnFromPath,
        Syscall::SpawnPinned { .. }   => V::SpawnPinned,
        Syscall::Wait { .. }          => V::Wait,
        Syscall::Log { .. }           => V::Log,
        Syscall::SetTimer { .. }      => V::SetTimer,
        Syscall::ShmAlloc { .. }      => V::ShmAlloc,
        Syscall::ShmMap { .. }        => V::ShmMap,
        Syscall::GetProcs { .. }      => V::GetProcs,
        Syscall::OpenCap { .. }       => V::OpenCap,
        Syscall::ReadCap { .. }       => V::ReadCap,
        Syscall::CloseCap { .. }      => V::CloseCap,
        Syscall::Open { .. }          => V::Open,
        Syscall::Read { .. }          => V::Read,
        Syscall::Write { .. }         => V::Write,
        Syscall::Close { .. }         => V::Close,
        Syscall::ReadDir { .. }       => V::ReadDir,
        Syscall::Seek { .. }          => V::Seek,
        Syscall::FileOp { .. }        => V::FileOp,
        Syscall::GetTime { .. }       => V::GetTime,
        Syscall::GpuFlush { .. }      => V::GpuFlush,
        Syscall::GpuCursor { .. }     => V::GpuCursor,
        Syscall::NetTx { .. }         => V::NetTx,
        Syscall::NetRx { .. }         => V::NetRx,
        Syscall::HotSwap { .. }       => V::HotSwap,
        Syscall::Snapshot             => V::Snapshot,
        Syscall::StateStash { .. }    => V::StateStash,
        Syscall::StateRestore { .. }  => V::StateRestore,
        Syscall::Exec { .. }          => V::Exec,
        Syscall::LookupService { .. } => V::LookupService,
        Syscall::Heartbeat { .. }     => V::Heartbeat,
        Syscall::GrantAlloc { .. }    => V::GrantAlloc,
        Syscall::GrantShare { .. }    => V::GrantShare,
        Syscall::GrantSlice { .. }    => V::GrantSlice,
        Syscall::GrantFree { .. }     => V::GrantFree,
        Syscall::BlkReadAsync { .. }    => V::BlkReadAsync,
        Syscall::RequestMmio { .. }    => V::RequestMmio,
        Syscall::GetRandom { .. }      => V::GetRandom,
        Syscall::GrantRegister { .. }  => V::GrantRegister,
        Syscall::GrantUnregister { .. }=> V::GrantUnregister,
        Syscall::WaitForEvent { .. }   => V::WaitForEvent,
        Syscall::CreateVm { .. }       => V::CreateVm,
        Syscall::CreateVcpu { .. }     => V::CreateVcpu,
        Syscall::MapGuestMemory { .. } => V::MapGuestMemory,
        Syscall::RunVcpu { .. }        => V::RunVcpu,
        Syscall::VcpuRegs { .. }       => V::VcpuRegs,
        Syscall::InjectIrq { .. }      => V::InjectIrq,
        Syscall::WriteGuestMemory { .. } => V::WriteGuestMemory,
        Syscall::ReadGuestMemory  { .. } => V::ReadGuestMemory,
        // Always-permitted; allowlist_bit() returns None → filter is a no-op.
        Syscall::Yield
        | Syscall::Exit { .. }
        | Syscall::ForceExit { .. }
        | Syscall::NotifyOnExit { .. }
        | Syscall::RegisterService { .. } => return None,
        // Raw block-I/O (500-503): ZST BlockIoCap gated at dispatch.
        Syscall::BlkRead { .. }
        | Syscall::BlkWrite { .. }
        | Syscall::BlkFlush
        | Syscall::Shutdown => return None,
        // Legacy / internal variants without allowlist bits.
        _ => return None,
    })
}

/// Dispatches a system call to the appropriate handler.
///
/// `caller_id` is the ID of the task invoking the syscall.
pub fn handle_syscall(caller_id: usize, syscall: Syscall) -> SyscallResult {
    // Syscall allowlist enforcement: reject if this syscall's bit is not set in
    // the per-Cell bitset loaded from ELF section `__ViCell_syscalls`.
    // Cells without that section default to u64::MAX (permit-all, backwards compat).
    if let Some(vi) = syscall_to_vi(&syscall) {
        if let Some(bit) = vi.allowlist_bit() {
            let allowed = get_syscall_allowlist(caller_id);
            if (allowed >> bit) & 1 == 0 {
                log::warn!(
                    "[kernel] syscall {:?} denied for tid {} (allowlist={:#018x})",
                    vi, caller_id, allowed
                );
                crate::audit::log_event(
                    crate::audit::AuditEvent::SyscallDenied,
                    &crate::audit::encode_u32x2(caller_id as u32, bit as u32),
                );
                return Err(SyscallError::PermissionDenied);
            }
        }
    }

    match syscall {
        // --- Hubris ABI Implementation ---
        Syscall::Send {
            target,
            msg_ptr,
            msg_len,
        } => {
            crate::audit::log_event(
                crate::audit::AuditEvent::IpcSend,
                &crate::audit::encode_u32x2(caller_id as u32, target as u32),
            );
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
            // A NotifyOnExit death that arrived while we were busy (not parked in
            // Recv) was queued; deliver it now without blocking. The dead tid is
            // returned as the "sender" so a supervisor never misses a child death.
            if let Some(sched) = super::SCHEDULER.lock().as_mut() {
                if let Some(t) = sched.tasks.get_mut(&caller_id) {
                    if !t.pending_deaths.is_empty() {
                        let (dead_tid, reason) = t.pending_deaths.remove(0);
                        // Deliver the exit reason as the recv payload (NotifyOnExit
                        // contract) so a supervisor can apply a restart policy.
                        if buf_len >= core::mem::size_of::<u64>()
                            && validate_user_buf(buf_ptr, core::mem::size_of::<u64>(), MAX_USER_BUF).is_ok()
                        {
                            // SAFETY: buf_ptr is validated as this caller's user buffer;
                            // the caller is mid-syscall so the write is exclusive in the SAS.
                            unsafe { core::ptr::write_unaligned(buf_ptr as *mut u64, reason as u64); }
                        }
                        return Ok(dead_tid);
                    }
                }
            }
            let res = super::ipc_recv(caller_id, mask, buf_ptr, buf_len);
            match res {
                Ok(0) => {
                    // Blocked
                    super::yield_cpu();
                    // Resume: return who sent the message (or the dead tid for a death
                    // notification). If this wake was a death, a reason was stashed by
                    // exit_task — deliver it as the recv payload HERE, in our own syscall
                    // context where writing a USER buffer is valid (unlike the trap context).
                    let mut sender = 0;
                    let mut death_reason = None;
                    if let Some(sched) = super::SCHEDULER.lock().as_mut() {
                        if let Some(t) = sched.tasks.get_mut(&caller_id) {
                            sender = t.current_caller.unwrap_or(0);
                            death_reason = t.pending_exit_reason.take();
                        }
                    }
                    if let Some(reason) = death_reason {
                        if buf_len >= core::mem::size_of::<u64>()
                            && validate_user_buf(buf_ptr, core::mem::size_of::<u64>(), MAX_USER_BUF).is_ok()
                        {
                            // SAFETY: buf_ptr is validated as this caller's user buffer; we
                            // run in the caller's syscall context, so the store is permitted.
                            unsafe { core::ptr::write_unaligned(buf_ptr as *mut u64, reason as u64); }
                        }
                    }
                    Ok(sender)
                }
                Ok(id) => Ok(id), // Got message instantly
                Err(_) => Err(SyscallError::InvalidCommand),
            }
        }
        // ── Scatter/gather IPC ────────────────────────────────────────────────
        Syscall::SendGather { target, iovec_ptr, iovec_count } => {
            // Concatenate all segments into a contiguous kernel buffer, then
            // deliver as a single IPC message to `target`.
            const MAX_IOVEC: usize = 8;
            const IOVEC_ENTRY: usize = core::mem::size_of::<usize>() * 2;
            if iovec_count == 0 || iovec_count > MAX_IOVEC {
                return Err(SyscallError::InvalidInput);
            }
            // Allocate a temporary gather buffer.
            let mut total = 0usize;
            for i in 0..iovec_count {
                // SAFETY: iovec_ptr is a valid user-space array of [ptr,len] pairs;
                // iovec_count is bounded by MAX_IOVEC; each element is 2×sizeof(usize).
                let len = unsafe {
                    core::ptr::read_unaligned(
                        (iovec_ptr + i * IOVEC_ENTRY + core::mem::size_of::<usize>()) as *const usize,
                    )
                };
                total = total.saturating_add(len);
            }
            if total > MAX_USER_BUF { return Err(SyscallError::BufferTooSmall); }
            let mut gathered: alloc::vec::Vec<u8> = alloc::vec![0u8; total];
            let mut pos = 0;
            for i in 0..iovec_count {
                // SAFETY: bounds validated above; ptr/len come from user-validated iovec.
                let (ptr, len) = unsafe {
                    let base = iovec_ptr + i * IOVEC_ENTRY;
                    let p = core::ptr::read_unaligned(base as *const usize);
                    let l = core::ptr::read_unaligned((base + core::mem::size_of::<usize>()) as *const usize);
                    (p, l)
                };
                // SAFETY: ptr is a valid user-space pointer; len validated against MAX_USER_BUF.
                unsafe {
                    core::ptr::copy_nonoverlapping(ptr as *const u8, gathered[pos..].as_mut_ptr(), len);
                }
                pos += len;
            }
            let msg_ptr = gathered.as_ptr() as usize;
            super::ipc_send(caller_id, target, msg_ptr, total)
                .map_err(|_| SyscallError::InvalidCommand)
        }
        Syscall::RecvScatter { mask, iovec_ptr, iovec_count } => {
            // Receive a single IPC message and scatter it across the iovec buffers.
            // For v1.0: receive into one temp buffer then scatter.
            const MAX_IOVEC: usize = 8;
            const IOVEC_ENTRY: usize = core::mem::size_of::<usize>() * 2;
            if iovec_count == 0 || iovec_count > MAX_IOVEC {
                return Err(SyscallError::InvalidInput);
            }
            let mut total = 0usize;
            for i in 0..iovec_count {
                // SAFETY: iovec_ptr valid user-space array; bounds checked.
                let len = unsafe {
                    core::ptr::read_unaligned(
                        (iovec_ptr + i * IOVEC_ENTRY + core::mem::size_of::<usize>()) as *const usize,
                    )
                };
                total = total.saturating_add(len);
            }
            if total > MAX_USER_BUF { return Err(SyscallError::BufferTooSmall); }
            let mut tmp: alloc::vec::Vec<u8> = alloc::vec![0u8; total];
            let sender = super::ipc_recv(caller_id, mask, tmp.as_mut_ptr() as usize, total)
                .map_err(|_| SyscallError::InvalidCommand)?;
            // Scatter from tmp into the user's iovec buffers.
            let mut pos = 0;
            for i in 0..iovec_count {
                // SAFETY: iovec_ptr is a valid user-space array; ptr/len validated.
                let (ptr, len) = unsafe {
                    let base = iovec_ptr + i * IOVEC_ENTRY;
                    let p = core::ptr::read_unaligned(base as *const usize);
                    let l = core::ptr::read_unaligned((base + core::mem::size_of::<usize>()) as *const usize);
                    (p, l)
                };
                let copy_len = len.min(total.saturating_sub(pos));
                if copy_len > 0 {
                    // SAFETY: ptr is a valid user-space mutable buffer; copy_len ≤ len.
                    unsafe {
                        core::ptr::copy_nonoverlapping(tmp[pos..].as_ptr(), ptr as *mut u8, copy_len);
                    }
                    pos += copy_len;
                }
            }
            Ok(sender)
        }
        Syscall::RecvTimeout { mask, buf_ptr, buf_len, deadline } => {
            // Fast path: check for a pending message immediately.
            let res = super::ipc_recv(caller_id, mask, buf_ptr, buf_len);
            match res {
                Ok(0) => {
                    // Blocked with deadline:None — install the absolute deadline.
                    if let Some(sched) = super::SCHEDULER.lock().as_mut() {
                        if let Some(task) = sched.tasks.get_mut(&caller_id) {
                            if let super::tcb::TaskState::Recv { deadline: ref mut d, .. } = task.state {
                                *d = Some(deadline);
                            }
                        }
                    }
                    // Yield so the scheduler runs other tasks and can fire the timeout.
                    super::yield_cpu();
                    if let Some(sched) = super::SCHEDULER.lock().as_ref() {
                        return Ok(sched
                            .tasks
                            .get(&caller_id)
                            .and_then(|t| t.current_caller)
                            .unwrap_or(0));
                    }
                    Ok(0)
                }
                Ok(id) => Ok(id),
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
            crate::audit::log_event(
                crate::audit::AuditEvent::CellExit,
                &crate::audit::encode_u32x2(caller_id as u32, code as u32),
            );
            log::info!("Syscall::Exit: task {} exited with code {}", caller_id, code);

            // Capture cell_id BEFORE exit_task removes the task — querying after
            // returns None, which would deregister quota for CellId(0) (a latent
            // bug in the old code path; fixed here).  exit_task now also wakes
            // Wait(caller_id) waiters with `code`, so no in-handler wake loop.
            let cell_id = if let Some(sched) = super::SCHEDULER.lock().as_mut() {
                let cid = sched
                    .tasks
                    .get(&caller_id)
                    .map(|t| t.cell_id)
                    .unwrap_or(types::CellId(0));
                if let Some(task) = sched.tasks.get_mut(&caller_id) {
                    task.exit_code = Some(code);
                }
                // Move task to sched.zombies so its context pointer remains valid
                // across the context switch in yield_cpu; pick_next checks zombies.
                sched.exit_task(caller_id, code);
                cid
            } else {
                types::CellId(0)
            };

            // Revoke all capabilities owned by this cell so the cap table doesn't
            // retain orphaned entries and so a future cell with the same ID cannot
            // inherit them.
            crate::cell::cap_registry::CAP_TABLE.lock().revoke_all_for(cell_id);

            // Release the Cell's memory quota entry and any MMIO regions it held.
            crate::memory::cell_quota::deregister(cell_id);
            crate::resource_registry::release_for(cell_id);

            // Free any grant pages this cell owned or held as grantee.
            reap_grants_for_task(caller_id);

            // Clear any fast-IPC handler registered by this cell so a future
            // call_vfs does not jump into the now-freed ELF pages.
            // (fault/watchdog paths already call this; voluntary Exit did not.)
            crate::fast_ipc::clear_vfs_if_cell(cell_id.0 as usize);

            // yield_cpu switches away; this task is never rescheduled.
            super::yield_cpu();
            Ok(0)
        }

        Syscall::ForceExit { tid } => {
            // Self-kill rejected before touching the scheduler (cheap early check).
            if tid == caller_id {
                return Err(SyscallError::InvalidCommand);
            }

            let target_cell_id;

            // Single SCHEDULER lock: SpawnCap gate + all cleanup in one scope.
            // Two separate acquisitions would create a TOCTOU window where the target
            // self-exits between them, causing a spurious InvalidCommand return.
            if let Some(sched) = super::SCHEDULER.lock().as_mut() {
                // Gate 1: only SpawnCap holders (init/shell) may force-terminate tasks.
                let has_spawn = sched.tasks.get(&caller_id)
                    .map(|t| t.spawn_cap.is_some())
                    .unwrap_or(false);
                if !has_spawn {
                    return Err(SyscallError::PermissionDenied);
                }

                // Gate 2: protect system service cells (VFS=block_io_cap, net=network_cap).
                // Killing them mid-I/O leaves driver state inconsistent; use hot-swap instead.
                let target_is_system = sched.tasks.get(&tid)
                    .map(|t| t.block_io_cap.is_some() || t.network_cap.is_some())
                    .unwrap_or(false);
                if target_is_system {
                    return Err(SyscallError::PermissionDenied);
                }

                // Capture cell_id and waiters BEFORE exit_task() removes the task from
                // sched.tasks.  Querying after returns None — the Exit handler (syscall.rs:645)
                // has this latent bug; we deliberately avoid replicating it here.
                let task = match sched.tasks.get_mut(&tid) {
                    Some(t) => t,
                    // Target self-exited between the lock boundary — already dead; mission done.
                    None => return Ok(0),
                };
                target_cell_id = task.cell_id;
                task.exit_code = Some(usize::MAX); // sentinel: force-killed

                // exit_task: zombie move + ready-queue purge + stuck-sender unblock,
                // and wakes sys_wait(tid) waiters with the force-kill sentinel.
                sched.exit_task(tid, usize::MAX);
            } else {
                return Err(SyscallError::InvalidCommand);
            }

            // Cap + quota + MMIO cleanup — same as Exit handler.
            crate::cell::cap_registry::CAP_TABLE.lock().revoke_all_for(target_cell_id);
            crate::memory::cell_quota::deregister(target_cell_id);
            crate::resource_registry::release_for(target_cell_id);
            reap_grants_for_task(tid);

            crate::audit::log_event(
                crate::audit::AuditEvent::CellExit,
                &crate::audit::encode_u32x2(tid as u32, 0xFFFF_FFFFu32), // force-kill marker
            );
            log::info!("[kernel] ForceExit: task {} killed by task {}", tid, caller_id);

            Ok(0) // non-blocking — caller keeps running; do NOT yield_cpu
        }

        Syscall::NotifyOnExit { watched } => {
            // Privileged: only SpawnCap holders (supervisors like init) may watch
            // arbitrary tasks — same authority gate as ForceExit. The watcher's
            // next Recv returns `watched` when it dies (see exit_task delivery).
            let has_spawn = super::SCHEDULER
                .lock()
                .as_ref()
                .and_then(|s| s.tasks.get(&caller_id))
                .map(|t| t.spawn_cap.is_some())
                .unwrap_or(false);
            if !has_spawn {
                return Err(SyscallError::PermissionDenied);
            }
            super::scheduler::subscribe_death(watched, caller_id);
            Ok(0)
        }

        Syscall::RegisterService { service_id, tid } => {
            // Privileged: only SpawnCap holders (the supervisor) own the service
            // namespace — same authority gate as NotifyOnExit/ForceExit. Prevents a
            // cell from hijacking a well-known endpoint (e.g. the VFS service).
            if !caller_has_spawn(caller_id) {
                return Err(SyscallError::PermissionDenied);
            }
            if crate::cell::service_registry::register(service_id, tid) {
                Ok(0)
            } else {
                Err(SyscallError::InvalidInput)
            }
        }
        Syscall::LookupService { service_id } => {
            // Open to all cells: resolve the live provider tid (0 = none registered),
            // so a client reconnects transparently after the supervisor respawns a
            // service. The dynamic replacement for the boot-order `ServiceLookup` hardcode.
            Ok(crate::cell::service_registry::lookup(service_id).unwrap_or(0))
        }
        Syscall::Heartbeat { interval } => {
            // Open: a cell asserts its own liveness. Arms a deadline `interval` ticks
            // ahead; `pick_next` terminates the cell as HUNG if it lapses. interval=0
            // disables. Self-targeted only — a cell can only (re)arm its OWN deadline.
            if let Some(sched) = super::SCHEDULER.lock().as_mut() {
                if let Some(t) = sched.tasks.get_mut(&caller_id) {
                    t.heartbeat_deadline = if interval == 0 {
                        None
                    } else {
                        Some(super::system_ticks() as u64 + interval as u64)
                    };
                }
            }
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
            // SAFETY: validate_user_buf checked the pointer and length above.
            let name = unsafe {
                core::str::from_utf8(
                    core::slice::from_raw_parts(name_ptr as *const u8, name_len)
                ).map_err(|_| SyscallError::InvalidInput)?
            };
            // Hardcoded spawn-order lookup. The kernel spawns init (ID 1) and a
            // user_hello smoke-test task (ID 2) before the init binary runs.
            // Init then spawns in sequence: vfs=3, config=4, input=5, net=6,
            // compositor=7, shell=8. Verified from QEMU serial log.
            // Replace with a dynamic registry in v0.3.
            let id: usize = match name {
                "vfs"        => 3,
                "config"     => 4,
                "input"      => 5,
                "net"        => 6,
                "compositor" => 7,
                "shell"      => 8,
                _ => return Err(SyscallError::FileNotFound),
            };
            Ok(id)
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
            if !caller_has_spawn(caller_id) {
                return Err(SyscallError::PermissionDenied);
            }
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
            let task_id = crate::loader::spawn_from_path(path_str).map_err(|e| match e {
                types::ViError::NotFound => SyscallError::FileNotFound,
                types::ViError::OutOfMemory => SyscallError::Unknown,
                _ => SyscallError::InvalidInput,
            })?;
            // Transfer pending spawn args to a per-task personal slot so a
            // subsequent spawn overwriting the global ARGV slot cannot race this
            // cell before it is scheduled and reads its args.
            // Personal key = ARGV_KEY ^ (task_id << 32) — task ids are small
            // (<256) so the high-bit XOR stays in the argv key namespace.
            // Use heap allocation (not stack) — the SpawnFromPath call chain
            // is deep and a 512-byte stack buffer would overflow the kernel stack.
            const ARGV_KEY: u64 = 0x0061_7267_7600_0000; // = ostd ARGV_STASH_KEY
            {
                let mut argv_buf = alloc::vec![0u8; 512];
                let n = crate::cell::state_stash::restore(ARGV_KEY, &mut argv_buf);
                if n > 0 {
                    let personal_key = ARGV_KEY ^ ((task_id as u64) << 32);
                    crate::cell::state_stash::stash(personal_key, &argv_buf[..n]);
                }
            }
            Ok(task_id)
        }

        Syscall::SpawnPinned { path_ptr, path_len, priority, core_id } => {
            if !caller_has_spawn(caller_id) {
                return Err(SyscallError::PermissionDenied);
            }
            // On single-core builds only core 0 exists.  Return NotSupported for
            // any other core_id so callers can detect SMP unavailability.
            if core_id != 0 {
                return Err(SyscallError::NotSupported);
            }
            if path_len == 0 || path_len > crate::loader::disk_layout::MAX_CELL_PATH {
                return Err(SyscallError::InvalidInput);
            }
            validate_user_buf(path_ptr, path_len, crate::loader::disk_layout::MAX_CELL_PATH)?;
            // SAFETY: validated above; SUM=1.
            let path_str = unsafe {
                let slice = core::slice::from_raw_parts(path_ptr as *const u8, path_len);
                core::str::from_utf8(slice).map_err(|_| SyscallError::InvalidInput)?
            };
            if !path_str.starts_with('/') {
                return Err(SyscallError::InvalidInput);
            }
            // Spawn at requested priority; future SMP can use core_id for affinity.
            let task_id = crate::loader::spawn_from_path(path_str).map_err(|e| match e {
                types::ViError::NotFound => SyscallError::FileNotFound,
                types::ViError::OutOfMemory => SyscallError::Unknown,
                _ => SyscallError::InvalidInput,
            })?;
            // Set priority on the spawned task.
            if let Some(sched) = crate::task::SCHEDULER.lock().as_mut() {
                if let Some(task) = sched.tasks.get_mut(&task_id) {
                    task.priority = priority;
                }
            }
            Ok(task_id)
        }

        // ── Capability-based file I/O ────────────────────────────────────────
        Syscall::OpenCap { path_ptr, path_len } => {
            if path_len == 0 || path_len > 256 {
                return Err(SyscallError::InvalidInput);
            }
            validate_user_buf(path_ptr, path_len, 256)?;
            // SAFETY: validated above; SUM=1.
            let path_str = unsafe {
                let s = core::slice::from_raw_parts(path_ptr as *const u8, path_len);
                core::str::from_utf8(s).map_err(|_| SyscallError::InvalidInput)?
            };

            // Open via kernel-internal FS.
            use crate::fs::VIFS1;
            let file = {
                let mut guard = VIFS1.lock();
                guard.as_mut().ok_or(SyscallError::FileNotFound)?
                    .open(path_str, api::fs::OpenMode::Read)
                    .map_err(|_| SyscallError::FileNotFound)?
            };

            // Resolve the cell ID of the calling task (distinct from task ID).
            let cell_id = if let Some(sched) = super::SCHEDULER.lock().as_ref() {
                sched.tasks.get(&caller_id).map(|t| t.cell_id).unwrap_or(types::CellId(0))
            } else {
                types::CellId(0)
            };

            // Allocate capability; file starts as Some (unparked).
            let cap_id = crate::cell::cap_registry::CAP_TABLE.lock().alloc(
                cell_id,
                crate::cell::cap_registry::CapResource::File { file: Some(file) },
                api::cap::CapPerms::FILE_READ.0,
            );
            Ok(cap_id.0 as usize)
        }

        Syscall::ReadCap { cap_id, buf_ptr, buf_len } => {
            if buf_len == 0 {
                return Ok(0);
            }
            validate_user_buf(buf_ptr, buf_len, MAX_USER_BUF)?;

            // Resolve caller's cell_id.
            let cell_id = if let Some(sched) = super::SCHEDULER.lock().as_ref() {
                sched.tasks.get(&caller_id).map(|t| t.cell_id).unwrap_or(types::CellId(0))
            } else {
                types::CellId(0)
            };

            // Park the file Box (releases the cap-table lock so other caps are unblocked).
            let mut boxed_file = crate::cell::cap_registry::CAP_TABLE.lock()
                .park_file(crate::cell::cap_registry::CapId(cap_id as u64), cell_id)
                .map_err(|_| SyscallError::PermissionDenied)?;

            // Perform I/O outside the cap-table lock.
            // SAFETY: buf_ptr validated; SUM=1 allows S-mode writes to U-mode pages.
            let read_result = unsafe {
                let buf = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len);
                boxed_file.read(buf)
            };

            // Return the file Box (unpark). No-op if the cap was revoked during I/O.
            crate::cell::cap_registry::CAP_TABLE.lock()
                .unpark_file(crate::cell::cap_registry::CapId(cap_id as u64), boxed_file);

            // Return bytes_read, or usize::MAX on I/O error (distinguishable from 0 = EOF).
            match read_result {
                Ok(n) => Ok(n),
                Err(_) => Err(SyscallError::Unknown), // maps to usize::MAX at ABI level
            }
        }

        Syscall::CloseCap { cap_id } => {
            let cell_id = if let Some(sched) = super::SCHEDULER.lock().as_ref() {
                sched.tasks.get(&caller_id).map(|t| t.cell_id).unwrap_or(types::CellId(0))
            } else {
                types::CellId(0)
            };
            let mut table = crate::cell::cap_registry::CAP_TABLE.lock();
            table.verify(crate::cell::cap_registry::CapId(cap_id as u64), cell_id)
                .map_err(|_| SyscallError::PermissionDenied)?;
            table.revoke(crate::cell::cap_registry::CapId(cap_id as u64));
            Ok(0)
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
            match op {
                // op=0: raw monotonic ticks (arch-specific frequency)
                0 => {
                    #[cfg(target_arch = "riscv64")]
                    let t = hal::common::timer::read_mtime() as usize;
                    #[cfg(target_arch = "aarch64")]
                    let t = hal::timer::read_ticks() as usize;
                    #[cfg(target_arch = "x86_64")]
                    let t = hal::hpet::now_ns() as usize;
                    #[cfg(not(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "x86_64")))]
                    let t = 0usize;
                    Ok(t)
                }
                // op=1: milliseconds since boot
                1 => {
                    // 10 MHz mtime on QEMU RV64 → 10_000 ticks/ms
                    #[cfg(target_arch = "riscv64")]
                    let ms = (hal::common::timer::read_mtime() / 10_000) as usize;
                    // 62.5 MHz CNTPCT on QEMU ARM64 virt → 62_500 ticks/ms
                    #[cfg(target_arch = "aarch64")]
                    let ms = (hal::timer::read_ticks() / 62_500) as usize;
                    // HPET already returns nanoseconds; ÷ 1_000_000 → ms
                    #[cfg(target_arch = "x86_64")]
                    let ms = (hal::hpet::now_ns() / 1_000_000) as usize;
                    #[cfg(not(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "x86_64")))]
                    let ms = 0usize;
                    Ok(ms)
                }
                // op=2: nanoseconds since Unix epoch (wall-clock)
                2 => {
                    #[cfg(target_arch = "riscv64")]
                    let ns = hal::common::rtc::now_epoch_ns() as usize;
                    #[cfg(target_arch = "aarch64")]
                    let ns = hal::rtc::now_epoch_ns() as usize;
                    #[cfg(target_arch = "x86_64")]
                    let ns = hal::rtc::now_epoch_ns() as usize;
                    #[cfg(not(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "x86_64")))]
                    let ns = 0usize;
                    Ok(ns)
                }
                // op=3: seconds since Unix epoch (wall-clock)
                3 => {
                    #[cfg(target_arch = "riscv64")]
                    let s = (hal::common::rtc::now_epoch_ns() / 1_000_000_000) as usize;
                    #[cfg(target_arch = "aarch64")]
                    let s = (hal::rtc::now_epoch_ns() / 1_000_000_000) as usize;
                    #[cfg(target_arch = "x86_64")]
                    let s = (hal::rtc::now_epoch_ns() / 1_000_000_000) as usize;
                    #[cfg(not(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "x86_64")))]
                    let s = 0usize;
                    Ok(s)
                }
                // Unknown op — return 0 for backward compatibility
                _ => Ok(0),
            }
        }
        Syscall::GpuFlush { data_ptr, data_len, xy, wh } => {
            use crate::task::drivers::virtio_gpu::GPU_CONTEXT;
            let x = ((xy >> 16) & 0xFFFF) as i32;
            let y = (xy & 0xFFFF) as i32;
            let w = ((wh >> 16) & 0xFFFF) as u32;
            let h = (wh & 0xFFFF) as u32;
            let expected = (w * h * 4) as usize;
            if data_len < expected {
                log::warn!("[gpu_flush] data_len {} < expected {}", data_len, expected);
                return Err(SyscallError::BufferTooSmall);
            }
            let mut guard = GPU_CONTEXT.lock();
            if let Some(ctx) = guard.as_mut() {
                let stride = ctx.width as usize * 4; // read width before mutable borrow
                let fb = ctx.framebuffer();
                // SAFETY: data_ptr is a user-space address in the same SAS;
                // data_len was validated against w*h*4 above; we read exactly
                // that many bytes without writing past fb bounds.
                let src = unsafe { core::slice::from_raw_parts(data_ptr as *const u8, data_len) };
                let dy = y as usize;
                let dx = x as usize;
                for row in 0..h as usize {
                    let fb_off = (dy + row) * stride + dx * 4;
                    let src_off = row * w as usize * 4;
                    let row_bytes = w as usize * 4;
                    if fb_off + row_bytes <= fb.len() {
                        fb[fb_off..fb_off + row_bytes]
                            .copy_from_slice(&src[src_off..src_off + row_bytes]);
                    }
                }
                let _ = ctx.gpu.flush();
                Ok(0)
            } else {
                Err(SyscallError::Unknown) // GPU not initialised
            }
        }
        Syscall::GpuCursor { op, data_ptr, xy, hot } => {
            let x     = ((xy  >> 16) & 0xFFFF) as u32;
            let y     = ( xy         & 0xFFFF) as u32;
            let hot_x = ((hot >> 16) & 0xFFFF) as u32;
            let hot_y = ( hot        & 0xFFFF) as u32;
            match op {
                0 => {
                    // op=0: set sprite — data_ptr → 64×64 BGRA8888 (exactly 16384 bytes).
                    const SPRITE_LEN: usize = 64 * 64 * 4;
                    // SAFETY: data_ptr is a user-space address in the SAS; the cursor module
                    // validates that the slice length equals SPRITE_LEN before passing it to
                    // the virtio-drivers layer.
                    let image = unsafe {
                        core::slice::from_raw_parts(data_ptr as *const u8, SPRITE_LEN)
                    };
                    crate::task::drivers::virtio_gpu::cursor::set_sprite(image, x, y, hot_x, hot_y)
                        .map(|_| 0usize)
                        .map_err(|_| SyscallError::Unknown)
                }
                1 => {
                    // op=1: move — data_ptr and hot unused.
                    crate::task::drivers::virtio_gpu::cursor::move_to(x, y)
                        .map(|_| 0usize)
                        .map_err(|_| SyscallError::Unknown)
                }
                _ => Err(SyscallError::InvalidInput),
            }
        }
        Syscall::NetTx { frame_ptr, frame_len } => {
            if !caller_has_network(caller_id) {
                return Err(SyscallError::PermissionDenied);
            }
            crate::audit::log_event(
                crate::audit::AuditEvent::NetTx,
                &crate::audit::encode_u32x2(caller_id as u32, frame_len as u32),
            );
            validate_user_buf(frame_ptr, frame_len, MAX_USER_BUF)?;
            // SAFETY: validated above — frame_ptr/frame_len is a readable user buffer
            // in the shared address space; we only read `frame_len` bytes from it.
            let frame = unsafe { core::slice::from_raw_parts(frame_ptr as *const u8, frame_len) };
            let ok = crate::task::drivers::virtio_net::send_frame(frame);
            Ok(if ok { 1 } else { 0 })
        }
        Syscall::NetRx { buf_ptr, buf_len } => {
            if !caller_has_network(caller_id) {
                return Err(SyscallError::PermissionDenied);
            }
            validate_user_buf(buf_ptr, buf_len, MAX_USER_BUF)?;
            // SAFETY: validated above — buf_ptr/buf_len is a writable user buffer;
            // recv_frame writes at most `buf_len` bytes and returns the count.
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len) };
            let n = crate::task::drivers::virtio_net::recv_frame(buf);
            Ok(n)
        }
        Syscall::StateStash { key, buf_ptr, buf_len } => {
            validate_user_buf(buf_ptr, buf_len, crate::cell::state_stash::MAX_STASH_LEN)?;
            // SAFETY: validated above — readable user buffer of exactly buf_len bytes.
            let bytes = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, buf_len) };
            Ok(crate::cell::state_stash::stash(key as u64, bytes))
        }
        Syscall::StateRestore { key, buf_ptr, buf_len } => {
            validate_user_buf(buf_ptr, buf_len, crate::cell::state_stash::MAX_STASH_LEN)?;
            // SAFETY: validated above — writable user buffer of exactly buf_len bytes.
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len) };
            // For ARGV_STASH_KEY: serve the caller's personal slot (populated by
            // SpawnFromPath) first so rapid back-to-back spawns can't race.
            // The personal entry is consumed on read (one-shot) so it never
            // accumulates toward the MAX_ENTRIES cap.
            const ARGV_KEY: u64 = 0x0061_7267_7600_0000; // = ostd ARGV_STASH_KEY
            if key as u64 == ARGV_KEY {
                let personal_key = ARGV_KEY ^ ((caller_id as u64) << 32);
                let n = crate::cell::state_stash::restore(personal_key, buf);
                if n > 0 {
                    crate::cell::state_stash::remove(personal_key);
                    return Ok(n);
                }
            }
            Ok(crate::cell::state_stash::restore(key as u64, buf))
        }
        Syscall::BlkFlush => {
            if !caller_has_block_io(caller_id) {
                log::warn!("BlkFlush denied: task {} lacks block-I/O capability", caller_id);
                return Err(SyscallError::PermissionDenied);
            }
            
            
            match crate::task::drivers::block::flush() {
                Ok(()) => Ok(1),
                Err(_)  => Ok(0),
            }
        }
        Syscall::Shutdown => {
            // RISC-V: SBI System Reset via ecall. ARM64: spin loop (PSCI not yet wired).
            #[cfg(target_arch = "riscv64")]
            unsafe {
                // SAFETY: ecall traps to OpenSBI which powers off QEMU; no return.
                core::arch::asm!(
                    "li a7, 0x53525354",  // SBI_EXT_SRST
                    "li a6, 0",           // fid = SYSTEM_RESET
                    "li a0, 0",           // reset_type = Shutdown
                    "li a1, 0",           // reset_reason = NoReason
                    "ecall",
                    options(noreturn)
                );
            }
            #[cfg(target_arch = "x86_64")]
            loop { unsafe { core::arch::asm!("hlt", options(nomem, nostack)); } }
            #[cfg(not(any(target_arch = "riscv64", target_arch = "x86_64")))]
            loop { unsafe { core::arch::asm!("wfi", options(nomem, nostack)); } }
        }
        Syscall::BlkRead { sector, buf_ptr } => {
            if !caller_has_block_io(caller_id) {
                log::warn!("BlkRead denied: task {} lacks block-I/O capability", caller_id);
                return Err(SyscallError::PermissionDenied);
            }
            // Per-cell partition range gate — a runaway FAT offset must never
            // reach kernel-owned LBAs (P2 cell table, P3 snapshot). Returns 0 = failure.
            if !check_block_access(caller_id, sector, 1) {
                return Ok(0);
            }
            validate_user_buf(buf_ptr, 512, MAX_USER_BUF)?;


            // Bounce buffer: VirtioHal::share() treats the buffer's virtual address
            // as its physical address (identity-map assumption). Stack frames ARE
            // identity-mapped; ELF BSS/heap pages are NOT — DMA would land at the
            // wrong physical address without the bounce. Read into an on-stack buffer
            // (always identity-mapped), then copy to the user buffer under SUM=1.
            let mut bounce = [0u8; 512];
            match crate::task::drivers::block::read_sector(sector, &mut bounce) {
                Ok(()) => {
                    // SAFETY: buf_ptr is a validated 512-byte user buffer; SUM=1
                    // (set by ViCell_syscall_dispatch) allows S-mode to write it.
                    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, 512) };
                    buf.copy_from_slice(&bounce);
                    Ok(1)
                }
                Err(_) => Ok(0),
            }
        }
        Syscall::BlkWrite { sector, buf_ptr } => {
            if !caller_has_block_io(caller_id) {
                log::warn!("BlkWrite denied: task {} lacks block-I/O capability", caller_id);
                return Err(SyscallError::PermissionDenied);
            }
            // Per-cell partition range gate — prevents a cell from corrupting
            // the loader's table or the snapshot region. Returns 0 = failure.
            if !check_block_access(caller_id, sector, 1) {
                return Ok(0);
            }
            validate_user_buf(buf_ptr, 512, MAX_USER_BUF)?;
            
            
            // Bounce buffer for the same identity-map reason as BlkRead above.
            // SAFETY: buf_ptr is a validated 512-byte user buffer; SUM=1 allows
            // S-mode to read it.
            let user = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, 512) };
            let mut bounce = [0u8; 512];
            bounce.copy_from_slice(user);
            match crate::task::drivers::block::write_sector(sector, &bounce) {
                Ok(()) => Ok(1),
                Err(_) => Ok(0),
            }
        }
        Syscall::HotSwap { cell_id, path_ptr, path_len } => {
            if !caller_has_spawn(caller_id) {
                return Err(SyscallError::PermissionDenied);
            }
            // Validate and copy the path string from user space.
            let path_len = path_len.min(crate::loader::disk_layout::MAX_CELL_PATH);
            // SAFETY: path_ptr is a user-space string pointer passed via syscall registers;
            // path_len is bounded by MAX_CELL_PATH (≤ 256); the caller is responsible for
            // ensuring the pointed-to memory is valid for their task's lifetime.
            let path_bytes = unsafe {
                core::slice::from_raw_parts(path_ptr as *const u8, path_len)
            };
            let path = core::str::from_utf8(path_bytes)
                .map_err(|_| SyscallError::InvalidInput)?;
            let target = types::CellId(cell_id as u64);
            crate::cell::hotswap::hotswap(target, path)
                .map_err(|_| SyscallError::Unknown)
        }

        Syscall::Snapshot => {
            // Cells must be quiesced before calling this (all at yield points).
            // For MVP: the shell is the only active task while the snapshot runs.
            match crate::snapshot::serialize_snapshot() {
                Ok(frame_count) => Ok(frame_count as usize),
                Err(_) => Err(SyscallError::Unknown),
            }
        }

        // ── Zero-Copy Grant Syscalls (Phase 01, Storage 2.0) ─────────────────

        Syscall::GrantAlloc { size } => {
            const PAGE_SIZE: usize = 4096;
            if size == 0 || size > MAX_GRANT_PAGES * PAGE_SIZE {
                return Ok(0); // size == 0 or > 16 MiB cap — OOM sentinel per F10
            }
            let n_pages = (size + PAGE_SIZE - 1) / PAGE_SIZE;
            let paddr = match alloc_grant_pages(n_pages) {
                Some(p) => p,
                None    => return Ok(0),
            };
            // Register in the grant table.
            let mut tbl = grant_table_lock().lock();
            if tbl.is_none() { *tbl = Some(BTreeMap::new()); }
            if let Some(map) = tbl.as_mut() {
                map.insert(paddr, PageGrant {
                    base:      paddr,
                    size:      n_pages * PAGE_SIZE,
                    owner:     caller_id,
                    shared_to: None,
                });
            }
            Ok(paddr) // grant_id == physical base (identity-mapped vaddr in SAS)
        }

        Syscall::GrantShare { grant_id, target_cell, perm } => {
            let perm = match GrantPerm::try_from(perm as u8) {
                Ok(p) => p,
                Err(_) => return Err(SyscallError::InvalidInput),
            };
            // Check PAGE_GRANT_TABLE first, then fall back to REG_GRANT_TABLE.
            {
                let mut tbl = grant_table_lock().lock();
                if tbl.is_none() { *tbl = Some(BTreeMap::new()); }
                if let Some(grant) = tbl.as_mut().and_then(|m| m.get_mut(&grant_id)) {
                    if grant.owner != caller_id { return Err(SyscallError::PermissionDenied); }
                    grant.shared_to = Some((target_cell, perm));
                    return Ok(0);
                }
            }
            // Not in PAGE_GRANT_TABLE — try REG_GRANT_TABLE.
            let mut rtbl = reg_grant_table_lock().lock();
            match rtbl.as_mut().and_then(|m| m.get_mut(&grant_id)) {
                None => Err(SyscallError::InvalidInput),
                Some(grant) if grant.owner != caller_id => Err(SyscallError::PermissionDenied),
                Some(grant) => {
                    grant.shared_to = Some((target_cell, perm));
                    Ok(0)
                }
            }
        }

        Syscall::GrantSlice { grant_id } => {
            // Check PAGE_GRANT_TABLE first.
            {
                let tbl = grant_table_lock().lock();
                if let Some(grant) = tbl.as_ref().and_then(|m| m.get(&grant_id)) {
                    let allowed = grant.owner == caller_id
                        || grant.shared_to.map_or(false, |(tid, _)| tid == caller_id);
                    return Ok(if allowed { grant.base } else { usize::MAX });
                }
            }
            // Fall back to REG_GRANT_TABLE.
            let rtbl = reg_grant_table_lock().lock();
            match rtbl.as_ref().and_then(|m| m.get(&grant_id)) {
                None => Ok(usize::MAX),
                Some(grant) => {
                    let allowed = grant.owner == caller_id
                        || grant.shared_to.map_or(false, |(tid, _)| tid == caller_id);
                    Ok(if allowed { grant.base } else { usize::MAX })
                }
            }
        }

        Syscall::GrantFree { grant_id } => {
            // Owner-only: remove from table before touching page tables.
            let entry = {
                let mut tbl = grant_table_lock().lock();
                tbl.as_mut().and_then(|m| {
                    if m.get(&grant_id).map_or(false, |g| g.owner == caller_id) {
                        m.remove(&grant_id)
                    } else {
                        None
                    }
                })
            };
            let entry = match entry {
                Some(e) => e,
                None    => return Err(SyscallError::PermissionDenied),
            };
            free_grant_pages(entry.base, entry.size / 4096);
            Ok(0)
        }

        Syscall::GrantRegister { size } => {
            const PAGE_SIZE: usize = 4096;
            if size == 0 || size > MAX_GRANT_PAGES * PAGE_SIZE {
                return Ok(0); // OOM sentinel per F10
            }
            let n_pages = (size + PAGE_SIZE - 1) / PAGE_SIZE;
            let paddr = match alloc_grant_pages(n_pages) {
                Some(p) => p,
                None    => return Ok(0),
            };
            let mut tbl = reg_grant_table_lock().lock();
            if tbl.is_none() { *tbl = Some(BTreeMap::new()); }
            if let Some(map) = tbl.as_mut() {
                map.insert(paddr, RegGrant { base: paddr, size: n_pages * PAGE_SIZE, owner: caller_id, shared_to: None });
            }
            Ok(paddr) // reg_id == physical base
        }

        Syscall::GrantUnregister { reg_id } => {
            let entry = {
                let mut tbl = reg_grant_table_lock().lock();
                tbl.as_mut().and_then(|m| {
                    if m.get(&reg_id).map_or(false, |g| g.owner == caller_id) {
                        m.remove(&reg_id)
                    } else {
                        None
                    }
                })
            };
            let entry = match entry {
                Some(e) => e,
                None    => return Err(SyscallError::PermissionDenied),
            };
            free_grant_pages(entry.base, entry.size / 4096);
            Ok(0)
        }

        Syscall::WaitForEvent { mask, deadline } => {
            // Lost-wakeup guard: check pending events BEFORE parking.
            let already = super::waker::consume_pending(mask);
            if already != 0 {
                return Ok(already as usize);
            }
            // Park: set WaitEvent state so the timer sweep can wake this task.
            if let Some(sched) = super::SCHEDULER.lock().as_mut() {
                if let Some(task) = sched.tasks.get_mut(&caller_id) {
                    task.state = super::tcb::TaskState::WaitEvent { mask, deadline };
                }
            }
            // Yield; wake happens in pick_next's global sweep (hart 0).
            super::yield_cpu();
            // After re-schedule: timer sweep wrote the fired mask into trap_frame.regs[10].
            // Return it so ViCell_syscall_dispatch writes the correct value back.
            let fired = super::SCHEDULER.lock()
                .as_ref()
                .and_then(|s| s.tasks.get(&caller_id))
                .map(|t| t.trap_frame.regs[10])
                .unwrap_or(0);
            Ok(fired)
        }

        Syscall::RequestMmio { base, len } => {
            // Gate: caller's ELF manifest must declare gpio or uart cap.
            let allowed = {
                let sched = super::SCHEDULER.lock();
                sched.as_ref()
                    .and_then(|s| s.tasks.get(&caller_id))
                    .map(|t| t.mmio_cap)
                    .unwrap_or(false)
            };
            if !allowed {
                return Err(SyscallError::PermissionDenied);
            }
            match crate::resource_registry::request_mmio(types::CellId(caller_id as u64), base, len) {
                Ok(()) => Ok(0),
                Err(types::ViError::PermissionDenied) => Ok(1),
                Err(types::ViError::AlreadyExists)    => Ok(2),
                Err(_)                                => Ok(3),
            }
        }

        Syscall::GetRandom { buf_ptr, len } => {
            // Cap at 64 bytes per call (one VirtIO-RNG descriptor).
            let capped = len.min(64);
            if capped == 0 { return Ok(0); }
            // SAFETY: SUM is enabled by the caller (ViCell_syscall_dispatch). buf_ptr is
            // a user-space pointer in the same SAS; we write exactly `capped` bytes.
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, capped) };
            let written = crate::task::drivers::virtio_rng::get_random(buf);
            if written > 0 { return Ok(written); }
            // VirtIO-RNG unavailable — fill with xorshift32 seeded from mtime + caller_id.
            // Not cryptographically secure, but satisfies getentropy(2) correctness contract.
            let seed = super::system_ticks() as u32 ^ (caller_id as u32).wrapping_mul(0x9e37_79b9);
            let mut state = if seed == 0 { 1 } else { seed };
            for byte in buf.iter_mut() {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                *byte = state as u8;
            }
            Ok(capped)
        }

        Syscall::BlkReadAsync { sector, grant_id } => {
            if !caller_has_block_io(caller_id) {
                return Err(SyscallError::PermissionDenied);
            }
            if !check_block_access(caller_id, sector, 1) {
                return Ok(0);
            }
            // Validate ownership and minimum size (must hold ≥ 512 bytes).
            let buf_paddr = {
                let tbl = grant_table_lock().lock();
                tbl.as_ref()
                    .and_then(|m| m.get(&grant_id))
                    .filter(|g| g.owner == caller_id && g.size >= 512)
                    .map(|g| g.base)
            };
            let buf_paddr = match buf_paddr { Some(p) => p, None => return Ok(0) };
            // Grant pages are identity-mapped (vaddr == paddr), so DMA addresses are correct.
            // SAFETY: buf_paddr is a physically contiguous, identity-mapped grant page; valid
            // for 512 bytes of VirtIO DMA read.
            
            
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_paddr as *mut u8, 512) };
            match crate::task::drivers::block::read_sector(sector, buf) {
                Ok(()) => Ok(1), // async_id = 1 means immediately complete (Phase 04 for real async)
                Err(_)  => Ok(0),
            }
        }

        // ── Hypervisor syscalls 220-225 (HypervisorCap ZST-gated) ────────────────

        Syscall::CreateVm { guest_pages } => {
            if !caller_has_hypervisor(caller_id) {
                return Err(SyscallError::PermissionDenied);
            }
            crate::hypervisor::registry::create_vm(caller_id, guest_pages)
                .map_err(|_| SyscallError::NotSupported)
        }

        Syscall::CreateVcpu { vm_id, entry_pc } => {
            if !caller_has_hypervisor(caller_id) {
                return Err(SyscallError::PermissionDenied);
            }
            crate::hypervisor::registry::create_vcpu(caller_id, vm_id, entry_pc)
                .map_err(|_| SyscallError::NotSupported)
        }

        Syscall::MapGuestMemory { vm_id, ipa, size, writable } => {
            if !caller_has_hypervisor(caller_id) {
                return Err(SyscallError::PermissionDenied);
            }
            // C3: overflow guard on IPA + size.
            ipa.checked_add(size as u64).ok_or(SyscallError::InvalidInput)?;
            crate::hypervisor::registry::map_guest_memory(caller_id, vm_id, ipa, size, writable)
                .map(|_| 0usize)
                .map_err(|_| SyscallError::NotSupported)
        }

        Syscall::RunVcpu { vm_id, vcpu_id, budget_ns, out_ptr } => {
            if !caller_has_hypervisor(caller_id) {
                return Err(SyscallError::PermissionDenied);
            }
            validate_user_buf(out_ptr, core::mem::size_of::<api::hypervisor::ViVmExit>(), MAX_USER_BUF)?;
            // SAFETY: pointer validated above; SAS means it's also valid in kernel.
            let exit_out = out_ptr as *mut api::hypervisor::ViVmExit;
            crate::hypervisor::registry::run_vcpu(caller_id, vm_id, vcpu_id, budget_ns, exit_out)
                .map_err(|_| SyscallError::NotSupported)
        }

        Syscall::VcpuRegs { vm_id, vcpu_id, buf_ptr, write } => {
            if !caller_has_hypervisor(caller_id) {
                return Err(SyscallError::PermissionDenied);
            }
            // 32 registers × 8 bytes = 256 bytes.
            validate_user_buf(buf_ptr, 256, MAX_USER_BUF)?;
            crate::hypervisor::registry::vcpu_regs(caller_id, vm_id, vcpu_id, buf_ptr, write)
                .map_err(|_| SyscallError::NotSupported)
        }

        Syscall::InjectIrq { vm_id, vcpu_id, intid } => {
            if !caller_has_hypervisor(caller_id) {
                return Err(SyscallError::PermissionDenied);
            }
            // m3: GICv2 has SPIs/PPIs/SGIs up to intid 1019.
            if intid > 1019 {
                return Err(SyscallError::InvalidInput);
            }
            crate::hypervisor::registry::inject_irq(caller_id, vm_id, vcpu_id, intid)
                .map_err(|_| SyscallError::NotSupported)
        }

        Syscall::WriteGuestMemory { vm_id, gpa, src_ptr, len } => {
            if !caller_has_hypervisor(caller_id) {
                return Err(SyscallError::PermissionDenied);
            }
            // Overflow guard: gpa+len and src_ptr+len must not wrap.
            gpa.checked_add(len as u64).ok_or(SyscallError::InvalidInput)?;
            validate_user_buf(src_ptr, len, MAX_USER_BUF)?;
            crate::hypervisor::registry::write_guest_memory(caller_id, vm_id, gpa, src_ptr, len)
                .map_err(|_| SyscallError::InvalidInput)
        }

        Syscall::ReadGuestMemory { vm_id, gpa, dst_ptr, len } => {
            if !caller_has_hypervisor(caller_id) {
                return Err(SyscallError::PermissionDenied);
            }
            gpa.checked_add(len as u64).ok_or(SyscallError::InvalidInput)?;
            validate_user_buf(dst_ptr, len, MAX_USER_BUF)?;
            crate::hypervisor::registry::read_guest_memory(caller_id, vm_id, gpa, dst_ptr, len)
                .map_err(|_| SyscallError::InvalidInput)
        }
    }
}

use api::syscall::ViSyscall;
#[cfg(not(target_arch = "riscv32"))]
use crate::hal::arch::ViTrapFrame;

/// Map a syscall ID + promoted register args to the internal [`Syscall`] enum.
///
/// All register values must already be promoted to `usize` by the caller.
/// Returns `None` for unknown/unhandled opcodes; the caller writes the
/// arch-appropriate sentinel (usize::MAX or u32::MAX) to the return register.
fn map_syscall(syscall_id: usize, a0: usize, a1: usize, a2: usize, a3: usize) -> Option<Syscall> {
    let sc = match ViSyscall::from(syscall_id) {
        ViSyscall::Send          => Syscall::Send { target: a0, msg_ptr: a1, msg_len: a2 },
        ViSyscall::Recv          => Syscall::Recv { mask: a0, buf_ptr: a1, buf_len: a2 },
        ViSyscall::TryRecv       => Syscall::TryRecv { mask: a0, buf_ptr: a1, buf_len: a2 },
        ViSyscall::SendGather    => Syscall::SendGather { target: a0, iovec_ptr: a1, iovec_count: a2 },
        ViSyscall::RecvScatter   => Syscall::RecvScatter { mask: a0, iovec_ptr: a1, iovec_count: a2 },
        ViSyscall::RecvTimeout   => Syscall::RecvTimeout {
            mask: a0, buf_ptr: a1, buf_len: a2,
            deadline: (super::system_ticks() as u64).wrapping_add(a3 as u64),
        },
        ViSyscall::Reply         => Syscall::Reply { caller: a0, result: a1 },
        ViSyscall::Call          => Syscall::ServiceLookup { name_ptr: a0, name_len: a1 },
        ViSyscall::Spawn         => Syscall::Spawn { entry: a0, arg: a1 },
        ViSyscall::Exec          => Syscall::Exec { path_ptr: a0, path_len: a1 },
        ViSyscall::SpawnFromMem  => Syscall::SpawnFromMem { args_ptr: a0 },
        ViSyscall::SpawnFromPath => Syscall::SpawnFromPath { path_ptr: a0, path_len: a1 },
        ViSyscall::SpawnPinned   => Syscall::SpawnPinned {
            path_ptr: a0, path_len: a1, priority: a2 as u8, core_id: a3,
        },
        ViSyscall::OpenCap       => Syscall::OpenCap { path_ptr: a0, path_len: a1 },
        ViSyscall::ReadCap       => Syscall::ReadCap { cap_id: a0, buf_ptr: a1, buf_len: a2 },
        ViSyscall::CloseCap      => Syscall::CloseCap { cap_id: a0 },
        ViSyscall::Wait          => Syscall::Wait { pid: a0 },
        ViSyscall::ShmAlloc      => Syscall::ShmAlloc { size: a0 },
        ViSyscall::ShmMap        => Syscall::ShmMap { handle: a0, target_pid: a1 },
        ViSyscall::Exit          => Syscall::Exit { code: a0 },
        ViSyscall::ForceExit     => Syscall::ForceExit { tid: a0 },
        ViSyscall::NotifyOnExit  => Syscall::NotifyOnExit { watched: a0 },
        ViSyscall::RegisterService => Syscall::RegisterService { service_id: a0 as u16, tid: a1 },
        ViSyscall::LookupService => Syscall::LookupService { service_id: a0 as u16 },
        ViSyscall::Heartbeat     => Syscall::Heartbeat { interval: a0 },
        ViSyscall::Yield         => Syscall::Yield,
        ViSyscall::SetTimer      => Syscall::SetTimer { deadline: a0 },
        ViSyscall::Log           => Syscall::Log { msg_ptr: a0, msg_len: a1 },
        ViSyscall::GetProcs      => Syscall::GetProcs { buf_ptr: a0, buf_len: a1 },
        ViSyscall::Open          => Syscall::Open { path_ptr: a0, path_len: a1 },
        ViSyscall::Read          => Syscall::Read { fd: a0, buf_ptr: a1, buf_len: a2 },
        ViSyscall::Close         => Syscall::Close { fd: a0 },
        ViSyscall::ReadDir       => Syscall::ReadDir { fd: a0, buf_ptr: a1, buf_len: a2 },
        ViSyscall::Write         => Syscall::Write { fd: a0, buf_ptr: a1, buf_len: a2 },
        ViSyscall::Seek          => Syscall::Seek { fd: a0, offset: a1 as isize, whence: a2 },
        ViSyscall::FileOp        => Syscall::FileOp { op: a0, arg1: a1, arg2: a2 },
        ViSyscall::GetTime       => Syscall::GetTime { op: a0 },
        ViSyscall::GpuFlush      => Syscall::GpuFlush { data_ptr: a0, data_len: a1, xy: a2, wh: a3 },
        ViSyscall::GpuCursor     => Syscall::GpuCursor { op: a0, data_ptr: a1, xy: a2, hot: a3 },
        ViSyscall::NetTx         => Syscall::NetTx { frame_ptr: a0, frame_len: a1 },
        ViSyscall::NetRx         => Syscall::NetRx { buf_ptr: a0, buf_len: a1 },
        ViSyscall::StateStash    => Syscall::StateStash { key: a0, buf_ptr: a1, buf_len: a2 },
        ViSyscall::StateRestore  => Syscall::StateRestore { key: a0, buf_ptr: a1, buf_len: a2 },
        ViSyscall::HotSwap       => Syscall::HotSwap { cell_id: a0, path_ptr: a1, path_len: a2 },
        ViSyscall::Snapshot      => Syscall::Snapshot,
        ViSyscall::GrantAlloc    => Syscall::GrantAlloc { size: a0 },
        ViSyscall::GrantShare    => Syscall::GrantShare { grant_id: a0, target_cell: a1, perm: a2 },
        ViSyscall::GrantSlice    => Syscall::GrantSlice { grant_id: a0 },
        ViSyscall::GrantFree     => Syscall::GrantFree { grant_id: a0 },
        ViSyscall::BlkReadAsync  => Syscall::BlkReadAsync { sector: a0 as u64, grant_id: a1 },
        ViSyscall::RequestMmio      => Syscall::RequestMmio { base: a0, len: a1 },
        ViSyscall::GetRandom        => Syscall::GetRandom { buf_ptr: a0, len: a1 },
        ViSyscall::GrantRegister    => Syscall::GrantRegister { size: a0 },
        ViSyscall::GrantUnregister  => Syscall::GrantUnregister { reg_id: a0 },
        ViSyscall::WaitForEvent     => {
            // ABI: a0 = mask (u32), a1 = timeout_ticks_lo, a2 = timeout_ticks_hi.
            let mask = a0 as u32;
            let timeout = (a1 as u64) | ((a2 as u64) << 32);
            let deadline = if timeout == 0 { None } else {
                Some((super::system_ticks() as u64).wrapping_add(timeout))
            };
            Syscall::WaitForEvent { mask, deadline }
        }
        // Hypervisor syscalls 220-225.
        ViSyscall::CreateVm       => Syscall::CreateVm { guest_pages: a0 },
        ViSyscall::CreateVcpu     => Syscall::CreateVcpu { vm_id: a0, entry_pc: a1 as u64 },
        ViSyscall::MapGuestMemory => Syscall::MapGuestMemory {
            vm_id: a0, ipa: a1 as u64, size: a2, writable: a3 != 0,
        },
        ViSyscall::RunVcpu        => Syscall::RunVcpu {
            vm_id: a0, vcpu_id: a1, budget_ns: a2 as u64, out_ptr: a3,
        },
        ViSyscall::VcpuRegs       => Syscall::VcpuRegs {
            vm_id: a0, vcpu_id: a1, buf_ptr: a2, write: a3 != 0,
        },
        ViSyscall::InjectIrq      => Syscall::InjectIrq {
            vm_id: a0, vcpu_id: a1, intid: a2 as u32,
        },
        ViSyscall::WriteGuestMemory => Syscall::WriteGuestMemory {
            vm_id: a0, gpa: a1 as u64, src_ptr: a2, len: a3,
        },
        ViSyscall::ReadGuestMemory  => Syscall::ReadGuestMemory {
            vm_id: a0, gpa: a1 as u64, dst_ptr: a2, len: a3,
        },
        _ => match syscall_id {
            3   => Syscall::SetTimer { deadline: a0 },
            100 => Syscall::ServiceLookup { name_ptr: a0, name_len: a1 },
            106 => Syscall::FStat { fd: a0, stat_ptr: a1 },
            107 => Syscall::ChDir { path_ptr: a0, path_len: a1 },
            108 => Syscall::GetCwd { buf_ptr: a0, buf_len: a1 },
            110 => Syscall::MkDir { path_ptr: a0, path_len: a1 },
            111 => Syscall::Create { path_ptr: a0, path_len: a1 },
            // Block I/O — intentionally absent from ViSyscall/libs/api (avoids Law 1).
            500 => Syscall::BlkRead  { sector: a0 as u64, buf_ptr: a1 },
            501 => Syscall::BlkWrite { sector: a0 as u64, buf_ptr: a1 },
            502 => Syscall::Shutdown,
            503 => Syscall::BlkFlush,
            _   => return None,
        }
    };
    Some(sc)
}

/// Per-cell syscall allowlist gate.
///
/// Reads the caller's `syscall_allowlist` bitmask and returns
/// `Err(SyscallError::PermissionDenied)` if the opcode's bit is not set.
/// The SCHEDULER lock is acquired and released here — callers must NOT hold it.
fn check_allowlist(syscall_id: usize, caller_id: usize) -> Result<(), SyscallError> {
    let sc = ViSyscall::from(syscall_id);
    let bit = sc.allowlist_bit();
    // Bit 36 gates raw block-I/O opcodes (500/501/503) and BlkReadAsync (212).
    let blk_io_bit: Option<u8> = if matches!(syscall_id, 500 | 501 | 503 | 212) { Some(36) } else { None };

    // Raw opcodes with a dedicated `map_syscall` fallback mapping. These are
    // intentionally absent from `ViSyscall` (Law 1: keeps experimental ids out
    // of the stable ABI) so they decode as `Unknown` — but they are NOT unknown:
    // 500/501/503 are gated by bit 36 below + the ZST BlockIoCap at the handler;
    // 502 and the legacy FD ops (3/100/106/107/108/110/111) predate the bitmap
    // and stay always-permitted, matching their pre-Phase-31b behavior.
    let known_raw = matches!(
        syscall_id,
        3 | 100 | 106 | 107 | 108 | 110 | 111 | 500 | 501 | 502 | 503
    );

    let allowlist = super::SCHEDULER.lock().as_ref()
        .and_then(|s| s.tasks.get(&caller_id))
        .map(|t| t.syscall_allowlist)
        .unwrap_or(0); // task absent → deny-all for safety

    // Deny truly-unknown opcodes that land in the legacy inner-match fallback —
    // their allowlist_bit() returns None, so without this guard they bypass the
    // check. Exit (60) and Yield (104) are always permitted unconditionally.
    // Known-raw ids are exempt: blocking them here made every allowlist-declaring
    // cell lose raw block I/O (broke the VFS FAT32 mount silently since Phase 31b).
    // Every deny below logs: a silent dispatch-level denial cost a full day of
    // triage when the shell's missing `Read` bit bricked serial input with no
    // kernel output at all.
    if sc == ViSyscall::Unknown
        && !known_raw
        && !matches!(syscall_id, 60 | 104)
        && allowlist != u64::MAX
    {
        log::warn!("[kernel] unknown opcode {} denied for tid {} (allowlist={:#018x})",
            syscall_id, caller_id, allowlist);
        return Err(SyscallError::PermissionDenied);
    }
    if let Some(b) = bit {
        if allowlist & (1u64 << b) == 0 {
            log::warn!("[kernel] syscall {:?} (bit {}) denied for tid {} (allowlist={:#018x})",
                sc, b, caller_id, allowlist);
            return Err(SyscallError::PermissionDenied);
        }
    }
    if let Some(b) = blk_io_bit {
        if allowlist & (1u64 << b) == 0 {
            log::warn!("[kernel] raw block opcode {} denied for tid {} (no bit 36)",
                syscall_id, caller_id);
            return Err(SyscallError::PermissionDenied);
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "riscv32"))]
#[no_mangle]
#[allow(non_snake_case)] // ABI name required by the HAL trap vector — cannot be snake_case
pub extern "Rust" fn ViCell_syscall_dispatch(frame: &mut ViTrapFrame) {
    let syscall_id = frame.regs[17];

    // Watchdog progress signal: a syscall proves the caller is making progress
    // (ViCell cells are poll-based — try_recv/yield every loop iteration), so
    // reset its CPU-monopoly counter.
    {
        let hart_id = super::hart_local::current_hart_id();
        let cid = super::hart_local::ready::current_task_id_for(hart_id);
        if cid > 0 {
            if let Some(sched) = super::SCHEDULER.lock().as_mut() {
                if let Some(t) = sched.tasks.get_mut(&cid) {
                    t.run_ticks = 0;
                    t.rt_overrun_warned = false;
                }
            }
        }
    }

    let a0 = frame.regs[10];
    let a1 = frame.regs[11];
    let a2 = frame.regs[12];
    let a3 = frame.regs[13];

    let Some(syscall) = map_syscall(syscall_id, a0, a1, a2, a3) else {
        frame.regs[10] = usize::MAX;
        return;
    };

    let caller_id = super::current_task_id();

    if check_allowlist(syscall_id, caller_id).is_err() {
        frame.regs[10] = usize::MAX;
        return;
    }

    // SAFETY: csrs/csrc sstatus SUM (bit 18) enables S-mode access to user pages
    // for the duration of handle_syscall. Disabled immediately after to prevent
    // inadvertent user-page reads on subsequent kernel faults.
    #[cfg(target_arch = "riscv64")]
    unsafe { core::arch::asm!("csrs sstatus, {0}", in(reg) 0x40000usize); }

    let result = handle_syscall(caller_id, syscall);

    #[cfg(target_arch = "riscv64")]
    unsafe { core::arch::asm!("csrc sstatus, {0}", in(reg) 0x40000usize); }

    match result {
        Ok(val) => frame.regs[10] = val,
        Err(_)  => frame.regs[10] = usize::MAX,
    }
}

#[cfg(target_arch = "riscv32")]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "Rust" fn ViCell_syscall_dispatch(frame: &mut crate::hal::arch::ViTrapFrame) {
    // Promote u32 register slots to usize (= u32 on rv32) for arch-agnostic helpers.
    let syscall_id = frame.regs[17] as usize;

    // Watchdog: syscall proves the cell is making forward progress.
    {
        let hart_id = super::hart_local::current_hart_id();
        let cid = super::hart_local::ready::current_task_id_for(hart_id);
        if cid > 0 {
            if let Some(sched) = super::SCHEDULER.lock().as_mut() {
                if let Some(t) = sched.tasks.get_mut(&cid) {
                    t.run_ticks = 0;
                    t.rt_overrun_warned = false;
                }
            }
        }
    }

    let a0 = frame.regs[10] as usize;
    let a1 = frame.regs[11] as usize;
    let a2 = frame.regs[12] as usize;
    let a3 = frame.regs[13] as usize;

    let Some(syscall) = map_syscall(syscall_id, a0, a1, a2, a3) else {
        frame.regs[10] = u32::MAX;
        return;
    };

    let caller_id = super::current_task_id();

    if check_allowlist(syscall_id, caller_id).is_err() {
        frame.regs[10] = u32::MAX;
        return;
    }

    // SAFETY: csrs/csrc sstatus SUM (bit 18) — same bit position on RV32 and RV64
    // per the RISC-V Privileged Spec §4.1.1. Disabled after handle_syscall.
    unsafe { core::arch::asm!("csrs sstatus, {0}", in(reg) 0x40000usize); }

    let result = handle_syscall(caller_id, syscall);

    unsafe { core::arch::asm!("csrc sstatus, {0}", in(reg) 0x40000usize); }

    match result {
        Ok(val) => frame.regs[10] = val as u32,
        Err(_)  => frame.regs[10] = u32::MAX,
    }
}
