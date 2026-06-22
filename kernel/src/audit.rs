//! Kernel audit ring buffer — 256 KB in-memory event log.
//!
//! Records key kernel events (IPC, file, net, spawn, fault, exit) with a
//! monotonic timestamp for post-mortem analysis.  A background Cell (or shell
//! builtin) drains the ring and appends to `/data/kernel.log`.
//!
//! # Concurrency
//! The timer ISR may preempt a syscall-context `log_event()` call, creating
//! two apparent producers.  We guard each write by briefly disabling S-mode
//! interrupts (`csrci sstatus, 0x2`) — safe on single-hart.
//!
//! # Overflow
//! When the ring is full, new writes are dropped and `DROPPED` is incremented.
//! No blocking, no corruption.
//!
//! # Record format
//! ```
//! [u64 mtime_ticks LE][u8 event_type][u8 payload_len][payload…]
//! ```
//! Total minimum: 10 bytes per event.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

const BUF_SIZE: usize = 256 * 1024; // must be a power of two
const MASK: usize = BUF_SIZE - 1;

/// Kernel audit event type byte.
#[repr(u8)]
#[allow(dead_code)] // reason: variants logged by different kernel subsystems
pub enum AuditEvent {
    IpcSend   = 1,
    IpcRecv   = 2,
    FileOpen  = 3,
    FileWrite = 4,
    NetTx     = 5,
    NetRx     = 6,
    CellSpawn      = 7,
    CellFault      = 8,
    CellExit       = 9,
    CellSpawnDenied = 10,
    /// An RT-priority cell's `RecvTimeout` deadline elapsed before its awaited
    /// message arrived — a missed control-loop cycle. Payload: `encode_u32x2(cell_id,
    /// cumulative_miss_count)`. Observability only (no enforcement): makes RT misses
    /// visible for post-mortem / tuning once real-hardware bench data is available.
    RtDeadlineMiss = 11,
    /// An RT-priority cell crossed the CPU-monopoly *warning* threshold (a fraction
    /// of the watchdog budget) without yielding — an early signal BEFORE the hard
    /// watchdog kill. Payload: `encode_u32x2(cell_id, run_ticks)`. One-shot per episode.
    RtCpuOverrun = 12,
    /// A cell that opted into liveness heartbeating missed its deadline — a silent hang
    /// (deadlock / stuck loop) the CPU watchdog cannot detect. The kernel terminates it
    /// for supervised restart. Payload: `encode_u32x2(cell_id, tid)`.
    CellHung = 13,
    /// A cell invoked a syscall not present in its `__ViCell_syscalls` allowlist.
    /// Payload: `encode_u32x2(caller_tid, allowlist_bit)`.
    SyscallDenied = 14,
    /// A cell's ELF image was measured at spawn (integrity measurement, IMA-style).
    /// Payload: `encode_u32x2(tid, sha256_prefix_le_u32)`. The full digest and the
    /// rolling aggregate live in [`crate::measurement_log`].
    CellMeasure = 15,
    /// The signed operator policy was loaded + verified at boot (P5).
    /// Payload: `encode_u32x2(entry_count, 0)`.
    PolicyLoaded = 16,
    /// The operator policy failed signature verification or parsing — fail-closed.
    /// Payload: `encode_u32x2(reason_code, 0)`.
    PolicyInvalid = 17,
    /// No operator policy present in VIFS1. Payload: `encode_u32x2(0, 0)`.
    PolicyAbsent = 18,
    /// Operator policy narrowed a cell's spawn-time caps. Payload:
    /// `encode_u32x2(tid, dropped_flags)` (dropped_flags: bit0 block_io, bit1
    /// network, bit2 spawn, bit3 hypervisor).
    CapNarrowedByPolicy = 19,
    /// Runtime revocation: a SpawnCap holder stripped capabilities from a live cell.
    /// Payload: `encode_u32x2(target_tid, cap_mask)` where `cap_mask` matches
    /// `api::syscall::cap_mask` constants (bit0=block_io, bit1=network, bit2=spawn,
    /// bit3=hypervisor, bits8-15=mmio_devices, bits16-23=block_regions).
    CapRevoked = 20,
}

