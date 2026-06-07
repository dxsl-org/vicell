//! Per-cell heap quota enforcement.
//!
//! Uses a split design to avoid the alloc-inside-alloc deadlock:
//! - `QUOTA_LIMITS`: `Spinlock<BTreeMap<usize, usize>>` stores the limit per Cell.
//!   Only locked in `register`/`deregister` — never inside `GlobalAlloc::alloc`.
//! - `IN_USE`: `[AtomicUsize; MAX_CELLS]` stores the live byte count per Cell.
//!   Updated atomically without any lock — safe to call from inside `GlobalAlloc::alloc`.

use crate::sync::Spinlock;
use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicUsize, Ordering};
use types::CellId;

/// Maximum CellId tracked (index into IN_USE array).
pub const MAX_CELLS: usize = 64;

/// Default heap quota per Cell: 4 MiB.
pub const DEFAULT_QUOTA_BYTES: usize = 4 * 1024 * 1024;

/// Limit store — BTreeMap keyed by CellId raw value, stores the byte limit.
/// Locked only in `register`/`deregister` — NOT inside the allocator hot path.
static QUOTA_LIMITS: Spinlock<BTreeMap<usize, usize>> = Spinlock::new(BTreeMap::new());

/// Live byte counters — one AtomicUsize per Cell slot, zero-initialized.
/// Updated lock-free inside `charge`/`refund` to avoid alloc-inside-alloc deadlock.
const ZERO: AtomicUsize = AtomicUsize::new(0);
static IN_USE: [AtomicUsize; MAX_CELLS] = [ZERO; MAX_CELLS];

/// Force-release this module's locks during fault teardown.
///
/// # Safety
/// Single-hart; called only from the fault/panic path with interrupts disabled.
pub unsafe fn force_unlock_locks() {
    QUOTA_LIMITS.force_unlock();
}

/// Register a new Cell with the given heap quota.
///
/// Call this at spawn, OUTSIDE the allocator.
///
/// Deadlock contract: `BTreeMap::insert` allocates a new tree node via the
/// global `QuotaAlloc`, whose `alloc` calls `charge(current_cell_id())`.  When
/// `register` runs inside a cell's syscall (e.g. init calling `SpawnFromPath`),
/// `current_cell_id()` is that cell (non-zero), so `charge` would try to RE-LOCK
/// the `QUOTA_LIMITS` Spinlock we already hold here → self-deadlock.  We pin
/// `CURRENT_CELL_ID` to 0 (kernel = unlimited, charge short-circuits without
/// locking) across the insert so the node allocation never re-enters `charge`'s
/// lock.  The node is kernel bookkeeping, so charging it to the kernel is also
/// semantically correct.
pub fn register(cell_id: CellId, limit: usize) {
    let id = cell_id.0 as usize;
    if id < MAX_CELLS {
        IN_USE[id].store(0, Ordering::Relaxed);
    }
    // Pin to kernel context (0) so the BTreeMap::insert's node allocation does
    // not re-enter `charge` (which locks QUOTA_LIMITS) while we hold that lock.
    // Deadlock contract: see module doc.
    let prev_cell = crate::task::hart_local::current_cell_id();
    crate::task::hart_local::set_current_cell_id(0);
    QUOTA_LIMITS.lock().insert(id, limit);
    crate::task::hart_local::set_current_cell_id(prev_cell);
}

/// Deregister a Cell on exit.
pub fn deregister(cell_id: CellId) {
    let id = cell_id.0 as usize;
    if id < MAX_CELLS {
        IN_USE[id].store(0, Ordering::Relaxed);
    }
    QUOTA_LIMITS.lock().remove(&id);
}

/// Charge `size` bytes to the Cell.
///
/// Returns `false` if the quota would be exceeded — the caller (`QuotaAlloc::alloc`)
/// must return `null_mut()` in that case.
///
/// Lock-ordering: acquires `QUOTA_LIMITS` briefly for a read (no allocation inside),
/// then updates `IN_USE` atomically without any lock.
pub fn charge(cell_id_raw: usize, size: usize) -> bool {
    if cell_id_raw == 0 {
        return true; // kernel itself: unlimited
    }
    // Read the limit — BTreeMap::get does NOT allocate.  Lock released immediately.
    let limit = QUOTA_LIMITS.lock().get(&cell_id_raw).copied().unwrap_or(usize::MAX);
    if cell_id_raw >= MAX_CELLS {
        return true; // no slot in IN_USE — uncapped
    }
    // Optimistic add; roll back on breach.
    // Use saturating_add to prevent wrapping past usize::MAX — a hostile layout.size()
    // near MAX would otherwise make the comparison read false and bypass the limit.
    let prev = IN_USE[cell_id_raw].fetch_add(size, Ordering::Relaxed);
    if prev.saturating_add(size) > limit {
        IN_USE[cell_id_raw].fetch_sub(size, Ordering::Relaxed);
        false
    } else {
        true
    }
}

/// Refund `size` bytes when the Cell frees memory.  Lock-free.
///
/// Uses saturating subtraction to prevent underflow if a deallocation is
/// attributed to a cell that has already been deregistered (e.g., dealloc
/// of a Box shared with another cell arrives after the originating cell exited).
pub fn refund(cell_id_raw: usize, size: usize) {
    if cell_id_raw == 0 || cell_id_raw >= MAX_CELLS {
        return;
    }
    // fetch_update with saturating_sub prevents wrapping to usize::MAX on spurious frees.
    let _ = IN_USE[cell_id_raw].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |cur| {
        Some(cur.saturating_sub(size))
    });
}

/// Current byte usage for a Cell (for diagnostics).
pub fn in_use(cell_id: CellId) -> usize {
    let id = cell_id.0 as usize;
    if id < MAX_CELLS { IN_USE[id].load(Ordering::Relaxed) } else { 0 }
}
