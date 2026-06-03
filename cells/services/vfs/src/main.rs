#![no_std]
#![no_main]

extern crate alloc;
extern crate driver_disk;

use api::hotswap::ViStateTransfer;

mod block_stream;
mod handle_table;
mod mount;
mod quota;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use handle_table::HandleTable;
use mount::MountTable;
use quota::QuotaTracker;
use ostd::io::println;
use ostd::prelude::*;

// Embedded binaries served from /bin/ until VirtIO-FAT integration lands.
static SHELL_ELF: &[u8] = include_bytes!("../../../../kernel/src/embedded/shell");
static HELLO_ELF: &[u8] = include_bytes!("../../../../kernel/src/embedded/hello");
static ECHO_ELF:  &[u8] = include_bytes!("../../../../kernel/src/embedded/echo");
static CAT_ELF:   &[u8] = include_bytes!("../../../../kernel/src/embedded/cat");
static LS_ELF:    &[u8] = include_bytes!("../../../../kernel/src/embedded/ls");
static LUA_ELF:   &[u8] = include_bytes!("../../../../kernel/src/embedded/lua");

// IPC opcodes.
const OP_GET_FILE: u8 = 1; // path -> (ptr:u64, len:u64)
const OP_LIST_DIR: u8 = 2; // path -> newline-separated names
const OP_STAT:     u8 = 3; // path -> (size:u64, is_dir:u8, pad:[u8;7])
const OP_WRITE:    u8 = 4; // [path_len:u8][content_len:u16 LE][path][content] → /data FAT16 or /tmp RamFS
const OP_READ:     u8 = 8; // path -> file bytes (up to 480), empty = not found
const OP_MKDIR:    u8 = 5; // path -> 0=ok, 1=err
const OP_RMDIR:            u8 = 6;  // path -> 0=ok, 1=err (only empty dirs)
const OP_UNLINK:           u8 = 7;  // path -> 0=ok, 1=err (only files)
const OP_RMDIR_RECURSIVE:  u8 = 9;  // path -> 0=ok, 1=err — recursive tree delete (/data only)
const OP_APPEND:           u8 = 10; // [path_len:u8][content_len:u16 LE][path][content] → seek-to-end append

#[derive(Clone)]
struct RamFile {
    #[allow(dead_code)]
    name: String,
    data: Vec<u8>,
    is_dir: bool,
    children: BTreeMap<String, Box<RamFile>>,
}

impl RamFile {
    fn new_file(name: &str, data: &[u8]) -> Self {
        Self { name: String::from(name), data: Vec::from(data), is_dir: false, children: BTreeMap::new() }
    }
    fn new_dir(name: &str) -> Self {
        Self { name: String::from(name), data: Vec::new(), is_dir: true, children: BTreeMap::new() }
    }
}

#[allow(dead_code)] // reason: handle/mount/quota fields used when write path is wired
pub struct VfsManager {
    root:    Box<RamFile>,
    handles: HandleTable,
    mounts:  MountTable,
    quota:   QuotaTracker,
}

impl VfsManager {
    pub fn new() -> Self {
        let mut root = Box::new(RamFile::new_dir("/"));
        root.children.insert(String::from("readme.txt"),
            Box::new(RamFile::new_file("readme.txt", b"Welcome to ViOS!\n")));

        let mut bin = Box::new(RamFile::new_dir("bin"));
        for (name, data) in [
            ("shell", SHELL_ELF), ("hello", HELLO_ELF), ("echo", ECHO_ELF),
            ("cat",   CAT_ELF),   ("ls",    LS_ELF),    ("lua",  LUA_ELF),
        ] {
            bin.children.insert(String::from(name), Box::new(RamFile::new_file(name, data)));
        }
        root.children.insert(String::from("bin"), bin);
        root.children.insert(String::from("tmp"), Box::new(RamFile::new_dir("tmp")));

        Self {
            root,
            handles: HandleTable::new(),
            mounts:  MountTable::new(),
            quota:   QuotaTracker::new(),
        }
    }

