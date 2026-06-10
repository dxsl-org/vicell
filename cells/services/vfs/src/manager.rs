//! `VfsManager` — mount table plus the cross-cutting service state (quota,
//! access control, async-read handles, grant handle table).
//!
//! Backend routing is fully encapsulated here: dispatch code calls these
//! delegates and never inspects path prefixes itself.

use alloc::boxed::Box;
use alloc::vec::Vec;

use api::hotswap::ViStateTransfer;
use ostd::prelude::*;

use crate::access::AccessTable;
use crate::backend_bootfs::BootFsProxy;
use crate::backend_fat::FatBackend;
use crate::backend_ramfs::RamFsBackend;
use crate::handle_table::HandleTable;
use crate::mount::MountTable;
use crate::pending::PendingTable;
use crate::quota::QuotaTracker;

pub struct VfsManager {
    mounts: MountTable,
    pub handles: HandleTable,
    pub quota:   QuotaTracker,
    pub access:  AccessTable,
    pub pending: PendingTable,
}

impl VfsManager {
    pub fn new() -> Self {
        let mut mounts = MountTable::new();
        let ram  = mounts.add_backend(Box::new(RamFsBackend::new()));
        let fat  = mounts.add_backend(Box::new(FatBackend::mount("/data")));
        let boot = mounts.add_backend(Box::new(BootFsProxy));
        // Longest prefix wins: /tmp, /data and /bin shadow the read-only root.
        mounts.mount("/",     ram,  false);
        mounts.mount("/tmp",  ram,  true);
        mounts.mount("/data", fat,  true);
        // /bin proxies the kernel initramfs (VIFS1) — no more double-embedded ELFs.
        mounts.mount("/bin",  boot, false);

        Self {
            mounts,
            handles: HandleTable::new(),
            // test-hooks: 2 KiB quota so vfs-test can hit the limit in a few
            // small writes; production keeps the full 32 MB default.
            #[cfg(feature = "test-hooks")]
            quota:   QuotaTracker::with_limit(2048),
            #[cfg(not(feature = "test-hooks"))]
            quota:   QuotaTracker::new(),
            access:  AccessTable::new(),
            pending: PendingTable::new(),
        }
    }

    pub fn get_file_ptr(&self, path: &str) -> Option<(usize, usize)> {
        self.mounts.backend(path)?.get_file_ptr(path)
    }

    pub fn list_dir(&self, path: &str, out: &mut [u8]) -> usize {
        self.mounts.backend(path).map(|b| b.list(path, out)).unwrap_or(0)
    }

    pub fn stat(&self, path: &str) -> Option<(u64, bool)> {
        self.mounts.backend(path)?.stat(path)
    }

    pub fn file_size(&self, path: &str) -> u64 {
        self.mounts.backend(path).map(|b| b.file_size(path)).unwrap_or(0)
    }

    pub fn read_to_vec(&self, path: &str) -> Vec<u8> {
        self.mounts.backend(path).map(|b| b.read_to_vec(path)).unwrap_or_default()
    }

    pub fn write(&mut self, path: &str, content: &[u8]) -> bool {
        self.mounts.backend_mut(path).map(|b| b.write(path, content)).unwrap_or(false)
    }

    pub fn append(&mut self, path: &str, content: &[u8]) -> bool {
        self.mounts.backend_mut(path).map(|b| b.append(path, content)).unwrap_or(false)
    }

    pub fn mkdir(&mut self, path: &str) -> bool {
        self.mounts.backend_mut(path).map(|b| b.mkdir(path)).unwrap_or(false)
    }

    pub fn rmdir(&mut self, path: &str) -> bool {
        self.mounts.backend_mut(path).map(|b| b.rmdir(path)).unwrap_or(false)
    }

    pub fn unlink(&mut self, path: &str) -> bool {
        self.mounts.backend_mut(path).map(|b| b.unlink(path)).unwrap_or(false)
    }

    pub fn rmdir_recursive(&mut self, path: &str) -> bool {
        self.mounts.backend_mut(path).map(|b| b.rmdir_recursive(path)).unwrap_or(false)
    }
}

// ─── Hot-swap state transfer ───────────────────────────────────────────────────
//
// VFS serialises its quota table so per-cell byte-usage accounting survives a
// live upgrade.  The handle table is NOT serialised — open handles are inherently
// session-scoped and client cells reopen files after the swap completes.
//
// Wire format (little-endian, schema v1):
//   [version: u32][cell_count: u32]
//     [cell_id: u64][bytes_used: u64]...

const VFS_SCHEMA_VERSION: u32 = 1;

impl ViStateTransfer for VfsManager {
    fn state_size(&self) -> usize {
        4 + 4 + self.quota.entry_count() * 16 // version + count + (id,used) pairs
    }

    fn serialize_state(&self, buf: &mut [u8]) -> ViResult<usize> {
        let needed = self.state_size();
        if buf.len() < needed { return Err(ViError::InvalidArgument); }
        let mut pos = 0;
        buf[pos..pos+4].copy_from_slice(&VFS_SCHEMA_VERSION.to_le_bytes()); pos += 4;
        let entries = self.quota.all_entries();
        buf[pos..pos+4].copy_from_slice(&(entries.len() as u32).to_le_bytes()); pos += 4;
        for (id, used) in &entries {
            buf[pos..pos+8].copy_from_slice(&id.to_le_bytes());   pos += 8;
            buf[pos..pos+8].copy_from_slice(&used.to_le_bytes()); pos += 8;
        }
        Ok(pos)
    }

    fn deserialize_state(&mut self, buf: &[u8]) -> ViResult<()> {
        if buf.len() < 8 { return Err(ViError::InvalidInput); }
        let _version   = u32::from_le_bytes([buf[0],buf[1],buf[2],buf[3]]);
        let count      = u32::from_le_bytes([buf[4],buf[5],buf[6],buf[7]]) as usize;
        let mut pos    = 8;
        for _ in 0..count {
            if pos + 16 > buf.len() { return Err(ViError::InvalidInput); }
            let id   = u64::from_le_bytes(buf[pos..pos+8].try_into().map_err(|_| ViError::InvalidInput)?);
            let used = u64::from_le_bytes(buf[pos+8..pos+16].try_into().map_err(|_| ViError::InvalidInput)?);
            self.quota.restore(types::CellId(id), used);
            pos += 16;
        }
        Ok(())
    }
}
