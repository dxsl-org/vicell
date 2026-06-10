//! FAT backend over the VirtIO block device (fatfs crate + CachedBlockStream).
//!
//! Path contract: receives absolute VFS paths and strips `self.prefix` itself.
//! Two strip modes mirror the pre-MountTable dispatch exactly:
//! - ops (`write`/`unlink`/…) require a NON-EMPTY relative path — operating on
//!   the mount root is rejected;
//! - `list`/`stat` accept the mount root (`/data` → volume root).
//!
//! fatfs uses interior mutability, so all operations go through `&self`
//! internally; `&mut self` on the trait is only for RamFS symmetry.

use alloc::string::String;
use alloc::vec::Vec;

use crate::backend::FsBackend;
use crate::block_stream::{BlockStream, CachedBlockStream};
use ostd::io::println;

/// Concrete `fatfs::FileSystem` type for the VirtIO FAT volume.
///
/// `NullTimeProvider` and `LossyOemCpConverter` are the fatfs defaults;
/// using them avoids needing a RTC or a UTF-8↔OEM code-page converter.
type DataFs  = fatfs::FileSystem<CachedBlockStream, fatfs::NullTimeProvider, fatfs::LossyOemCpConverter>;
/// Convenience alias for a FAT directory handle — avoids repeating the full generic.
type DataDir<'a> = fatfs::Dir<'a, CachedBlockStream, fatfs::NullTimeProvider, fatfs::LossyOemCpConverter>;

pub struct FatBackend {
    fs: Option<DataFs>,
    prefix: &'static str,
}

/// Reads sector 0 of the FAT partition and returns `true` when the volume is
/// exFAT — identified by OEM-Name `"EXFAT   "` at BPB offset 3 (exFAT spec
/// §3.1). FAT12/16/32 never use this string.
///
/// A temporary `BlockStream` is used so the probe does not interfere with the
/// `CachedBlockStream` fatfs will create afterwards (positions are per-instance;
/// both start at offset 0).
fn probe_exfat() -> bool {
    use fatfs::Read;
    let mut bs = BlockStream::new();
    let mut sec = [0u8; 512];
    // Guard against short reads — need at least 11 bytes for the OEM-Name field.
    match bs.read(&mut sec) {
        Ok(n) if n >= 11 => &sec[3..11] == b"EXFAT   ",
        _ => false,
    }
}

impl FatBackend {
    /// Mount the persistent FAT volume on the VirtIO disk. On failure (no disk
    /// attached, bad BPB, or exFAT) the backend stays in fallback mode — every
    /// operation fails cleanly while other mounts keep working.
    pub fn mount(prefix: &'static str) -> Self {
        if probe_exfat() {
            println("[vfs] exFAT volume detected — not supported; reformat to FAT32 or use /data (littlefs)");
            return Self { fs: None, prefix };
        }
        let opts = fatfs::FsOptions::new().update_accessed_date(false);
        let fs = match fatfs::FileSystem::new(CachedBlockStream::new(), opts) {
            Ok(fs) => {
                println("[vfs] FAT32 /mnt/sd volume mounted");
                Some(fs)
            }
            Err(_) => {
                println("[vfs] WARNING: FAT32 mount failed — /mnt/sd writes will fail");
                None
            }
        };
        Self { fs, prefix }
    }

    /// Strip the mount prefix; reject the mount root itself.
    /// `/data/x` → `x`; `/data` and `/data/` → None.
    fn rel_nonempty<'a>(&self, path: &'a str) -> Option<&'a str> {
        let r = path.strip_prefix(self.prefix)?.strip_prefix('/')?;
        if r.is_empty() { None } else { Some(r) }
    }

    /// Strip the mount prefix; the mount root maps to the volume root (`""`).
    fn rel_allow_root<'a>(&self, path: &'a str) -> &'a str {
        let r = path.strip_prefix(self.prefix).unwrap_or(path);
        r.strip_prefix('/').unwrap_or(r)
    }
}

/// Split `"sub/dir/file"` into `("sub/dir", "file")`. `"file"` → `("", "file")`.
fn split_last(rel: &str) -> (&str, &str) {
    match rel.rfind('/') {
        Some(i) => (&rel[..i], &rel[i + 1..]),
        None    => ("", rel),
    }
}

