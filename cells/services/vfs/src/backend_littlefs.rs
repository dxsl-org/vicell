//! littlefs backend for `/data` — the power-loss-resilient persistent store
//! (MBR partition P4; Milestone 2.5 Phase 04).
//!
//! Every operation mounts via `Filesystem::mount_and_then`: littlefs mounts
//! are cheap (superblock scan), the borrow gymnastics of holding a mounted
//! `Filesystem<'a>` across IPC turns disappear, and each request ends with
//! the volume in a consistent on-disk state — which is the whole point of
//! using littlefs (a power cut between requests can never tear the volume).
//!
//! First mount auto-formats: a blank P4 fails `mount`, gets one `format`,
//! and is retried (scoped to P4 — FAT and the cell table are untouched).

use alloc::string::String;
use alloc::vec::Vec;

use littlefs2::fs::{Filesystem, OpenOptions};
use littlefs2::io::Read as _;
use littlefs2::path::PathBuf;

use crate::backend::FsBackend;
use crate::lfs_disk::LfsDisk;

pub struct LittlefsBackend {
    disk: LfsDisk,
    prefix: &'static str,
}

impl LittlefsBackend {
    /// Create the backend and ensure P4 holds a mountable littlefs volume.
    pub fn mount(prefix: &'static str) -> Self {
        let mut disk = LfsDisk;
        let ok = Filesystem::mount_and_then(&mut disk, |_| Ok(())).is_ok();
        if !ok {
            match Filesystem::format(&mut disk) {
                Ok(()) => ostd::io::println("[vfs] littlefs: P4 blank — formatted"),
                Err(_) => ostd::io::println("[vfs] WARNING: littlefs format failed — /data unavailable"),
            }
        } else {
            ostd::io::println("[vfs] littlefs /data volume mounted");
        }
        Self { disk, prefix }
    }

    /// `/data/x/y` → littlefs absolute path `/x/y`; the mount root maps to `/`.
    fn rel_path(&self, path: &str) -> Option<PathBuf> {
        let r = path.strip_prefix(self.prefix).unwrap_or(path);
        let lfs = if r.is_empty() { "/" } else { r };
        if lfs.split('/').any(|c| c == "..") { return None; }
        PathBuf::try_from(lfs).ok()
    }

    fn with_fs<R>(
        &mut self,
        f: impl FnOnce(&Filesystem<'_, LfsDisk>) -> littlefs2::io::Result<R>,
    ) -> Option<R> {
        Filesystem::mount_and_then(&mut self.disk, f).ok()
    }
}

impl FsBackend for LittlefsBackend {
    fn get_file_ptr(&self, _path: &str) -> Option<(usize, usize)> {
        None // disk-backed: callers use the ReadAsync copy path
    }

    fn list(&self, path: &str, out: &mut [u8]) -> usize {
        // `list` takes &self but mounting needs &mut storage — LfsDisk is a ZST
        // with no state, so a scratch instance is equivalent.
        let mut disk = LfsDisk;
        let rel = match self.rel_path(path) { Some(p) => p, None => return 0 };
        let mut pos = 0;
        let _ = Filesystem::mount_and_then(&mut disk, |fs| {
            fs.read_dir_and_then(&rel, |iter| {
                for entry in iter.flatten() {
                    let name = entry.file_name().as_str_ref_with_trailing_nul();
                    let name = name.trim_end_matches('\0');
                    if name.is_empty() || name == "." || name == ".." { continue; }
                    let prefix: &[u8] = if entry.metadata().is_dir() { b"d:" } else { b"f:" };
                    let nb = name.as_bytes();
                    let entry_len = 2 + nb.len() + 1;
                    if pos + entry_len > out.len() { break; }
                    out[pos..pos + 2].copy_from_slice(prefix);
                    out[pos + 2..pos + 2 + nb.len()].copy_from_slice(nb);
                    out[pos + 2 + nb.len()] = b'\n';
                    pos += entry_len;
                }
                Ok(())
            })
        });
        pos
    }

    fn stat(&self, path: &str) -> Option<(u64, bool)> {
        let mut disk = LfsDisk;
        let rel = self.rel_path(path)?;
        Filesystem::mount_and_then(&mut disk, |fs| {
            let md = fs.metadata(&rel)?;
            Ok((md.len() as u64, md.is_dir()))
        })
        .ok()
    }

    fn file_size(&self, path: &str) -> u64 {
        self.stat(path).map(|(s, _)| s).unwrap_or(0)
    }

    fn read_to_vec(&self, path: &str) -> Vec<u8> {
        let mut disk = LfsDisk;
        let rel = match self.rel_path(path) { Some(p) => p, None => return Vec::new() };
        Filesystem::mount_and_then(&mut disk, |fs| {
            fs.open_file_and_then(&rel, |file| {
                let mut buf = [0u8; 512];
                let mut result = Vec::new();
                loop {
                    match file.read(&mut buf)? {
                        0 => break,
                        n => result.extend_from_slice(&buf[..n]),
                    }
                }
                Ok(result)
            })
        })
        .unwrap_or_default()
    }

    fn write(&mut self, path: &str, content: &[u8]) -> bool {
        let rel = match self.rel_path(path) { Some(p) => p, None => return false };
        self.with_fs(|fs| {
            OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open_and_then(fs, &rel, |file| {
                    use littlefs2::io::Write as _;
                    file.write_all(content)
                })
        })
        .is_some()
    }

    fn append(&mut self, path: &str, content: &[u8]) -> bool {
        let rel = match self.rel_path(path) { Some(p) => p, None => return false };
        self.with_fs(|fs| {
            OpenOptions::new()
                .write(true)
                .create(true)
                .append(true)
                .open_and_then(fs, &rel, |file| {
                    use littlefs2::io::Write as _;
                    file.write_all(content)
                })
        })
        .is_some()
    }

    /// Single-level create (mkdir -p callers walk components themselves; the
    /// pre-existing FAT backend did mkdir -p, so mirror that for /data users).
    fn mkdir(&mut self, path: &str) -> bool {
        let rel = match self.rel_path(path) { Some(p) => p, None => return false };
        self.with_fs(|fs| {
            // create_dir_all = mkdir -p semantics, matching the FAT backend.
            fs.create_dir_all(&rel)
        })
        .is_some()
    }

    fn rmdir(&mut self, path: &str) -> bool {
        let rel = match self.rel_path(path) { Some(p) => p, None => return false };
        self.with_fs(|fs| {
            // Type guard: only directories; littlefs remove_dir errors on files.
            let md = fs.metadata(&rel)?;
            if !md.is_dir() { return Err(littlefs2::io::Error::INVALID); }
            fs.remove_dir(&rel)
        })
        .is_some()
    }

    fn unlink(&mut self, path: &str) -> bool {
        let rel = match self.rel_path(path) { Some(p) => p, None => return false };
        self.with_fs(|fs| {
            // Type guard: only regular files (POSIX unlink semantics).
            let md = fs.metadata(&rel)?;
            if md.is_dir() { return Err(littlefs2::io::Error::INVALID); }
            fs.remove(&rel)
        })
        .is_some()
    }

    fn rmdir_recursive(&mut self, path: &str) -> bool {
        let rel = match self.rel_path(path) { Some(p) => p, None => return false };
        self.with_fs(|fs| fs.remove_dir_all(&rel)).is_some()
    }
}
