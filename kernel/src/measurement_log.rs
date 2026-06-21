//! Per-Cell integrity measurement log (Linux IMA-style, TPM-free).
//!
//! On every `spawn_from_path`, the loader records `SHA-256(elf_bytes)` here
//! BEFORE the cell is scheduled to run. Entries form an append-only log plus a
//! rolling aggregate (`agg = SHA256(agg || entry_hash)`) — the single value a
//! future DICE/EAT remote-attestation token signs to prove the exact software
//! that ran on this device. See
//! `docs/research/research-cell-security-permissions.md` §3.6.
//!
//! This is *measurement* (evidence), not *enforcement*: it does not block a
//! cell. Signature-based rejection (Cell binary signing) is a separate,
//! orthogonal gate. Pairing the two gives "measured + verified launch".

use crate::sync::Spinlock;
use alloc::string::String;
use alloc::vec::Vec;

/// Soft cap on retained entries — bounds kernel memory. Beyond this individual
/// entries stop being appended (the aggregate still advances, so attestation
/// integrity is preserved) and a warning is logged. Cell spawns are bounded in
/// practice on G1.
const MAX_ENTRIES: usize = 256;

/// One measured cell: its TID, the SHA-256 of its ELF image, and its path.
pub struct MeasureEntry {
    pub tid:  u32,
    pub hash: [u8; 32],
    pub path: String,
}

struct Log {
    entries:   Vec<MeasureEntry>,
    aggregate: [u8; 32],
}

static LOG: Spinlock<Log> = Spinlock::new(Log {
    entries:   Vec::new(),
    aggregate: [0u8; 32],
});

/// Force-release this module's lock during fault teardown.
///
/// # Safety
/// Single-hart kernel; called only from the fault/panic path with interrupts
/// disabled. Force-unlocking an already-free Spinlock is a no-op.
pub unsafe fn force_unlock_locks() {
    LOG.force_unlock();
}

/// Measure a cell's ELF image: hash it, append to the log, extend the aggregate,
/// and emit a `CellMeasure` audit event. Returns the digest.
///
/// Call BEFORE the cell is scheduled (the loader does this right after spawn,
/// while still single-threaded in kernel context — the cell cannot have run yet).
pub fn measure(tid: usize, path: &str, elf: &[u8]) -> [u8; 32] {
    let hash = crate::sha256::sha256(elf);

    {
        let mut log = LOG.lock();
        // Extend the aggregate: agg = SHA256(agg || hash).
        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(&log.aggregate);
        buf[32..].copy_from_slice(&hash);
        log.aggregate = crate::sha256::sha256(&buf);

        if log.entries.len() < MAX_ENTRIES {
            log.entries.push(MeasureEntry { tid: tid as u32, hash, path: String::from(path) });
        } else {
            log::warn!("[measure] log full ({} entries) — aggregate still advancing", MAX_ENTRIES);
        }
    }

    // Audit: tid + first 4 hash bytes for quick correlation (full hash lives here).
    let hp = u32::from_le_bytes([hash[0], hash[1], hash[2], hash[3]]);
    crate::audit::log_event(
        crate::audit::AuditEvent::CellMeasure,
        &crate::audit::encode_u32x2(tid as u32, hp),
    );
    hash
}

/// Rolling aggregate over every measured cell (for future remote attestation).
pub fn aggregate() -> [u8; 32] {
    LOG.lock().aggregate
}

/// Number of entries currently retained (diagnostics).
pub fn entry_count() -> usize {
    LOG.lock().entries.len()
}