/// Walk `parts` (a '/'-separated relative dir path) from `root`, creating any
/// missing component. Returns the leaf `Dir`, or `Err(())` on fatfs failure.
/// Empty `parts` returns `root` unchanged.
fn ensure_dir_chain<'a>(root: DataDir<'a>, parts: &str) -> Result<DataDir<'a>, ()> {
    let mut cur = root;
    for part in parts.split('/').filter(|p| !p.is_empty()) {
        // create_dir is create-or-open → idempotent on existing dirs.
        cur = cur.create_dir(part).map_err(|_| ())?;
    }
    Ok(cur)
}

/// Depth-first delete of `rel` (path relative to the FAT root).
///
/// Rebuilds `root_dir()` per level and addresses children by full relative path
/// so no `Dir` handle is held across a recursive call (borrow-checker safe).
/// Collects `iter()` entries before mutating — avoids iterator-vs-mutation aliasing.
fn remove_tree(fs: &DataFs, rel: &str) -> bool {
    let dir = match fs.root_dir().open_dir(rel) {
        Ok(d)  => d,
        // `rel` is a file (or already gone) — remove it directly.
        Err(_) => return fs.root_dir().remove(rel).is_ok(),
    };
    // Collect (name, is_dir) so the iterator borrow is released before we mutate.
    let entries: Vec<(String, bool)> = dir
        .iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            if name == "." || name == ".." { None } else { Some((name, e.is_dir())) }
        })
        .collect();
    drop(dir);

    for (name, is_dir) in &entries {
        let child = alloc::format!("{}/{}", rel, name);
        let ok = if *is_dir {
            remove_tree(fs, &child)
        } else {
            fs.root_dir().remove(&child).is_ok()
        };
        if !ok { return false; }
    }
    fs.root_dir().remove(rel).is_ok()
}

impl FsBackend for FatBackend {
    fn get_file_ptr(&self, _path: &str) -> Option<(usize, usize)> {
        None // disk-backed: no stable in-memory pointer — callers use the copy path
    }

    fn list(&self, path: &str, out: &mut [u8]) -> usize {
        let fs = match &self.fs { Some(f) => f, None => return 0 };
        let rel = self.rel_allow_root(path);
        let dir = if rel.is_empty() {
            fs.root_dir()
        } else {
            match fs.root_dir().open_dir(rel) {
                Ok(d) => d,
                Err(_) => return 0,
            }
        };

        let mut pos = 0;
        for entry in dir.iter() {
            let e = match entry { Ok(e) => e, Err(_) => break };
            let name = e.file_name();
            if name == "." || name == ".." { continue; }
            let prefix: &[u8] = if e.is_dir() { b"d:" } else { b"f:" };
            let name_b = name.as_bytes();
            let entry_len = 2 + name_b.len() + 1;
            if pos + entry_len > out.len() { break; }
            out[pos..pos + 2].copy_from_slice(prefix);
            out[pos + 2..pos + 2 + name_b.len()].copy_from_slice(name_b);
            out[pos + 2 + name_b.len()] = b'\n';
            pos += entry_len;
        }
        pos
    }

    fn stat(&self, path: &str) -> Option<(u64, bool)> {
        use fatfs::Seek as _;
        let fs = self.fs.as_ref()?;
        let rel = self.rel_allow_root(path);
        if rel.is_empty() { return Some((0, true)); } // mount root is a directory
        if rel.split('/').any(|c| c == "..") { return None; }
        if let Ok(mut file) = fs.root_dir().open_file(rel) {
            let size = file.seek(fatfs::SeekFrom::End(0)).unwrap_or(0);
            return Some((size, false));
        }
        if fs.root_dir().open_dir(rel).is_ok() {
            return Some((0, true));
        }
        None
    }

    fn file_size(&self, path: &str) -> u64 {
        use fatfs::Seek as _;
        let fs = match &self.fs { Some(f) => f, None => return 0 };
        let rel = match self.rel_nonempty(path) { Some(r) => r, None => return 0 };
        let mut file = match fs.root_dir().open_file(rel) {
            Ok(f) => f,
            Err(_) => return 0,
        };
        file.seek(fatfs::SeekFrom::End(0)).unwrap_or(0)
    }