struct AuditRing {
    buf:     UnsafeCell<[u8; BUF_SIZE]>,
    head:    AtomicUsize,
    tail:    AtomicUsize,
    dropped: AtomicUsize,
}

// SAFETY: single-hart kernel; interrupt-disable in log_event prevents
// timer ISR from preempting a partial write.
unsafe impl Sync for AuditRing {}

static RING: AuditRing = AuditRing {
    // SAFETY: UnsafeCell<[u8; N]> in a static is always zero-initialised.
    buf:     UnsafeCell::new([0u8; BUF_SIZE]),
    head:    AtomicUsize::new(0),
    tail:    AtomicUsize::new(0),
    dropped: AtomicUsize::new(0),
};

/// Write a kernel audit event to the ring.
///
/// Disables S-mode interrupts for the duration of the write so the timer ISR
/// cannot preempt a partial record.  Drops silently on ring-full.
pub fn log_event(event: AuditEvent, payload: &[u8]) {
    let plen = payload.len().min(255) as u8;
    let record_len = 10 + plen as usize;

    // Disable S-mode interrupts: prevent timer ISR from racing this write.
    // SAFETY: single-hart; restoring sie after the write preserves the invariant.
    let sie_was_set = {
        #[cfg(target_arch = "riscv64")]
        {
            let v: usize;
            // SAFETY: csrrci clears SIE (bit 1) and returns old sstatus.
            unsafe { core::arch::asm!("csrrci {}, sstatus, 0x2", out(reg) v) };
            v & 0x2 != 0
        }
        #[cfg(not(target_arch = "riscv64"))]
        { false }
    };

    let head = RING.head.load(Ordering::Relaxed);
    let tail = RING.tail.load(Ordering::Acquire);

    // Drop-on-full: never overwrite consumer bytes.
    if head.wrapping_sub(tail) + record_len > BUF_SIZE {
        RING.dropped.fetch_add(1, Ordering::Relaxed);
        restore_sie(sie_was_set);
        return;
    }

    #[cfg(target_arch = "riscv64")]
    let mtime = hal::common::timer::read_mtime().to_le_bytes();
    #[cfg(not(target_arch = "riscv64"))]
    let mtime = 0u64.to_le_bytes();
    let buf = unsafe { &mut *RING.buf.get() };
    let mut pos = head;
    for &b in mtime.iter()
        .chain(core::slice::from_ref(&(event as u8)))
        .chain(core::slice::from_ref(&plen))
        .chain(&payload[..plen as usize])
    {
        buf[pos & MASK] = b;
        pos = pos.wrapping_add(1);
    }

    // Publish the write — consumer sees it only after this Release store.
    RING.head.store(head.wrapping_add(record_len), Ordering::Release);

    restore_sie(sie_was_set);
}

#[inline(always)]
fn restore_sie(was_set: bool) {
    if was_set {
        #[cfg(target_arch = "riscv64")]
        // SAFETY: restoring SIE to its prior state.
        unsafe { core::arch::asm!("csrsi sstatus, 0x2"); }
    }
}

/// Drain up to `out.len()` bytes from the ring.  Returns bytes copied.
///
/// Called by the log-flusher Cell (consumer side).
pub fn drain(out: &mut [u8]) -> usize {
    let head = RING.head.load(Ordering::Acquire);
    let tail = RING.tail.load(Ordering::Relaxed);
    let available = head.wrapping_sub(tail);
    if available == 0 { return 0; }

    let to_copy = available.min(out.len());
    let buf = unsafe { &*RING.buf.get() };
    for (i, byte) in out[..to_copy].iter_mut().enumerate() {
        *byte = buf[(tail.wrapping_add(i)) & MASK];
    }
    RING.tail.store(tail.wrapping_add(to_copy), Ordering::Release);
    to_copy
}

/// Number of records dropped due to ring-full since boot.
pub fn dropped_count() -> usize {
    RING.dropped.load(Ordering::Relaxed)
}

// ── Payload helpers ────────────────────────────────────────────────────────

/// Encode two `u32` values into a fixed 8-byte payload.
pub fn encode_u32x2(a: u32, b: u32) -> [u8; 8] {
    let mut out = [0u8; 8];
    out[..4].copy_from_slice(&a.to_le_bytes());
    out[4..].copy_from_slice(&b.to_le_bytes());
    out
}