    fn find_node(&self, path: &str) -> Option<&RamFile> {
        if path == "/" { return Some(&self.root); }
        let mut cur = &self.root;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            cur = cur.children.get(part)?;
        }
        Some(cur)
    }

    fn get_file_content(&self, path: &str) -> Option<(usize, usize)> {
        let n = self.find_node(path)?;
        if n.is_dir { return None; }
        Some((n.data.as_ptr() as usize, n.data.len()))
    }

    fn list_dir(&self, path: &str, out: &mut [u8]) -> usize {
        let node = match self.find_node(path) {
            Some(n) if n.is_dir => n,
            _ => return 0,
        };
        let mut pos = 0;
        for name in node.children.keys() {
            let b = name.as_bytes();
            if pos + b.len() + 1 > out.len() { break; }
            out[pos..pos + b.len()].copy_from_slice(b);
            out[pos + b.len()] = b'\n';
            pos += b.len() + 1;
        }
        pos
    }

    fn stat(&self, path: &str, out: &mut [u8; 16]) -> bool {
        match self.find_node(path) {
            Some(n) => {
                out[0..8].copy_from_slice(&(n.data.len() as u64).to_le_bytes());
                out[8] = if n.is_dir { 1 } else { 0 };
                true
            }
            None => false,
        }
    }

    /// Mutable tree traversal — returns `None` if `path` does not exist.
    fn find_node_mut(&mut self, path: &str) -> Option<&mut RamFile> {
        if path == "/" { return Some(&mut self.root); }
        let mut cur: &mut RamFile = &mut self.root;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            cur = cur.children.get_mut(part)?.as_mut();
        }
        Some(cur)
    }

    /// Split `path` into (parent_path, child_name). Returns `None` for root "/".
    fn split_parent_name(path: &str) -> Option<(String, String)> {
        let path = path.trim_end_matches('/');
        if path.is_empty() { return None; }
        let slash = path.rfind('/')?;
        let parent = if slash == 0 { String::from("/") } else { String::from(&path[..slash]) };
        let name   = String::from(&path[slash + 1..]);
        if name.is_empty() { return None; }
        Some((parent, name))
    }

    /// Create a new empty directory at `path`. Returns false if it already exists or
    /// if the parent is not a directory.
    fn mkdir(&mut self, path: &str) -> bool {
        if let Some((parent_path, name)) = Self::split_parent_name(path) {
            if let Some(parent) = self.find_node_mut(&parent_path) {
                if parent.is_dir && !parent.children.contains_key(&name) {
                    parent.children.insert(name.clone(), Box::new(RamFile::new_dir(&name)));
                    return true;
                }
            }
        }
        false
    }

    /// Remove an empty directory at `path`. Returns false if it does not exist,
    /// is not a directory, or is non-empty.
    fn rmdir(&mut self, path: &str) -> bool {
        if let Some((parent_path, name)) = Self::split_parent_name(path) {
            if let Some(parent) = self.find_node_mut(&parent_path) {
                let removable = parent.children.get(&name)
                    .map(|c| c.is_dir && c.children.is_empty())
                    .unwrap_or(false);
                if removable {
                    parent.children.remove(&name);
                    return true;
                }
            }
        }
        false
    }

    /// Remove a regular file at `path`. Returns false if it does not exist or is
    /// a directory.
    fn unlink(&mut self, path: &str) -> bool {
        if let Some((parent_path, name)) = Self::split_parent_name(path) {
            if let Some(parent) = self.find_node_mut(&parent_path) {
                let removable = parent.children.get(&name)
                    .map(|c| !c.is_dir)
                    .unwrap_or(false);
                if removable {
                    parent.children.remove(&name);
                    return true;
                }
            }
        }
        false
    }

    /// Create or overwrite a regular file at `path` with `content`.
    ///
    /// Returns false if the parent directory does not exist or if `path` names
    /// an existing directory. Authorization (e.g. `/tmp/` prefix check) is the
    /// caller's responsibility.
    fn write_file(&mut self, path: &str, content: &[u8]) -> bool {
        let (parent_path, name) = match Self::split_parent_name(path) {
            Some(pn) => pn,
            None => return false,
        };
        let parent = match self.find_node_mut(&parent_path) {
            Some(p) if p.is_dir => p,
            _ => return false,
        };
        match parent.children.get_mut(&name) {
            Some(existing) if existing.is_dir => false,
            Some(existing) => { existing.data = Vec::from(content); true }
            None => {
                parent.children.insert(name.clone(),
                    Box::new(RamFile::new_file(&name, content)));
                true
            }
        }
    }

    /// Return a reference to the file's raw bytes, or `None` if not found or is a directory.
    fn get_file_data(&self, path: &str) -> Option<&[u8]> {
        let n = self.find_node(path)?;
        if n.is_dir { return None; }
        Some(&n.data)
    }
}

