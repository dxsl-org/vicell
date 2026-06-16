//! Resource Registry — exclusive MMIO region grants for Driver Cells.
//!
//! A Driver Cell calls `sys_request_mmio(base, len)` (added in Phase 03).
//! The kernel checks here before handing an `MmioRegion` to the Cell:
//!
//! 1. **Allowlist**: the requested range must fall within a known-safe
//!    device window for the current QEMU target.  Unknown ranges are rejected
//!    so a misbehaving Cell cannot map arbitrary kernel memory as MMIO.
//!
//! 2. **Exclusive ownership**: at most one Cell may hold a given MMIO range.
//!    A second `request_mmio` for an overlapping range returns `AlreadyExists`.
//!
//! 3. **Release-on-exit**: `release_for(cell_id)` frees all ranges owned by
//!    a Cell.  Call this from every Cell-exit path alongside
//!    `cell_quota::deregister`.
//!
//! # v1 scope
//! Allowlist is hardcoded per QEMU target (DTB discovery deferred to v2).
//!
//! | Target | Device | Base | Size |
//! |--------|--------|------|------|
//! | QEMU ARM virt (aarch64) | PL011 UART0 | 0x0900_0000 | 0x1000 |
//! | QEMU ARM virt (aarch64) | PL061 GPIO  | 0x0903_0000 | 0x1000 |
//! | QEMU RISC-V virt (riscv64) | (none yet — kernel serial owns UART) | — | — |

use crate::sync::Spinlock;
use alloc::collections::BTreeMap;
use types::{CellId, ViError, ViResult};

// ---------------------------------------------------------------------------
// Allowlist (per QEMU machine, v1 hardcoded)
// ---------------------------------------------------------------------------

/// `(base, len)` pairs that a Driver Cell may request.
#[cfg(target_arch = "aarch64")]
const ALLOWED: &[(usize, usize)] = &[
    (0x0900_0000, 0x1000), // PL011 UART0  — QEMU ARM virt
    (0x0903_0000, 0x1000), // PL061 GPIO   — QEMU ARM virt
];

/// SiFive GPIO0 for QEMU `sifive_u` machine (FU540/FU740).
/// The kernel serial driver owns NS16550 at 0x1000_0000 — excluded from allowlist.
#[cfg(target_arch = "riscv64")]
const ALLOWED: &[(usize, usize)] = &[
    (0x1001_2000, 0x1000), // SiFive GPIO0 — QEMU sifive_u machine
];

#[cfg(target_arch = "x86_64")]
const ALLOWED: &[(usize, usize)] = &[];

#[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64", target_arch = "x86_64")))]
const ALLOWED: &[(usize, usize)] = &[];

// ---------------------------------------------------------------------------
// Registry state
// ---------------------------------------------------------------------------

/// Maps MMIO base address → (len, owner CellId).
static REGISTRY: Spinlock<BTreeMap<usize, (usize, CellId)>> =
    Spinlock::new(BTreeMap::new());

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Force-release this module's lock during fault teardown.
///
/// # Safety
/// Single-hart kernel; called only from the fault/panic path with interrupts
/// disabled.  Force-unlocking an already-free Spinlock is a no-op.
pub unsafe fn force_unlock_locks() {
    REGISTRY.force_unlock();
}

/// Request exclusive ownership of `[base, base+len)` for `cell_id`.
///
/// Returns:
/// - `Ok(())` — range is now owned by the caller; construct `MmioRegion` and
///   hand it to the Cell.
/// - `Err(PermissionDenied)` — range not in allowlist.
/// - `Err(AlreadyExists)` — range overlaps an already-granted region.
/// - `Err(InvalidInput)` — arithmetic overflow in `base + len`.
pub fn request_mmio(cell_id: CellId, base: usize, len: usize) -> ViResult<()> {
    // 1. Allowlist check
    let end = base.checked_add(len).ok_or(ViError::InvalidInput)?;
    let in_allowlist = ALLOWED.iter().any(|&(ab, al)| {
        let ae = ab + al; // allowlist range end (no overflow — hardcoded values)
        base >= ab && end <= ae
    });
    if !in_allowlist {
        return Err(ViError::PermissionDenied);
    }

    // 2. Overlap check — no two cells may share a byte
    let mut reg = REGISTRY.lock();
    for (&eb, &(el, _owner)) in reg.iter() {
        let ee = eb + el;
        // Ranges overlap when NOT (end ≤ eb OR base ≥ ee)
        if !(end <= eb || base >= ee) {
            return Err(ViError::AlreadyExists);
        }
    }

    reg.insert(base, (len, cell_id));
    Ok(())
}

/// Release all MMIO regions owned by `cell_id`.
///
/// Call this from every Cell-exit path (Exit syscall, ForceExit, fault, watchdog).
pub fn release_for(cell_id: CellId) {
    REGISTRY.lock().retain(|_base, &mut (_len, owner)| owner != cell_id);
}

/// Return the task ID (TID) of the cell that currently owns the MMIO region
/// whose base address exactly matches `base`.
///
/// Returns `None` if no cell has requested that exact base address.
/// Used by the GPIO IRQ handler to route interrupts to the current MMIO owner.
pub fn lookup_mmio_owner(base: usize) -> Option<usize> {
    REGISTRY.lock().get(&base).map(|&(_len, cell_id)| cell_id.0 as usize)
}

/// Current number of registered regions (diagnostics).
pub fn region_count() -> usize {
    REGISTRY.lock().len()
}
