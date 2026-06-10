//! BootFS proxy backend — serves `/bin` from the kernel's embedded initramfs
//! (`VIFS1`, the FAT16 `kernel_fs.img` baked into the kernel binary).
//!
//! Replaces the VFS-side `include_bytes!` copies of the /bin ELFs, which
//! duplicated every binary already present in `kernel_fs.img` (two copies of
//! each ELF resident in RAM). The proxy reads through the kernel's FD
//! syscalls (Open/Read/ReadDir/Close), which resolve against VIFS1 —
//! see `kernel/src/task.rs::file_open` and `kernel/src/fs.rs`.
//!
//! Strictly read-only: every mutating op returns false before any syscall.
//! `get_file_ptr` returns `None` (no syscalls) so the fast-IPC handler —
//! which runs with interrupts disabled in the CALLER's context — never
//! blocks inside this backend; readers fall back to the ReadAsync copy path.

use alloc::vec::Vec;

use crate::backend::FsBackend;
use ostd::syscall::{sys_close, sys_close_cap, sys_open, sys_open_cap, sys_read_cap, sys_readdir};

pub struct BootFsProxy;

/// RAII guard: kernel FDs live in the VFS task's TCB; leaking them across
/// requests would exhaust the per-task handle map.
struct Fd(usize);
impl Drop for Fd {
    fn drop(&mut self) {
        sys_close(self.0);
    }
}

fn open(path: &str) -> Option<Fd> {
    sys_open(path).ok().map(Fd)
}

/// Look up one entry of `parent` by name via ReadDir. `(size, is_dir)`.
fn dir_lookup(parent: &str, name: &str) -> Option<(u64, bool)> {
    let fd = open(parent)?;
    while let Ok(Some(e)) = sys_readdir(fd.0) {
        let len = e.name.iter().position(|&b| b == 0).unwrap_or(e.name.len());
        if &e.name[..len] == name.as_bytes() {
            let is_dir = matches!(e.file_type, types::FileType::Directory);
            return Some((e.size, is_dir));
        }
    }
    None
}

impl FsBackend for BootFsProxy {
    fn get_file_ptr(&self, _path: &str) -> Option<(usize, usize)> {
        None // no stable user-visible pointer; callers use the ReadAsync copy path
    }

    fn list(&self, path: &str, out: &mut [u8]) -> usize {
        let fd = match open(path) { Some(f) => f, None => return 0 };
        let mut pos = 0;
        while let Ok(Some(e)) = sys_readdir(fd.0) {
            let len = e.name.iter().position(|&b| b == 0).unwrap_or(e.name.len());
            if len == 0 { continue; }
            let name = &e.name[..len];
            if name == b"." || name == b".." { continue; }
            let prefix: &[u8] = if matches!(e.file_type, types::FileType::Directory) { b"d:" } else { b"f:" };
            let entry_len = 2 + len + 1;
            if pos + entry_len > out.len() { break; }
            out[pos..pos + 2].copy_from_slice(prefix);
            out[pos + 2..pos + 2 + len].copy_from_slice(name);
            out[pos + 2 + len] = b'\n';
            pos += entry_len;
        }
        pos
    }

    fn stat(&self, path: &str) -> Option<(u64, bool)> {
        // Mount roots ("/bin") are directories by definition; children are
        // resolved through the parent's directory entries (file_fstat is a
        // kernel stub, and DirEntry already carries size + type).
        let trimmed = path.trim_end_matches('/');
        let slash = trimmed.rfind('/')?;
        let name = &trimmed[slash + 1..];
        if name.is_empty() { return Some((0, true)); }
        let parent = if slash == 0 { "/" } else { &trimmed[..slash] };
        match dir_lookup(parent, name) {
            Some(hit) => Some(hit),
            // The mount prefix itself may not appear in its parent's listing
            // (e.g. stat "/bin" when VIFS1 only lists from "/") — probe by open.
            None if open(trimmed).is_some() => Some((0, true)),
            None => None,
        }
    }

    fn file_size(&self, path: &str) -> u64 {
        self.stat(path).map(|(s, _)| s).unwrap_or(0)
    }

    /// File reads go through the SYNCHRONOUS cap path (OpenCap/ReadCap), NOT
    /// the FD `Read` syscall: `file_read(fd > 2)` is an async transformation —
    /// it parks the task as `Polling` and the scheduler sweep writes the result
    /// into the saved trap frame later. That contract requires the caller to
    /// stop running immediately after the syscall; a busy read loop here kept
    /// executing, the sweep clobbered a live trap frame, and the VFS cell died
    /// with a zeroed-frame fault (scause=0).
    fn read_to_vec(&self, path: &str) -> Vec<u8> {
        // FAT16 stores 8.3 names uppercase; OpenCap matches the raw path string
        // (unlike the loader's read_file_from_vifs1, which uppercases first).
        let upper: alloc::string::String =
            path.chars().map(|c| c.to_ascii_uppercase()).collect();
        let cap = match sys_open_cap(&upper) { Ok(c) => c, Err(_) => return Vec::new() };
        let mut buf = [0u8; 512];
        let mut result = Vec::new();
        loop {
            match sys_read_cap(cap, &mut buf) {
                Ok(0) => break,
                Ok(n) => result.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        sys_close_cap(cap);
        result
    }

    fn write(&mut self, _path: &str, _content: &[u8]) -> bool { false }
    fn append(&mut self, _path: &str, _content: &[u8]) -> bool { false }
    fn mkdir(&mut self, _path: &str) -> bool { false }
    fn rmdir(&mut self, _path: &str) -> bool { false }
    fn unlink(&mut self, _path: &str) -> bool { false }
    fn rmdir_recursive(&mut self, _path: &str) -> bool { false }
}
