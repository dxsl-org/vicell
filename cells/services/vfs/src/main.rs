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

// IPC now uses typed api::ipc::VfsRequest / VfsResponse via postcard encoding.
// Raw byte opcode constants removed — see libs/api/src/ipc.rs.

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
            Box::new(RamFile::new_file("readme.txt", b"Welcome to ViCell!\n")));

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

// Global VFS manager for the fast-IPC handler (which runs outside the main recv loop).
// Protected by a spinlock; on single-hart there is no actual contention.
static GLOBAL_VFS: ostd::prelude::Mutex<Option<VfsManager>> = ostd::prelude::Mutex::new(None);

/// Fast-IPC handler: serves VfsRequest::GetFile without ecall overhead.
///
/// # Safety
/// Called with S-mode interrupts disabled (guaranteed by `ostd::fast_ipc::call_vfs`).
unsafe fn vfs_fast_handler(
    req: &api::ipc::VfsRequest<'_>,
    out: &mut [u8; api::ipc::IPC_BUF_SIZE],
) -> usize {
    let resp = match req {
        api::ipc::VfsRequest::GetFile(path) => {
            if let Some(vfs) = GLOBAL_VFS.lock().as_ref() {
                if let Some((ptr, len)) = vfs.get_file_content(path) {
                    api::ipc::VfsResponse::DataPtr { ptr: ptr as u64, len: len as u64 }
                } else {
                    api::ipc::VfsResponse::Err(1)
                }
            } else {
                api::ipc::VfsResponse::Err(0xFF)
            }
        }
        _ => api::ipc::VfsResponse::Err(0xFE), // other ops must use ecall path
    };
    api::ipc::encode(&resp, out).map(|s| s.len()).unwrap_or(0)
}

