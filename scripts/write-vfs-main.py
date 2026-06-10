"""Write the updated VFS service main.rs with extended IPC protocol."""
# IPC opcode constants for documentation
# OP_GET_FILE = 1: path -> (ptr: u64, len: u64)  read-only zero-copy
# OP_LIST_DIR = 2: path -> newline-separated entry names
# OP_STAT     = 3: path -> (size: u64, is_dir: u8, pad: [u8;7])
# OP_WRITE    = 4: stub, returns 0xff (requires VirtIO-FAT backing)

content = r"""#![no_std]
#![no_main]

extern crate alloc;
extern crate driver_disk;

mod handle_table;
mod mount;
mod quota;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use handle_table::HandleTable;
use mount::MountTable;
use ostd::io::println;
use ostd::prelude::*;
use quota::QuotaTracker;

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
const OP_WRITE:    u8 = 4; // stub — requires VirtIO-FAT, returns 0xff

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
}

#[no_mangle]
pub fn main() {
    println("VFS Service v0.2: RamFS + extended IPC protocol");
    let mut vfs = VfsManager::new();
    let mut buf = [0u8; 512];

    loop {
        match ostd::syscall::sys_recv(0, &mut buf) {
            ostd::syscall::SyscallResult::Ok(sender) if sender > 0 => {
                let path_len = buf[1] as usize;
                let path = core::str::from_utf8(&buf[2..2usize.saturating_add(path_len)]).ok();

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
                        // Requires VirtIO-FAT backing; stub returns 0xff (error).
                        ostd::syscall::sys_send(sender, b"\xff");
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
"""

with open("d:/ViCell/cells/services/vfs/src/main.rs", "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("main.rs written")
