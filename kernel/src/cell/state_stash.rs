//! Kernel state stash for hot migration.
//!
//! A Cell about to be replaced serialises its state and `stash`es it under a
//! well-known key; the replacement instance `restore`s it on startup. Keeping
//! the bytes in the kernel (not a file) means the transfer works before the
//! VFS is reachable and outlives the old cell's address space. This is the
//! standalone state-transfer primitive; it is independent of the IPC-based
//! orchestrator in `hotswap.rs`.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use crate::sync::Spinlock;

/// Upper bound on a single stashed blob (64 KB) — bounds kernel memory held on
/// a cell's behalf for one slot.
pub const MAX_STASH_LEN: usize = 64 * 1024;

/// Upper bound on the number of distinct stash slots. Caps total kernel memory
/// the stash can hold (≤ MAX_ENTRIES × MAX_STASH_LEN) so a misbehaving cell
/// cannot exhaust the heap by stashing under unboundedly many keys.
pub const MAX_ENTRIES: usize = 64;

/// key → serialized state bytes. Keys are cell-chosen (typically a stable
/// FNV hash of the cell name), so a replacement instance reads the same slot.
static STASH: Spinlock<BTreeMap<u64, Vec<u8>>> = Spinlock::new(BTreeMap::new());

/// Store `bytes` under `key`, replacing any previous value. Returns the number
/// of bytes stored (clamped to [`MAX_STASH_LEN`]), or 0 if the stash is full
/// (at [`MAX_ENTRIES`] distinct keys) and `key` is not already present.
pub fn stash(key: u64, bytes: &[u8]) -> usize {
    let mut map = STASH.lock();
    if map.len() >= MAX_ENTRIES && !map.contains_key(&key) {
        log::warn!("[state-stash] full ({} entries); rejecting new key", MAX_ENTRIES);
        return 0;
    }
    let n = bytes.len().min(MAX_STASH_LEN);
    map.insert(key, bytes[..n].to_vec());
    n
}

/// Copy stashed bytes for `key` into `buf`. Returns the number of bytes written
/// (0 if no state is stashed). The stash entry is left in place so multiple
/// readers (or a retry) can recover it.
pub fn restore(key: u64, buf: &mut [u8]) -> usize {
    let guard = STASH.lock();
    let Some(bytes) = guard.get(&key) else { return 0 };
    let n = bytes.len().min(buf.len());
    buf[..n].copy_from_slice(&bytes[..n]);
    n
}

/// Remove and discard the entry for `key`, freeing its slot.
///
/// Used for one-shot keys (per-spawn argv personal slots) so they do not
/// accumulate toward [`MAX_ENTRIES`]. No-op when `key` is absent.
pub fn remove(key: u64) {
    STASH.lock().remove(&key);
}

/// Boot-time self-test of the state-transfer primitive: stash a sentinel under
/// a scratch key, restore it, and confirm the bytes round-trip. Logs the
/// outcome so an integration test can assert it. The scratch entry is removed
/// afterwards so it never collides with a real cell's state.
pub fn self_test() {
    const SCRATCH_KEY: u64 = 0xFFFF_FFFF_FFFF_FFFEu64;
    let sentinel: [u8; 8] = [0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0x12, 0x34];
    stash(SCRATCH_KEY, &sentinel);
    let mut buf = [0u8; 8];
    let n = restore(SCRATCH_KEY, &mut buf);
    STASH.lock().remove(&SCRATCH_KEY);
    if n == 8 && buf == sentinel {
        log::info!("state-stash: round-trip OK");
    } else {
        log::error!("state-stash: round-trip FAILED (n={}, buf={:?})", n, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stash_restore_round_trip() {
        let key = 42u64;
        let data = [1u8, 2, 3, 4, 5];
        assert_eq!(stash(key, &data), 5);
        let mut out = [0u8; 5];
        assert_eq!(restore(key, &mut out), 5);
        assert_eq!(out, data);
    }

    #[test]
    fn restore_missing_key_returns_zero() {
        let mut out = [0u8; 8];
        assert_eq!(restore(0xDEAD_0000_0000_0001, &mut out), 0);
    }

    #[test]
    fn stash_overwrites_previous() {
        let key = 7u64;
        stash(key, &[9u8; 4]);
        stash(key, &[1u8, 2]);
        let mut out = [0u8; 4];
        assert_eq!(restore(key, &mut out), 2);
        assert_eq!(&out[..2], &[1u8, 2]);
    }
}