#[no_mangle]
pub fn main() {
    println("VFS Service v0.2: RamFS + mkdir/rmdir/unlink IPC (typed postcard)");
    let vfs = VfsManager::new();
    *GLOBAL_VFS.lock() = Some(vfs);

    // Register the fast-IPC handler so trusted Cells can bypass ecall for VFS reads.
    // The kernel records the VFS cell's ID at spawn time so it can clear this
    // pointer if VFS crashes — see loader.rs fast_ipc::set_vfs_handler_cell call.
    ostd::fast_ipc::register_vfs(vfs_fast_handler);
    let mut buf = [0u8; 512];

    // Mount the persistent FAT16 volume on the VirtIO disk. On failure (no disk
    // attached, bad BPB) fall back to RamFS-only — /data writes will fail with
    // 0x01 but /tmp and /bin still work.
    let opts = fatfs::FsOptions::new().update_accessed_date(false);
    let mut fat_fs: Option<DataFs> = match fatfs::FileSystem::new(BlockStream::new(), opts) {
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
                // Encode the response into a local buffer while holding the VFS lock,
                // then DROP the lock before sys_send.  If ipc_send blocks (client not
                // yet in Recv), yield_cpu switches to another cell.  That cell may call
                // call_vfs which also acquires GLOBAL_VFS — a deadlock if we still hold
                // the lock during the send.
                let mut encoded = [0u8; 512];
                let encoded_len: usize;
                {
                    let mut resp_buf = [0u8; 512];
                    // Acquire VFS state; released at end of this block, before sys_send.
                    let mut gvfs = GLOBAL_VFS.lock();
                    let vfs = gvfs.as_mut().expect("VFS initialized before serving requests");
                    // Decode typed request; `take_from_bytes` tolerates trailing zeros
                    // in the 512-byte receive buffer left over from previous messages.
                    let resp: api::ipc::VfsResponse = match api::ipc::decode::<api::ipc::VfsRequest>(&buf) {
                    Ok(req) => match req {
                        api::ipc::VfsRequest::GetFile(p) => {
                            if let Some((ptr, len)) = vfs.get_file_content(p) {
                                api::ipc::VfsResponse::DataPtr { ptr: ptr as u64, len: len as u64 }
                            } else {
                                api::ipc::VfsResponse::Err(1)
                            }
                        }
                        api::ipc::VfsRequest::ListDir(p) => {
                            let mut tmp = [0u8; 480];
                            let n = vfs.list_dir(p, &mut tmp);
                            resp_buf[..n].copy_from_slice(&tmp[..n]);
                            // Encode raw bytes as VfsResponse::Data — borrows resp_buf.
                            // The encode happens inside the lock; sys_send happens
                            // outside (lock released at end of the enclosing block).
                            api::ipc::VfsResponse::Data(&resp_buf[..n])
                        }
                        api::ipc::VfsRequest::Stat(p) => {
                            if let Some(node) = vfs.find_node(p) {
                                api::ipc::VfsResponse::Stat {
                                    size: node.data.len() as u64,
                                    is_dir: node.is_dir,
                                }
                            } else {
                                api::ipc::VfsResponse::Err(1)
                            }
                        }
                        api::ipc::VfsRequest::Write { path, content } => {
                            let ok = if path.starts_with("/data/") {
                                write_fat16(fat_fs.as_ref(), path, content)
                            } else if path.starts_with("/tmp/") {
                                vfs.write_file(path, content)
                            } else { false };
                            if ok { api::ipc::VfsResponse::Ok } else { api::ipc::VfsResponse::Err(1) }
                        }
                        api::ipc::VfsRequest::Append { path, content } => {
                            let ok = if path.starts_with("/data/") {
                                append_fat16(fat_fs.as_ref(), path, content)
                            } else if path.starts_with("/tmp/") {
                                let mut data = vfs.get_file_data(path)
                                    .map(|d| d.to_vec())
                                    .unwrap_or_default();
                                data.extend_from_slice(content);
                                vfs.write_file(path, &data)
                            } else { false };
                            if ok { api::ipc::VfsResponse::Ok } else { api::ipc::VfsResponse::Err(1) }
                        }
                        api::ipc::VfsRequest::Mkdir(p) => {
                            let ok = if p.starts_with("/data/") {
                                fat16_mkdir(fat_fs.as_ref(), p)
                            } else { vfs.mkdir(p) };
                            if ok { api::ipc::VfsResponse::Ok } else { api::ipc::VfsResponse::Err(1) }
                        }
                        api::ipc::VfsRequest::Rmdir(p) => {
                            // Verifies the target IS a directory — POSIX ENOTDIR semantics.
                            let ok = if p.starts_with("/data/") {
                                rmdir_fat16(fat_fs.as_ref(), p)
                            } else { vfs.rmdir(p) };
                            if ok { api::ipc::VfsResponse::Ok } else { api::ipc::VfsResponse::Err(1) }
                        }
                        api::ipc::VfsRequest::Unlink(p) => {
                            let ok = if p.starts_with("/data/") {
                                unlink_fat16(fat_fs.as_ref(), p)
                            } else { vfs.unlink(p) };
                            if ok { api::ipc::VfsResponse::Ok } else { api::ipc::VfsResponse::Err(1) }
                        }
                        api::ipc::VfsRequest::RmdirRecursive(p) => {
                            // Recursive delete only supported on the persistent FAT16 volume.
                            let ok = p.starts_with("/data/") && rmdir_recursive_fat16(fat_fs.as_ref(), p);
                            if ok { api::ipc::VfsResponse::Ok } else { api::ipc::VfsResponse::Err(1) }
                        }
                    },
                    Err(_) => api::ipc::VfsResponse::Err(0xFF), // malformed request
                    };
                    // Encode while holding the lock (safe: no sys_send yet).
                    encoded_len = api::ipc::encode(&resp, &mut encoded).map(|s| s.len()).unwrap_or(0);
                    let _ = resp_buf; // suppress unused warning
                } // GLOBAL_VFS lock released here — before sys_send

                // Send after releasing the lock so a blocked ipc_send + yield_cpu
                // cannot switch to a cell that deadlocks on GLOBAL_VFS.
                ostd::syscall::sys_send(sender, &encoded[..encoded_len]);
                buf = [0u8; 512];
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