    fn read_to_vec(&self, path: &str) -> Vec<u8> {
        use fatfs::Read as _;
        let fs = match &self.fs { Some(f) => f, None => return Vec::new() };
        let rel = match self.rel_nonempty(path) { Some(r) => r, None => return Vec::new() };
        let mut file = match fs.root_dir().open_file(rel) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };
        let mut buf = alloc::vec![0u8; 4096];
        let mut result = Vec::new();
        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => result.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        result
    }

    /// Create-or-overwrite with mkdir -p on intermediate directories. Uses
    /// remove-then-create for truncate semantics without `seek(End)`.
    fn write(&mut self, path: &str, content: &[u8]) -> bool {
        use fatfs::Write as _;
        let fs = match &self.fs { Some(f) => f, None => return false };
        let rel = match self.rel_nonempty(path) { Some(r) => r, None => return false };
        let (parent, name) = split_last(rel);
        if name.is_empty() { return false; }
        let dir = match ensure_dir_chain(fs.root_dir(), parent) {
            Ok(d)   => d,
            Err(()) => return false,
        };
        let _ = dir.remove(name);
        let mut file = match dir.create_file(name) { Ok(f) => f, Err(_) => return false };
        file.write_all(content).is_ok()
    }

    /// Append; creates the file (and parents) if absent — first append behaves
    /// like a write. `fatfs::File::seek(End(0))` translates to
    /// `disk.seek(Start(abs_end))` internally, so the `End` arm of
    /// `BlockStream::seek` (which errors) is never reached.
    fn append(&mut self, path: &str, content: &[u8]) -> bool {
        use fatfs::{Write as _, Seek as _};
        let fs = match &self.fs { Some(f) => f, None => return false };
        let rel = match self.rel_nonempty(path) { Some(r) => r, None => return false };
        if rel.split('/').any(|c| c == "..") { return false; }
        let (parent, name) = split_last(rel);
        if name.is_empty() { return false; }
        let dir = match ensure_dir_chain(fs.root_dir(), parent) {
            Ok(d)   => d,
            Err(()) => return false,
        };
        let mut file = match dir.open_file(name) {
            Ok(f)  => f,
            Err(_) => match dir.create_file(name) { Ok(f) => f, Err(_) => return false },
        };
        if file.seek(fatfs::SeekFrom::End(0)).is_err() { return false; }
        file.write_all(content).is_ok()
    }

    fn mkdir(&mut self, path: &str) -> bool {
        let fs = match &self.fs { Some(f) => f, None => return false };
        let rel = match self.rel_nonempty(path) { Some(r) => r, None => return false };
        ensure_dir_chain(fs.root_dir(), rel).is_ok()
    }

    /// Remove an EMPTY directory. `open_dir` succeeds only for directories;
    /// `remove` errors on a non-empty dir — strict POSIX type checking.
    fn rmdir(&mut self, path: &str) -> bool {
        let fs = match &self.fs { Some(f) => f, None => return false };
        let rel = match self.rel_nonempty(path) { Some(r) => r, None => return false };
        // Reject any path component that is ".." — defense-in-depth even though
        // fatfs confines resolution to the volume root.
        if rel.split('/').any(|c| c == "..") { return false; }
        if fs.root_dir().open_dir(rel).is_err() { return false; }
        fs.root_dir().remove(rel).is_ok()
    }

    /// Remove a regular FILE. `open_file` succeeds only for files in fatfs —
    /// acts as the type guard (use `rmdir` for directories).
    fn unlink(&mut self, path: &str) -> bool {
        let fs = match &self.fs { Some(f) => f, None => return false };
        let rel = match self.rel_nonempty(path) { Some(r) => r, None => return false };
        if fs.root_dir().open_file(rel).is_err() { return false; }
        fs.root_dir().remove(rel).is_ok()
    }

    /// Recursive delete (`rm -r`). A path resolving to a regular file is
    /// removed directly.
    fn rmdir_recursive(&mut self, path: &str) -> bool {
        let fs = match &self.fs { Some(f) => f, None => return false };
        let rel = match self.rel_nonempty(path) { Some(r) => r, None => return false };
        // Defense-in-depth: reject ".." before the recursive delete amplifies any
        // path-confusion. fatfs also confines to the volume root, but explicit is safer.
        if rel.split('/').any(|c| c == "..") { return false; }
        remove_tree(fs, rel)
    }
}