use block_stream::BlockStream;

/// Concrete `fatfs::FileSystem` type for the VirtIO FAT16 volume.
///
/// `NullTimeProvider` and `LossyOemCpConverter` are the fatfs defaults;
/// using them avoids needing a RTC or a UTF-8↔OEM code-page converter.
type DataFs  = fatfs::FileSystem<BlockStream, fatfs::NullTimeProvider, fatfs::LossyOemCpConverter>;
/// Convenience alias for a FAT16 directory handle — avoids repeating the full generic.
type DataDir<'a> = fatfs::Dir<'a, BlockStream, fatfs::NullTimeProvider, fatfs::LossyOemCpConverter>;

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

/// Create-or-overwrite `/data/[sub/]NAME` with `content` in the FAT16 volume.
///
/// Creates intermediate directories automatically (mkdir -p semantics). Uses
/// remove-then-create for truncate semantics without `seek(End)`.
fn write_fat16(fs: Option<&DataFs>, path: &str, content: &[u8]) -> bool {
    use fatfs::Write as _;
    let fs  = match fs { Some(f) => f, None => return false };
    let rel = match path.strip_prefix("/data/") {
        Some(n) if !n.is_empty() => n,
        _ => return false,
    };
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

/// Append `content` to `/data/[sub/]NAME`. Creates the file (and any parent dirs)
/// if absent — first append behaves like a write. Reuses `ensure_dir_chain` (mkdir -p).
///
/// `fatfs::File::seek(End(0))` translates to `disk.seek(Start(abs_end))` internally,
/// so the `End` arm of `BlockStream::seek` (which errors) is never reached.
fn append_fat16(fs: Option<&DataFs>, path: &str, content: &[u8]) -> bool {
    use fatfs::{Write as _, Seek as _};
    let fs  = match fs { Some(f) => f, None => return false };
    let rel = match path.strip_prefix("/data/") {
        Some(n) if !n.is_empty() => n,
        _ => return false,
    };
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

/// Read up to 480 bytes of `/data/[sub/]NAME` from the FAT16 volume.
///
/// fatfs `open_file` traverses '/'-separated paths natively, so no manual
/// traversal is needed for reads. Sends an empty reply on any failure.
fn read_fat16(fs: Option<&DataFs>, path: &str, sender: usize) {
    use fatfs::Read as _;
    let send_empty = || { ostd::syscall::sys_send(sender, b""); };
    let fs  = match fs { Some(f) => f, None => return send_empty() };
    let rel = match path.strip_prefix("/data/") {
        Some(n) if !n.is_empty() => n,
        _ => return send_empty(),
    };
    let mut file = match fs.root_dir().open_file(rel) { Ok(f) => f, Err(_) => return send_empty() };
    let mut resp  = [0u8; 480];
    let mut total = 0usize;
    while total < resp.len() {
        match file.read(&mut resp[total..]) {
            Ok(0)  => break,
            Ok(n)  => total += n,
            Err(_) => break,
        }
    }
    ostd::syscall::sys_send(sender, &resp[..total]);
}

/// Remove `/data/[sub/]NAME` where NAME is a regular FILE. Returns false if the
/// entry is a directory or does not exist (use `rmdir_fat16` for directories).
/// Phase H: `open_file` succeeds only for files in fatfs — acts as the type guard.
fn unlink_fat16(fs: Option<&DataFs>, path: &str) -> bool {
    let fs  = match fs { Some(f) => f, None => return false };
    let rel = match path.strip_prefix("/data/") {
        Some(n) if !n.is_empty() => n,
        _ => return false,
    };
    if fs.root_dir().open_file(rel).is_err() { return false; }
    fs.root_dir().remove(rel).is_ok()
}

/// Remove an EMPTY `/data/[sub/]DIR`. Returns false if the entry is a regular
/// file, is non-empty, or does not exist. Phase H: strict POSIX type checking.
/// `open_dir` succeeds only for directories; `remove` errors on a non-empty dir.
fn rmdir_fat16(fs: Option<&DataFs>, path: &str) -> bool {
    let fs  = match fs { Some(f) => f, None => return false };
    let rel = match path.strip_prefix("/data/") {
        Some(n) if !n.is_empty() => n,
        _ => return false,
    };
    // Reject any path component that is ".." — defense-in-depth even though
    // fatfs confines resolution to the volume root.
    if rel.split('/').any(|c| c == "..") { return false; }
    if fs.root_dir().open_dir(rel).is_err() { return false; }
    fs.root_dir().remove(rel).is_ok()
}

/// Recursively remove `/data/[sub/]DIR` and all its contents (POSIX `rm -r`).
/// A path resolving to a regular file is removed directly. Returns false on any
/// fatfs error or missing target. Only `/data/` is supported.
fn rmdir_recursive_fat16(fs: Option<&DataFs>, path: &str) -> bool {
    let fs  = match fs { Some(f) => f, None => return false };
    let rel = match path.strip_prefix("/data/") {
        Some(n) if !n.is_empty() => n,
        _ => return false,
    };
    // Defense-in-depth: reject ".." before the recursive delete amplifies any
    // path-confusion. fatfs also confines to the volume root, but explicit is safer.
    if rel.split('/').any(|c| c == "..") { return false; }
    remove_tree(fs, rel)
}

/// Depth-first delete of `rel` (path relative to the FAT16 root).
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
    let entries: alloc::vec::Vec<(alloc::string::String, bool)> = dir
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

/// Create `/data/[sub/]...` directory chain in the FAT16 volume (mkdir -p).
fn fat16_mkdir(fs: Option<&DataFs>, path: &str) -> bool {
    let fs  = match fs { Some(f) => f, None => return false };
    let rel = match path.strip_prefix("/data/") {
        Some(n) if !n.is_empty() => n,
        _ => return false,
    };
    ensure_dir_chain(fs.root_dir(), rel).is_ok()
}

#[no_mangle]
pub fn main() {
    println("VFS Service v0.2: RamFS + mkdir/rmdir/unlink IPC");
    let mut vfs = VfsManager::new();
    let mut buf = [0u8; 512];

    // Mount the persistent FAT16 volume on the VirtIO disk. On failure (no disk
    // attached, bad BPB) fall back to RamFS-only — /data writes will fail with
    // 0x01 but /tmp and /bin still work.
    let opts = fatfs::FsOptions::new().update_accessed_date(false);
    let fat_fs: Option<DataFs> = match fatfs::FileSystem::new(BlockStream::new(), opts) {
        Ok(fs) => {
            println("[vfs] FAT16 /data volume mounted");
            Some(fs)
        }
        Err(_) => {
            println("[vfs] WARNING: FAT16 mount failed — /data writes will fail");
            None
        }
    };

    loop {
        match ostd::syscall::sys_recv(0, &mut buf) {
            ostd::syscall::SyscallResult::Ok(sender) if sender > 0 => {
                let path_len = buf[1] as usize;
                let path = core::str::from_utf8(&buf[2..2usize.saturating_add(path_len)]).ok();

                // NOTE: `path` (from buf[2..]) is the 2-byte-header parse used by
                // mkdir/rmdir/unlink. OP_WRITE/OP_APPEND re-parse from buf[4..] (4-byte
                // header). Future opcodes must pick ONE scheme and NOT use the loop-level
                // `path` if they use the 4-byte header.
                match buf[0] {
                    OP_GET_FILE => {
                        if let Some(p) = path {
                            let mut resp = [0u8; 16];
                            if let Some((ptr, len)) = vfs.get_file_content(p) {
                                resp[0..8].copy_from_slice(&(ptr as u64).to_le_bytes());
                                resp[8..16].copy_from_slice(&(len as u64).to_le_bytes());
                            }
                            ostd::syscall::sys_send(sender, &resp);
                        }
                    }
                    OP_LIST_DIR => {
                        if let Some(p) = path {
                            let mut resp = [0u8; 480];
                            let n = vfs.list_dir(p, &mut resp);
                            ostd::syscall::sys_send(sender, &resp[..n]);
                        }
                    }
                    OP_STAT => {
                        if let Some(p) = path {
                            let mut resp = [0u8; 16];
                            vfs.stat(p, &mut resp);
                            ostd::syscall::sys_send(sender, &resp);
                        }
                    }
                    OP_WRITE => {
                        // Header: [4][path_len:u8][content_len:u16 LE][path][content]
                        let pl = buf[1] as usize;
                        let cl = u16::from_le_bytes([buf[2], buf[3]]) as usize;
                        let ok = if 4 + pl + cl <= buf.len() {
                            match core::str::from_utf8(&buf[4..4 + pl]) {
                                Ok(p) if p.starts_with("/data/") => {
                                    write_fat16(fat_fs.as_ref(), p, &buf[4 + pl..4 + pl + cl])
                                }
                                Ok(p) if p.starts_with("/tmp/") => {
                                    vfs.write_file(p, &buf[4 + pl..4 + pl + cl])
                                }
                                _ => false,
                            }
                        } else { false };
                        ostd::syscall::sys_send(sender, if ok { b"\x00" } else { b"\x01" });
                    }
                    OP_READ => {
                        if let Some(p) = path {
                            if p.starts_with("/data/") {
                                read_fat16(fat_fs.as_ref(), p, sender);
                            } else if let Some(data) = vfs.get_file_data(p) {
                                let n = data.len().min(480);
                                ostd::syscall::sys_send(sender, &data[..n]);
                            } else {
                                ostd::syscall::sys_send(sender, b"");
                            }
                        }
                    }
                    OP_MKDIR => {
                        if let Some(p) = path {
                            let ok = if p.starts_with("/data/") {
                                fat16_mkdir(fat_fs.as_ref(), p)
                            } else {
                                vfs.mkdir(p)
                            };
                            ostd::syscall::sys_send(sender, if ok { b"\x00" } else { b"\x01" });
                        }
                    }
                    OP_RMDIR => {
                        if let Some(p) = path {
                            // Phase H: rmdir_fat16 verifies the target IS a directory
                            // before calling remove() — POSIX type semantics (ENOTDIR).
                            let ok = if p.starts_with("/data/") {
                                rmdir_fat16(fat_fs.as_ref(), p)
                            } else {
                                vfs.rmdir(p)
                            };
                            ostd::syscall::sys_send(sender, if ok { b"\x00" } else { b"\x01" });
                        }
                    }
                    OP_UNLINK => {
                        if let Some(p) = path {
                            let ok = if p.starts_with("/data/") {
                                unlink_fat16(fat_fs.as_ref(), p)
                            } else {
                                vfs.unlink(p)
                            };
                            ostd::syscall::sys_send(sender, if ok { b"\x00" } else { b"\x01" });
                        }
                    }
                    OP_RMDIR_RECURSIVE => {
                        if let Some(p) = path {
                            // Recursive delete only for the persistent FAT16 volume.
                            // /tmp RamFS is volatile and out of scope for recursive delete.
                            let ok = if p.starts_with("/data/") {
                                rmdir_recursive_fat16(fat_fs.as_ref(), p)
                            } else {
                                false
                            };
                            ostd::syscall::sys_send(sender, if ok { b"\x00" } else { b"\x01" });
                        }
                    }
                    OP_APPEND => {
                        // Same header as OP_WRITE: [op][path_len:u8][content_len:u16 LE][path][content]
                        let pl = buf[1] as usize;
                        let cl = u16::from_le_bytes([buf[2], buf[3]]) as usize;
                        let ok = if 4 + pl + cl <= buf.len() {
                            match core::str::from_utf8(&buf[4..4 + pl]) {
                                Ok(p) if p.starts_with("/data/") =>
                                    append_fat16(fat_fs.as_ref(), p, &buf[4 + pl..4 + pl + cl]),
                                Ok(p) if p.starts_with("/tmp/") => {
                                    let content = &buf[4 + pl..4 + pl + cl];
                                    let mut data = vfs.get_file_data(p)
                                        .map(|d| d.to_vec())
                                        .unwrap_or_default();
                                    data.extend_from_slice(content);
                                    vfs.write_file(p, &data)
                                }
                                _ => false,
                            }
                        } else { false };
                        ostd::syscall::sys_send(sender, if ok { b"\x00" } else { b"\x01" });
                    }
                    _ => {
                        ostd::syscall::sys_send(sender, b"");
                    }
                }
            }
            _ => {
                ostd::task::yield_now();
            }
        }
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
